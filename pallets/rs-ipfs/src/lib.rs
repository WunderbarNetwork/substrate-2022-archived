// #![feature(option_result_contains)]
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
	app_crypto!(sr25519, sp_core::crypto::key_types::IPFS);

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

// ** TODO: move this public methods into another file

pub fn generate_id<T: Config>() -> [u8; 32] {
	let payload = (
		T::IpfsRandomness::random(&b"ipfs-request-id"[..]).0,
		<frame_system::Pallet<T>>::block_number(),
	);
	payload.using_encoded(sp_io::hashing::blake2_256)
}


/** Process each IPFS `command_request`
	1) lock the request for asynchronous processing
	2) Call each command in CommandRequest.ipfs_commands
		- Make sure each command is successfully before attempting the next
		-
 */
pub fn process_command<T: Config>(block_number: T::BlockNumber, command_request: CommandRequest<T>, persistence_key: &[u8; 24]) -> Result<Vec<IpfsResponse>, Error<T>>{

	let acquire_lock = acquire_command_request_lock::<T>(block_number, &command_request);

	match acquire_lock {
		Ok(_block) => {
			let mut result = Vec::<IpfsResponse>::new();

			for command in command_request.clone().ipfs_commands {
				match command {
					IpfsCommand::ConnectTo(ref address) => {
						match ipfs_request::<T>(IpfsRequest::Connect(OpaqueMultiaddr(address.clone()))) {
							Ok(IpfsResponse::Success) => { Ok(result.push(IpfsResponse::Success)) }
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::DisconnectFrom(ref address) => {
						match ipfs_request::<T>(IpfsRequest::Disconnect(OpaqueMultiaddr(address.clone()))) {
							Ok(IpfsResponse::Success) => { Ok(result.push(IpfsResponse::Success)) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::AddBytes(ref bytes_to_add) => {
						match ipfs_request::<T>(IpfsRequest::AddBytes(bytes_to_add.clone())) {
							Ok(IpfsResponse::AddBytes(cid)) => { Ok(result.push(IpfsResponse::AddBytes(cid))) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::CatBytes(ref cid) => {
						match ipfs_request::<T>(IpfsRequest::CatBytes(cid.clone())) {
							Ok(IpfsResponse::CatBytes(bytes_received)) => { Ok(result.push(IpfsResponse::CatBytes(bytes_received))) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::InsertPin(ref cid) => {
						match ipfs_request::<T>(IpfsRequest::InsertPin(cid.clone(), false)) {
							Ok(IpfsResponse::Success) => { Ok(result.push(IpfsResponse::Success)) },
							_ => {Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::RemovePin(ref cid) => {
						match ipfs_request::<T>(IpfsRequest::RemovePin(cid.clone(), false)) {
							Ok(IpfsResponse::Success) => { Ok(result.push(IpfsResponse::Success)) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},

					IpfsCommand::RemoveBlock(ref cid) 		=> {
						match ipfs_request::<T>(IpfsRequest::RemoveBlock(cid.clone())) {
							Ok(IpfsResponse::Success) => { Ok(result.push(IpfsResponse::Success)) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},
					IpfsCommand::FindPeer(ref peer_id) => {
						match ipfs_request::<T>(IpfsRequest::FindPeer(peer_id.clone())) {
							Ok(IpfsResponse::FindPeer(addresses)) => { Ok(result.push(IpfsResponse::FindPeer(addresses))) },
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},
					IpfsCommand::GetProviders(ref cid) => {
						match ipfs_request::<T>(IpfsRequest::GetProviders(cid.clone())) {
							Ok(IpfsResponse::GetProviders(peer_ids)) => { Ok(result.push(IpfsResponse::GetProviders(peer_ids))) }
							_ => { Err(Error::<T>::RequestFailed) }
						}
					},
				};
			}

			processed_commands::<T>(&command_request, persistence_key);

			Ok(result)
		},
		_ => { Err(Error::<T>::FailedToAcquireLock) },
	}
}
/** Send a request to the local IPFS node; Can only be called in an offchain worker. **/
pub fn ipfs_request<T: Config>(request: IpfsRequest) -> Result<IpfsResponse, Error<T>> {
	let ipfs_request = ipfs::PendingRequest::new(request).map_err(|_| Error::CannotCreateRequest)?;

	ipfs_request.try_wait(Some(sp_io::offchain::timestamp().add(Duration::from_millis(1_200))))
		.map_err(|_| Error::<T>::RequestTimeout)?
		.map(|req| req.response)
		.map_err(|_error| { Error::<T>::RequestFailed })
}

/** Using the CommandRequest<T>.identifier we can attempt to create a lock via StorageValueRef,
leaving behind a block number of when the lock was formed. */
fn acquire_command_request_lock<T: Config>(block_number: T::BlockNumber, command_request: &CommandRequest<T>) -> Result<T::BlockNumber, MutateStorageError<T::BlockNumber, Error<T>>> {
	let storage = StorageValueRef::persistent(&command_request.identifier);

	storage.mutate(|command_identifier: Result<Option<T::BlockNumber>, StorageRetrievalError>| {
		match command_identifier {
			Ok(Some(block))  => {
				if block_number != block {
					info!("Lock failed, lock was not in current block");
					Err(Error::<T>::FailedToAcquireLock)
				} else {
					Ok(block)
				}
			},
			_ => {
				info!("IPFS: Acquired lock!");
				Ok(block_number)
			},
		}
	})
}

/** Store a list of command identifiers to remove the lock in a following block */
fn processed_commands<T: Config>(command_request: &CommandRequest<T>, persistence_key: &[u8; 24]) -> Result<Vec<[u8; 32]>, MutateStorageError<Vec<[u8; 32]>, ()>>{
	let processed_commands = StorageValueRef::persistent(persistence_key);

	processed_commands.mutate(|processed_commands: Result<Option<Vec<[u8; 32]>>, StorageRetrievalError>| {
		match processed_commands {
			Ok(Some(mut commands)) => {
				commands.push(command_request.identifier);

				Ok(commands)
			}
			_ => {
				let mut res = Vec::<[u8; 32]>::new();
				res.push(command_request.identifier);

				Ok(res)
			}
		}
	})
}

// END of public methods

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_core::crypto::KeyTypeId;
	use {
		generate_id, ipfs_request, process_command
	};

	pub const KEY_TYPE: KeyTypeId = sp_core::crypto::key_types::IPFS;
	const PROCESSED_COMMANDS: &[u8; 24] = b"ipfs::processed_commands";

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config + CreateSignedTransaction<Call<Self>> {
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

	/** Commands for interacting with IPFS

		Connection Commands:
		- ConnectTo(OpaqueMultiaddr)
		- DisconnectFrom(OpaqueMultiaddr)

		Data Commands:
		- AddBytes(Vec<u8>)
		- CatBytes(Vec<u8>)
		- InsertPin(Vec<u8>)
		- RemoveBlock(Vec<u8>)
		- RemovePin(Vec<u8>)

	 	Dht Commands:
	 	- FindPeer(Vec<u8>)
		- GetProviders(Vec<u8>)*/
	#[derive(PartialEq, Eq, Clone, Debug, Encode, Decode, TypeInfo)]
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	pub enum IpfsCommand {
		// Connection Commands
		ConnectTo(Vec<u8>),
		DisconnectFrom(Vec<u8>),

		// Data Commands
		AddBytes(Vec<u8>),
		CatBytes(Vec<u8>),
		InsertPin(Vec<u8>),
		RemoveBlock(Vec<u8>),
		RemovePin(Vec<u8>),

		// DHT Commands
		FindPeer(Vec<u8>),
		GetProviders(Vec<u8>),
	}

	/** CommandRequest is used for issuing requests to an ocw that can be connected to connections to IPFS nodes. **/
	#[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	pub struct CommandRequest<T: Config> {
		pub identifier: [u8; 32],
		pub requester: T::AccountId,
		pub ipfs_commands: Vec<IpfsCommand>,
	}

	// The pallet's runtime storage items.
	// https://docs.substrate.io/v3/runtime/storage

	/** Store a list of Commands for the ocw to process */
	#[pallet::storage]
	#[pallet::getter(fn commands)]
	pub type Commands<T: Config> = StorageValue<_, Vec<CommandRequest<T>>>;

	/** Pallets use events to inform users when important changes are made.
	- ConnectionRequested(T::AccountId),
	- DisconnectedRequested(T::AccountId),
	- QueuedDataToAdd(T::AccountId),
	- QueuedDataToCat(T::AccountId),
	- QueuedDataToPin(T::AccountId),
	- QueuedDataToRemove(T::AccountId),
	- QueuedDataToUnpin(T::AccountId),
	- FindPeerIssued(T::AccountId),
	- FindProvidersIssued(T::AccountId),
	https://docs.substrate.io/v3/runtime/events-and-errors*/
	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		ConnectionRequested(T::AccountId),
		DisconnectedRequested(T::AccountId),
		QueuedDataToAdd(T::AccountId),
		QueuedDataToCat(T::AccountId),
		QueuedDataToPin(T::AccountId),
		QueuedDataToRemove(T::AccountId),
		QueuedDataToUnpin(T::AccountId),
		FindPeerIssued(T::AccountId),
		FindProvidersIssued(T::AccountId),
		OcwCallback(T::AccountId),
	}

	/** Errors inform users that something went wrong.
	- CannotCreateRequest,
	- RequestTimeout,
	- RequestFailed,
	- NoneValue,
	- StorageOverflow,
	- FailedToAcquireLock,
	- OffchainSignedTxError,
	*/
	#[pallet::error]
	pub enum Error<T> {
		CannotCreateRequest,
		RequestTimeout,
		RequestFailed,
		NoneValue,
		StorageOverflow,
		FailedToAcquireLock,
		OffchainSignedTxError,
	}

	/** Modify the genesis state of the blockchain.
		This is pretty pointless atm since we overwrite the data each block.

		TODO: Until we have a working signed ocw callback is would mostly likely be used for configuring initial state in specs.

		Optional values are:
		-	commands: Vec<CommandRequest<T>>
	 **/
	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub commands: Vec<CommandRequest<T>>,
	}

	#[cfg(feature = "std")]
	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> GenesisConfig<T> {
			GenesisConfig { commands: Vec::<CommandRequest<T>>::new() }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
		fn build(&self) {
			// TODO: Allow the configs to use strings, then convert them to the Vec<u8> collections.
			Commands::<T>::set(Some(Vec::<CommandRequest<T>>::new()));
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn offchain_worker(block_number: T::BlockNumber) {
			if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker started with ***") { error!("IPFS: Error occurred during `print_metadata`"); }

			if let Err(_err) = Self::process_command_requests(block_number) { error!("IPFS: command request"); }
		}
	}


	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/** Mark a `multiaddr` as a desired connection target. The connection target will be established
			during the next run of the off-chain `connection` process.
			- Example: /ip4/127.0.0.1/tcp/4001/p2p/<Ipfs node Id> */
		#[pallet::weight(100_000)]
		pub fn ipfs_connect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::ConnectTo(address));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::ConnectionRequested(requester)))
		}

		/** Queues a `Multiaddr` to be disconnected. The connection will be severed during the next
		run of the off-chain `connection` process.
		**/
		#[pallet::weight(500_000)]
		pub fn ipfs_disconnect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::DisconnectFrom(address));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::DisconnectedRequested(requester)))
		}

		/// Adds arbitrary bytes to the IPFS repository. The registered `Cid` is printed out in the logs.
		#[pallet::weight(200_000)]
		pub fn ipfs_add_bytes(origin: OriginFor<T>, received_bytes: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::AddBytes(received_bytes));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToAdd(requester)))
		}

		/** Fin IPFS data by the `Cid`; if it is valid UTF-8, it is printed in the logs.
		Otherwise the decimal representation of the bytes is displayed instead. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_cat_bytes(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::CatBytes(cid));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToCat(requester)))
		}

		/** Add arbitrary bytes to the IPFS repository. The registered `Cid` is printed in the logs **/
		#[pallet::weight(300_000)]
		pub fn ipfs_remove_block(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::RemoveBlock(cid));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToRemove(requester)))
		}

		/** Pin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_insert_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::InsertPin(cid));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append( ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToPin(requester)))
		}

		/** Unpin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_remove_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::RemovePin(cid));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToUnpin(requester)))
		}

		/** Find addresses associated with the given `PeerId` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_peer(origin: OriginFor<T>, peer_id: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::FindPeer(peer_id));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::FindPeerIssued(requester)))
		}

		/** Find the list of `PeerId`'s known to be hosting the given `Cid` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_providers(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let mut commands = Vec::<IpfsCommand>::new();
			commands.push(IpfsCommand::GetProviders(cid));

			let ipfs_command = CommandRequest::<T> { identifier: generate_id::<T>(), requester: requester.clone(), ipfs_commands: commands };

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::FindProvidersIssued(requester)))
		}

		#[pallet::weight(0)]
		pub fn ocw_callback(origin: OriginFor<T>, identifier: [u8; 32], data: Vec<u8>) -> DispatchResult {
			info!("IPFS: #ocw_callback !!!");
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

			Self::command_callback(&callback_command.unwrap(), data);

			Ok(Self::deposit_event(Event::OcwCallback(signer)))
		}
	}

	impl<T: Config> Pallet<T> {
		// helper functions

		/**
			Iterate over all of the Active CommandRequests calling them.

			Im sure some more logic will go in here.
		 */
		fn process_command_requests(block_number: T::BlockNumber) -> Result<(), Error<T>> {
			let commands: Vec<CommandRequest<T>> = Commands::<T>::get().unwrap_or(Vec::<CommandRequest<T>>::new());

			for command_request in commands {

				let result= process_command::<T>(block_number, command_request,PROCESSED_COMMANDS);
			}

			Ok(())
		}


		/** Output the current state of IPFS worker */
		fn print_metadata(message: &str) -> Result<(), Error<T>> {
			let peers = if let IpfsResponse::Peers(peers) = ipfs_request::<T>(IpfsRequest::Peers)? {
				peers
			} else {
				Vec::new()
			};

			let peers_count= peers.len();

			info!("{}", message);
			info!("IPFS: Is currently connected to {} peers", peers_count);
			info!("IPFS: CommandRequest size: {}", Commands::<T>::decode_len().unwrap_or(0));

			Ok(())
		}

		/** callback to the on-chain validators to continue processing the CID  **/
		fn signed_callback(command_request: &CommandRequest<T>, data: Vec<u8>) -> Result<(), Error<T>> {
			let signer = Signer::<T, T::AuthorityId>::all_accounts();
			if !signer.can_sign() {
				error!("*** IPFS *** ---- No local accounts available. Consider adding one via `author_insertKey` RPC.");

				return Err(Error::<T>::RequestFailed)?
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

			Ok(())
		}

		// - Ideally the callback function can be override by another pallet that is coupled to this one allowing for custom functionality.
		// - data can be used for callbacks IE add a cid to the signer / uploader.
		// - Validate a connected peer has the CID, and which peer has it etc.
		// TODO: Result
		fn command_callback(command_request: &CommandRequest<T>, data: Vec<u8>) -> Result<(), ()>{
			// TODO:
			Ok(())
		}

	}
}
