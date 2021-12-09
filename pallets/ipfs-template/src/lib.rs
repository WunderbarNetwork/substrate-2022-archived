#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>

use codec::{ Decode, Encode };
use frame_system::{
	offchain::{ AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer},
};
use sp_runtime::{
	offchain::{
		ipfs,
		storage::{MutateStorageError, StorageRetrievalError, StorageValueRef},
	},
	RuntimeDebug
};

use frame_support::{
	traits::Randomness,
	dispatch::DispatchResult,
};

#[cfg(feature = "std")]
use frame_support::serde::{ Deserialize, Serialize };

use sp_core::{
	offchain::{ Duration, IpfsRequest, IpfsResponse, OpaqueMultiaddr, Timestamp},
};
use log::{ error, info };
use sp_std::str;
use sp_std::vec::Vec;
use core::{convert::TryInto};

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod crypto {
	use sp_core::{
		sr25519::Signature as Sr25519Signature,
	};
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner
	};

	app_crypto!(sr25519, sp_core::crypto::key_types::IPFS_EXAMPLE);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericPublic = sp_core::sr25519::Public;
		type GenericSignature = sp_core::sr25519::Signature;
	}

	// Implemented for mock runtime in tests
	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericPublic = sp_core::sr25519::Public;
		type GenericSignature = sp_core::sr25519::Signature;
	}
}

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_core::crypto::KeyTypeId;
	use pallet_ipfs_core as IpfsCore;
	use pallet_ipfs_core::{
		generate_id, ipfs_request, ocw_process_command,
		ocw_parse_ipfs_response, addresses_to_utf8_safe_bytes,
		IpfsCommand, CommandRequest,
		Event as IpfsEvent,
		Error as IpfsError,
	};

	pub const KEY_TYPE: KeyTypeId = sp_core::crypto::key_types::IPFS_EXAMPLE;
	const PROCESSED_COMMANDS: &[u8; 24] = b"ipfs::ipfs-example-comds";

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_ipfs_core::Config + CreateSignedTransaction<Call<Self>> {
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
	// EXAMPLE: store the CommandRequests in a StorageValue for fast iteration.
	#[pallet::storage]
	#[pallet::getter(fn commands)]
	pub type Commands<T: Config> = StorageValue<_, Vec<CommandRequest<T>>>;

	/** Pallets use events to inform users when important changes are made.
	 */
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		OcwCallback(T::AccountId),
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
			if let Err(_err) = Self::ocw_process_command_requests(block_number) { error!("IPFS: command request"); }
		}
	}


	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// EXAMPLE: Add extrinsics here.

		/** Process a signed callback from the ocw */
		#[pallet::weight(0)]
		pub fn ocw_callback(origin: OriginFor<T>, identifier: [u8; 32], data: Vec<u8>) -> DispatchResult {
			let signer = ensure_signed(origin)?;

			// Side effect:, swap_remove will change the ordering of the Vec!
			// TODO: the lock still exists in the ocw storage
			let mut callback_command :Option<CommandRequest<T>> = None;
			Commands::<T>::mutate(|command_requests| {
				let mut commands = command_requests.clone().unwrap();

				if let Some(index) = commands.iter().position(|cmd| { cmd.identifier == identifier }) {
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
		/**
		Iterate over all of the Active CommandRequests calling them.

		Im sure some more logic will go in here.
		 */
		fn ocw_process_command_requests(block_number: T::BlockNumber) -> Result<(), Error<T>> {
			let commands: Vec<CommandRequest<T>> = Commands::<T>::get().unwrap_or(Vec::<CommandRequest<T>>::new());

			for command_request in commands {

				match ocw_process_command::<T>(block_number, command_request.clone(), PROCESSED_COMMANDS) {
					Ok(responses) => {
						let callback_response = ocw_parse_ipfs_response::<T>(responses);

						// EXAMPLE: Some more logic can be put here.

						Self::signed_callback(&command_request, callback_response);
					}
					Err(e) => {
						match e {
							IpfsError::<T>::RequestFailed => { error!("IPFS: failed to perform a request") },
							_ => {}
						}
					}
				}
			}

			Ok(())
		}

		/** callback to the on-chain validators to continue processing the CID  **/
		fn signed_callback(command_request: &CommandRequest<T>, data: Vec<u8>) -> Result<(), IpfsError<T>> {
			// TODO: Dynamic signers so that its not only the validator who can sign the request.
			let signer = Signer::<T, T::AuthorityId>::all_accounts();
			if !signer.can_sign() {
				error!("*** IPFS *** ---- No local accounts available. Consider adding one via `author_insertKey` RPC.");
				return Err(IpfsError::<T>::RequestFailed)?
			}

			let results  = signer.send_signed_transaction(|_account| {
				Call::ocw_callback{
					identifier: command_request.identifier,
					data: data.clone()
				}
			});
			for (account, result) in &results {
				match result {
					Ok(()) => { info!("callback sent") },
					Err(e) => { error!("Failed to submit transaction {:?}", e)}
				}
			}
			// TODO: return a better Result
			Ok(())
		}

		// - Ideally the callback function can be override by another pallet that is coupled to this one allowing for custom functionality.
		// - data can be used for callbacks IE add a cid to the signer / uploader.
		// - Validate a connected peer has the CID, and which peer has it etc.
		// TODO: Result
		fn command_callback(command_request: &CommandRequest<T>, data: Vec<u8>) -> Result<(), Error<T>>{
			let owner = &command_request.clone().requester;

			for command in command_request.clone().ipfs_commands {
				match command {
					// EXAMPLE: Callback logic goes in here. Matching pallet_ipfs_core::pallet::IpfsCommand's
					_ => {}
				}
			}

			// TODO: return a better Result
			Ok(())
		}
	}
}
