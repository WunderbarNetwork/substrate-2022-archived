#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>
use codec::{Decode, Encode};
use frame_system::offchain::{AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer};
use sp_runtime::{
	offchain::{
		ipfs,
		storage::{MutateStorageError, StorageRetrievalError, StorageValueRef},
	},
	RuntimeDebug,
};

use frame_support::{dispatch::DispatchResult, traits::Randomness};

#[cfg(feature = "std")]
use frame_support::serde::{Deserialize, Serialize};

use core::convert::TryInto;
use log::{error, info};
use sp_core::offchain::{Duration, IpfsRequest, IpfsResponse, OpaqueMultiaddr, Timestamp};
use sp_std::{str, vec::Vec};

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod crypto {
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner,
	};
	app_crypto!(sr25519, sp_core::crypto::key_types::POCKET_MINTS);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericPublic = sp_core::sr25519::Public;
		type GenericSignature = sp_core::sr25519::Signature;
	}

	// Implemented for mock runtime in tests
	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
		for TestAuthId
	{
		type RuntimeAppPublic = Public;
		type GenericPublic = sp_core::sr25519::Public;
		type GenericSignature = sp_core::sr25519::Signature;
	}
}

// ** TODO: move this public methods into another file

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use pallet_ipfs_core as IpfsCore;
	use pallet_ipfs_core::{
		addresses_to_utf8_safe_bytes, generate_id, ipfs_request, ocw_parse_ipfs_response,
		ocw_process_command, CommandRequest, Error as IpfsError, Event as IpfsEvent, IpfsCommand,
	};
	use sp_core::crypto::KeyTypeId;

	pub const KEY_TYPE: KeyTypeId = sp_core::crypto::key_types::POCKET_MINTS;
	const PROCESSED_COMMANDS: &[u8; 24] = b"ipfs::pocket-mint-assets";

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config:
		frame_system::Config + pallet_ipfs_core::Config + CreateSignedTransaction<Call<Self>>
	{
		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// overarching dispatch call type.
		type Call: From<Call<Self>>;

		type IpfsRandomness: Randomness<Self::Hash, Self::BlockNumber>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	// The pallet's runtime storage items.
	// https://docs.substrate.io/v3/runtime/storage

	/** Store a list of Commands for the ocw to process */
	#[pallet::storage]
	#[pallet::getter(fn commands)]
	pub type Commands<T: Config> = StorageValue<_, Vec<CommandRequest<T>>>;

	#[pallet::storage]
	#[pallet::getter(fn wallet_assets)]
	pub type WalletAssets<T: Config> =
		StorageMap<_, Twox64Concat, T::AccountId, Vec<Vec<u8>>, ValueQuery>;

	/** Pallets use events to inform users when important changes are made.
	 */
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		MintCid(T::AccountId),
		OcwCallback(T::AccountId),
		MintedCid(T::AccountId, Vec<u8>),
		FailedToFindCid(T::AccountId, Vec<u8>),
	}

	/** Errors inform users that something went wrong.
	- RequestFailed,
	 */
	#[pallet::error]
	pub enum Error<T> {
		StorageError,
		FailedToFindCid,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn offchain_worker(block_number: T::BlockNumber) {
			if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker started with ***") {
				error!("IPFS: Error occurred during `print_metadata`");
			}

			if let Err(_err) = Self::ocw_process_command_requests(block_number) {
				error!("IPFS: command request");
			}
		}
	}

	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::weight(300)]
		pub fn mint_cid(origin: OriginFor<T>, address: Vec<u8>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();

			// TODO : check that the cid doesn't already exist

			commands.push(IpfsCommand::ConnectTo(address));
			// commands.push(IpfsCommand::GetProviders(cid.clone()));
			commands.push(IpfsCommand::CatBytes(cid.clone()));

			let ipfs_command = CommandRequest::<T> {
				identifier: generate_id::<T>(),
				requester: requester.clone(),
				ipfs_commands: commands,
			};

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::MintCid(requester)))
		}

		#[pallet::weight(0)]
		pub fn ocw_callback(
			origin: OriginFor<T>,
			identifier: [u8; 32],
			data: Vec<u8>,
		) -> DispatchResult {
			let signer = ensure_signed(origin)?;

			// Side effect:, swap_remove will change the ordering of the Vec!
			// TODO: the lock still exists in the ocw storage
			let mut callback_command: Option<CommandRequest<T>> = None;
			Commands::<T>::mutate(|command_requests| {
				let mut commands = command_requests.clone().unwrap();

				if let Some(index) = commands.iter().position(|cmd| cmd.identifier == identifier) {
					info!("Removing at index {}", index.clone());
					callback_command = Some(commands.swap_remove(index).clone());
				};

				*command_requests = Some(commands);
			});

			Self::deposit_event(Event::OcwCallback(signer));

			Self::command_callback(&callback_command.unwrap(), data);

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		// helper functions

		/**
		Iterate over all of the Active CommandRequests calling them.

		Im sure some more logic will go in here.
		 */
		fn ocw_process_command_requests(block_number: T::BlockNumber) -> Result<(), Error<T>> {
			let commands: Vec<CommandRequest<T>> =
				Commands::<T>::get().unwrap_or(Vec::<CommandRequest<T>>::new());

			for command_request in commands {
				match ocw_process_command::<T>(
					block_number,
					command_request.clone(),
					PROCESSED_COMMANDS,
				) {
					Ok(responses) => {
						let callback_response = ocw_parse_ipfs_response::<T>(responses);

						Self::signed_callback(&command_request, callback_response);
					},
					Err(e) => match e {
						IpfsError::<T>::RequestFailed => {
							error!("IPFS: failed to perform a request")
						},
						_ => {},
					},
				}
			}

			Ok(())
		}

		/** Output the current state of IPFS worker */
		fn print_metadata(message: &str) -> Result<(), IpfsError<T>> {
			let peers = if let IpfsResponse::Peers(peers) = ipfs_request::<T>(IpfsRequest::Peers)? {
				peers
			} else {
				Vec::new()
			};

			info!("{}", message);
			info!("IPFS: Is currently connected to {} peers", peers.len());
			if !peers.is_empty() {
				info!("IPFS: Peer Ids: {:?}", str::from_utf8(&addresses_to_utf8_safe_bytes(peers)))
			}
			info!("IPFS: CommandRequest size: {}", Commands::<T>::decode_len().unwrap_or(0));

			Ok(())
		}

		/** callback to the on-chain validators to continue processing the CID  * */
		fn signed_callback(
			command_request: &CommandRequest<T>,
			data: Vec<u8>,
		) -> Result<(), IpfsError<T>> {
			// TODO: Dynamic signers so that its not only the validator who can sign the request.
			let signer = Signer::<T, T::AuthorityId>::all_accounts();
			if !signer.can_sign() {
				error!("*** IPFS *** ---- No local accounts available. Consider adding one via `author_insertKey` RPC.");

				return Err(IpfsError::<T>::RequestFailed)?
			}

			let results = signer.send_signed_transaction(|_account| Call::ocw_callback {
				identifier: command_request.identifier,
				data: data.clone(),
			});
			for (account, result) in &results {
				match result {
					Ok(()) => {
						info!("callback sent")
					},
					Err(e) => {
						error!("Failed to submit transaction {:?}", e)
					},
				}
			}
			// TODO: return a better Result
			Ok(())
		}

		// 
		// - Ideally the callback function can be override by another pallet that is coupled to this
		//   one allowing for custom functionality.
		// - data can be used for callbacks IE add a cid to the signer / uploader.
		// - Validate a connected peer has the CID, and which peer has it etc.
		// TODO: Result
		fn command_callback(
			command_request: &CommandRequest<T>,
			data: Vec<u8>,
		) -> Result<(), Error<T>> {
			if let Ok(utf8_str) = str::from_utf8(&*data) {
				info!("Received string: {:?}", utf8_str);
			} else {
				info!("Received data: {:?}", data);
			}

			let owner = &command_request.clone().requester;

			for command in command_request.clone().ipfs_commands {
				match command {
					IpfsCommand::GetProviders(cid) | IpfsCommand::CatBytes(cid) => {
						if !data.is_empty() {
							WalletAssets::<T>::mutate(&owner, |assets| {
								assets.push(cid.clone());
							});
							Self::deposit_event(Event::MintedCid(
								command_request.clone().requester,
								cid.clone(),
							));
						} else {
							Self::deposit_event(Event::FailedToFindCid(
								command_request.clone().requester,
								cid.clone(),
							));
							return Err(Error::<T>::FailedToFindCid)
						}
					},
					_ => {},
				}
			}

			// TODO: return a better Result
			Ok(())
		}
	}
}
