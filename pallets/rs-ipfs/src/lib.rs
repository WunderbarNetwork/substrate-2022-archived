#![feature(option_result_contains)]
#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>

use codec::{ Decode, Encode };
use frame_system::{
	self as system,
	offchain::{ AppCrypto, CreateSignedTransaction, SendSignedTransaction, SignedPayload, Signer, SigningTypes, SubmitTransaction },
};
use sp_runtime::{
	offchain::{
		ipfs,
		storage::{MutateStorageError, StorageRetrievalError, StorageValueRef}
	},
	transaction_validity::{InvalidTransaction, TransactionValidity, ValidTransaction},
	RuntimeDebug
};
use sp_std::vec::Vec;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod crypto {
	use sp_std::prelude::*;
	use frame_system::offchain::SigningTypes;
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_core::crypto::key_types;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner
	};

	app_crypto!(sr25519, key_types::IPFS);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}

	// Implemented for mock runtime in tests
	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_system::pallet_prelude::*;
	use sp_io::{ offchain::{ timestamp }, hashing::blake2_128 };
	use frame_support::{
		dispatch::DispatchResult,
		pallet_prelude::*,
		sp_runtime::traits::Hash,
	};
	use frame_system::offchain::{ SubmitTransaction, CreateSignedTransaction };

	#[cfg(feature = "std")]
	use frame_support::serde::{ Deserialize, Serialize };

	use sp_core::{
		offchain::{Duration, IpfsRequest, IpfsResponse, OpaqueMultiaddr, Timestamp}
	};
	use log::{ error, info };
	use sp_std::{ str, vec::Vec, }; //fmt::Formatter

	const TIMEOUT_DURATION: u64 = 1_000;

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config + CreateSignedTransaction<Call<Self>> {
		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// overarching dispatch call type.
		type Call: From<Call<Self>>;
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	/** Commands for interacting with IPFS connections'
		- ConnectTo(OpaqueMultiaddr)
		- DisconnectFrom(OpaqueMultiaddr) **/
	#[derive(PartialEq, Eq, Clone, Debug, Encode, Decode, TypeInfo)]
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	pub enum ConnectionCommand {
		ConnectTo(Vec<u8>),
		DisconnectFrom(Vec<u8>),
	}

	/** Commands for interacting with IPFS `Content Addressed Data` functionality
	- AddBytes(Vec<u8>)
	- CatBytes(Vec<u8>)
	- InsertPin(Vec<u8>)
	- RemoveBlock(Vec<u8>)
	- RemovePin(Vec<u8>)**/
	#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	pub enum DataCommand {
		AddBytes(Vec<u8>),
		CatBytes(Vec<u8>),
		InsertPin(Vec<u8>),
		RemoveBlock(Vec<u8>),
		RemovePin(Vec<u8>),
	}

	/** Commands for interacting with IPFS `Distributed Hash Tables` functionality
	- FindPeer(Vec<u8>)
	- GetProviders(Vec<u8>)**/
	#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	pub enum DhtCommand {
		FindPeer(Vec<u8>),
		GetProviders(Vec<u8>),
	}

	#[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	pub struct CommandRequest<T: Config> {
		pub requester: T::AccountId,
		pub hash_id: T::Hash,
		pub connection_command: Option<ConnectionCommand>,
		pub data_command: Option<DataCommand>,
		pub dht_command: Option<DhtCommand>,
	}

	// The pallet's runtime storage items.
	// https://docs.substrate.io/v3/runtime/storage
	#[pallet::storage]
	#[pallet::getter(fn connection_queue)]
	pub(super) type ConnectionQueue<T: Config> = StorageValue<_, Vec<ConnectionCommand>>;

	/// TODO: this should be thread-safe and user-safe
	#[pallet::storage]
	#[pallet::getter(fn data_queue)]
	pub(super) type DataQueue<T: Config> = StorageValue<_, Vec<DataCommand>>;

	#[pallet::storage]
	#[pallet::getter(fn command_queue)]
	pub(super) type CommandQueue<T: Config> = StorageValue<_, Vec<CommandRequest<T>>>;

	#[pallet::storage]
	#[pallet::getter(fn dht_queue)]
	pub(super) type DhtQueue<T: Config> = StorageValue<_, Vec<DhtCommand>>;

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
	https://docs.substrate.io/v3/runtime/events-and-errors **/
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
	**/
	#[pallet::error]
	pub enum Error<T> {
		CannotCreateRequest,
		RequestTimeout,
		RequestFailed,
		NoneValue,
		StorageOverflow,
		OffchainSignedTxError,
		NoLocalAcctForSigning,
	}

	/** Modify the genesis state of the blockchain.
		This is pretty pointless atm since we overwrite the data each block.
	Optional values are:
	- connection_queue: Vec<ConnectionCommand>
	- data_queue: Vec<DataCommand>
	- dht_queue: Vec<DhtCommand>
	 **/
	#[pallet::genesis_config]
	pub struct GenesisConfig {
		pub connection_queue: Vec<ConnectionCommand>,
		pub data_queue: Vec<DataCommand>,
		pub dht_queue: Vec<DhtCommand>,
	}

	#[cfg(feature = "std")]
	impl Default for GenesisConfig {
		fn default() -> GenesisConfig {
			GenesisConfig {
				connection_queue: Vec::<ConnectionCommand>::new(),
				data_queue: Vec::<DataCommand>::new(),
				dht_queue: Vec::<DhtCommand>::new(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig {
		fn build(&self) {
			// TODO: Allow the configs to use strings, then convert them to the Vec<u8> collections.
			ConnectionQueue::<T>::set(Some(self.connection_queue.clone()));
			DataQueue::<T>::set(Some(self.data_queue.clone()));
			DhtQueue::<T>::set(Some(self.dht_queue.clone()));
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_block_number: BlockNumberFor<T>) -> Weight {
			ConnectionQueue::<T>::set(Some(Vec::<ConnectionCommand>::new()));
			DataQueue::<T>::set(Some(Vec::<DataCommand>::new()));
			DhtQueue::<T>::set(Some(Vec::<DhtCommand>::new()));

			0
		}

		/** synchronize with offchain_worker activities
			Use the ocw lock for this
			offchain_worker configuration **/
		fn offchain_worker(_block_number: T::BlockNumber) {
			// if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker started with ***") {
			// 	error!("IPFS: Error occurred during `print_metadata`");
			// }

			if let Err(_err) = Self::command_request() {
				error!("IPFS: command request");
			}

			 if let Err(_err) = Self::connection_requests() {
			 	error!("IPFS: Error occurred during `connection`");
			 }

			 if let Err(_err) = Self::data_request() {
			 	error!("IPFS: Error occurred during `data_request`");
			 }

			if let Err(_err) = Self::handle_dht_request() {
				error!("IPFS: Error occurred during `handle_dht_request`");
			}

			// if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker finished with ***") {
			// 	error!("IPFS: Error occurred during `print_metadata`");
			// }
		}
	}

	// Used in unsigned transactions.
/*	#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, scale_info::TypeInfo)]
	pub struct Payload<Public> {
		hash_id: Vec<u8>,
		cid: Vec<u8>,
		public: Public,
	}

	impl<S: SigningTypes> SignedPayload<S> for Payload<S::Public> {
		fn public(&self) -> S::Public {
			info!("Signing payload: {:?}", self.public.clone());
			self.public.clone()
		}
	}
*/

	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// fn deposit_event() = default;

		/// Mark a `multiaddr` as a desired connection target. The connection target will be established
		/// during the next run of the off-chain `connection` process.
		#[pallet::weight(100_000)]
		pub fn ipfs_connect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let command = ConnectionCommand::ConnectTo(address);

			ConnectionQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::ConnectionRequested(who)))
		}

		/** Queues a `Multiaddr` to be disconnected. The connection will be severed during the next
		run of the off-chain `connection` process.
		**/
		#[pallet::weight(500_000)]
		pub fn ipfs_disconnect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let command = ConnectionCommand::DisconnectFrom(address);

			ConnectionQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::DisconnectedRequested(who)))
		}

		/// Adds arbitrary bytes to the IPFS repository. The registered `Cid` is printed out in the logs.
		#[pallet::weight(200_000)]
		pub fn ipfs_add_bytes(origin: OriginFor<T>, received_bytes: Vec<u8>) -> DispatchResult {
			let requester: T::AccountId = ensure_signed(origin)?;

			// TODO: add the requester to the hash
			let hash_id = T::Hashing::hash(&received_bytes.clone());

			let command = CommandRequest::<T> {
				requester: requester.clone(),
				hash_id,
				data_command: Some(DataCommand::AddBytes(received_bytes)),
				connection_command: None,
				dht_command: None
			};

			CommandQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::QueuedDataToAdd(requester.clone())))
		}

		/** Fin IPFS data by the `Cid`; if it is valid UTF-8, it is printed in the logs.
		Otherwise the decimal representation of the bytes is displayed instead. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_cat_bytes(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::CatBytes(cid));
			Ok(Self::deposit_event(Event::QueuedDataToCat(who)))
		}

		/** Add arbitrary bytes to the IPFS repository. The registered `Cid` is printed in the logs **/
		#[pallet::weight(300_000)]
		pub fn ipfs_remove_block(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::RemoveBlock(cid));
			Ok(Self::deposit_event(Event::QueuedDataToRemove(who)))
		}

		/** Pin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_insert_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::InsertPin(cid));
			Ok(Self::deposit_event(Event::QueuedDataToPin(who)))
		}

		/** Unpin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_remove_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::RemovePin(cid));
			Ok(Self::deposit_event(Event::QueuedDataToUnpin(who)))
		}

		/** Find addresses associated with the given `PeerId` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_peer(origin: OriginFor<T>, peer_id: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::append( DhtCommand::FindPeer(peer_id));
			Ok(Self::deposit_event(Event::FindPeerIssued(who)))
		}

		/** Find the list of `PeerId`'s known to be hosting the given `Cid` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_providers(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::append( DhtCommand::GetProviders(cid));
			Ok(Self::deposit_event(Event::FindProvidersIssued(who)))
		}

		#[pallet::weight(0)]
		pub fn ocw_callback(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			info!("Offchain callback hit from: {:?}", str::from_utf8(&cid));

			let signer = ensure_signed(origin)?;

			info!("Offchain callback hit from: {:?} {:?}", signer , str::from_utf8(&cid));
			Ok(Self::deposit_event(Event::OcwCallback(signer)))
		}
	}

	impl<T: Config> Pallet<T> {
		// `private?` helper functions

		/** Send a request to the local IPFS node; Can only be called in an offchain worker. **/
		fn ipfs_request(request: IpfsRequest, deadline: impl Into<Option<Timestamp>>) -> Result<IpfsResponse, Error<T>> {
			let ipfs_request = ipfs::PendingRequest::new(request).map_err(|_| Error::CannotCreateRequest)?;

			ipfs_request.try_wait(deadline)
				.map_err(|_| Error::<T>::RequestTimeout)?
				.map(|req| req.response)
				.map_err(|_error| { Error::<T>::RequestFailed })
		}

		/** Process off-chain Connections for the running IPFS instance. **/
		fn connection_requests() -> Result<(), Error<T>> {
			for command in ConnectionQueue::<T>::get().unwrap_or(Vec::new()) {
				let deadline = Self::deadline();
				match command {
					ConnectionCommand::ConnectTo(address) => {
						match Self::ipfs_request(IpfsRequest::Connect(OpaqueMultiaddr(address.clone())), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: connected to {}", str::from_utf8(&address).expect("Our own calls can be trusted to be UTF-8"));
							},
							Ok(_) => { unreachable!("only Success can be a response for that request type") },
							Err(_e) => { Self::failure_message(&"connect") }, // TODO: Add error to error message
						}
					},

					ConnectionCommand::DisconnectFrom(address) => {
						match Self::ipfs_request(IpfsRequest::Disconnect(OpaqueMultiaddr(address.clone())), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: disconnected from {}", str::from_utf8(&address).expect("Our own calls can be trusted to be UTF-8"));
							},
							Ok(_) => {unreachable!("only Success can be a response for that request type")},
							Err(_e) => { Self::failure_message(&"disconnect", ) }, // TODO: Add error to error message
						}
					},
				}
			}

			Ok(())
		}

		fn command_request() -> Result<(), Error<T>> {
			let commands: Vec<CommandRequest<T>> = CommandQueue::<T>::get().unwrap_or(Vec::<CommandRequest<T>>::new());

			for command in commands {
				let deadline = Self::deadline();

				match command.data_command.clone().unwrap() {
					DataCommand::AddBytes(bytes_to_add) => {
						// info!("Found bytes to add from: {:?} bytes: {:?} hash_id: {:?}", command.requester.clone(), bytes_to_add.clone(), command.hash_id.clone());

						match Self::ipfs_request(IpfsRequest::AddBytes(bytes_to_add.clone()), deadline) {
							Ok(IpfsResponse::AddBytes(cid)) => {
								info!("IPFS: added data with Cid: {:?}", str::from_utf8(&cid));
								let callback = Self::signed_callback(cid);
								return Ok(());
							},
							_ => { Self::failure_message(&"add bytes!") }

						}
					}
					_ => { Self::failure_message(&"To run commands") }, // TODO: handle error message
				}
			}

			Ok(())
		}

		//** Process the data request with IPFS offchain worker **/
		fn data_request() -> Result<(), Error<T>> {
			let data_messages = DataQueue::<T>::get().unwrap_or(Vec::new());
			if DataQueue::<T>::decode_len() == 0.into() { return Ok(()); }

			for (index, command) in data_messages.iter().enumerate() {
				let deadline = Self::deadline();

				match command {
					DataCommand::AddBytes(bytes_to_add) => {
						match Self::ipfs_request(IpfsRequest::AddBytes(bytes_to_add.clone()), deadline) {
							Ok(IpfsResponse::AddBytes(cid)) => {
								info!("IPFS: added data with Cid: {:?}", str::from_utf8(&cid));
								// info!("IPFS CALLBACK DATA: {:?} {:?}", Vec::from("add_bytes".as_bytes()), bytes_to_add.clone());

								// let call = Call::ipfs_ocw_callback({ who, Vec::from("add_bytes".as_bytes())), hash, bytes_to_add.clone() });
							}
							_ => { Self::failure_message(&"add bytes!") }, // TODO: handle error message
						}
					},
					DataCommand::CatBytes(bytes_to_cat) => {
						match Self::ipfs_request(IpfsRequest::CatBytes(bytes_to_cat.clone()), deadline) {
							Ok(IpfsResponse::CatBytes(bytes_received)) => {
								if let Ok(str) = str::from_utf8(&bytes_received) {
									info!("Ipfs: received bytes: {:?}", str);
								} else {
									info!("IPFS: received bytes: {:x?}", bytes_received)
								}
							}
							_ => { Self::failure_message(&"cat bytes!") }, // TODO: handle error message
						}
					},
					DataCommand::InsertPin(cid) => {
						let utf8_cid= str::from_utf8(&cid).unwrap();

						match Self::ipfs_request(IpfsRequest::InsertPin(cid.clone(), false), deadline) {
							Ok(IpfsResponse::Success) => { info!("IPFS: pinned data wit cid {:?}", utf8_cid); }
							_ => { Self::failure_message(&"pin cid" ) }
						}
					},
					DataCommand::RemovePin(cid) => {
						let utf8_cid= str::from_utf8(&cid).unwrap();

						match Self::ipfs_request(IpfsRequest::RemovePin(cid.clone(), false), deadline) {
							Ok(IpfsResponse::Success) => { info!("IPFS: removed pinned cid {:?}", utf8_cid); }
							_ => { Self::failure_message( &"remove pin" )}
						}
					},
					DataCommand::RemoveBlock(cid) => {
						let utf8_cid = str::from_utf8(&cid).unwrap();

						match Self::ipfs_request(IpfsRequest::RemoveBlock(cid.clone()), deadline) {
							Ok(IpfsResponse::Success) => { info!("IPFS: removed block {:?}", utf8_cid); }
							_ => { Self::failure_message(&"remove block") }
						}

					}
				}
			}

				// DataQueue::<T>::set(Some(remaining_messages));
			// 	info!("end of mut remaining_messages {:?}", remaining_messages);
			// 	*data_messages = Some(remaining_messages);
			// });

			Ok(())
		}

		fn handle_dht_request() -> Result<(), Error<T>> {
			for command in DhtQueue::<T>::get().unwrap_or(Vec::new()) {
				let deadline = Self::deadline();

				match command {
					DhtCommand::FindPeer(peer_id) => {
						match Self::ipfs_request(IpfsRequest::FindPeer(peer_id.clone()), deadline) {
							Ok(IpfsResponse::FindPeer(addresses)) => {
								info!("IPFS: Found peer {:?} with addresses {:?}",
									str::from_utf8(&peer_id),
									addresses.iter()
										.map(|address| str::from_utf8(&address.0))
										.collect::<Vec<_>>()
								);
							},
							_ => { Self::failure_message(&"find DHT addresses")}

						}
					},
					DhtCommand::GetProviders(cid) => {
						match Self::ipfs_request(IpfsRequest::GetProviders(cid.clone()), deadline) {
							Ok(IpfsResponse::GetProviders(peer_ids)) => {
								info!("IPFS: found the following providers of {:?}: {:?}",
									str::from_utf8(&cid),
									peer_ids.iter()
										.map(|peer| str::from_utf8(peer))
										.collect::<Vec<_>>()
								);
							},
							_ => { Self::failure_message(&"get DHT providers")} }
					}
				}
			}

			Ok(())
		}



		fn print_metadata(message: &str) -> Result<(), Error<T>> {
			let deadline = Self::deadline();

			let peers = if let IpfsResponse::Peers(peers) = Self::ipfs_request(IpfsRequest::Peers, deadline)? {
				peers
			} else {
          Vec::new()
			};

			let peers_count= peers.len();

			info!("{}", message);
			info!("IPFS: Is currently connected to {} peers", peers_count);
			info!("IPFS: CommandRequest size: {}", CommandQueue::<T>::decode_len().unwrap_or(0));
			info!("IPFS: ConnectionQueue size: {}", ConnectionQueue::<T>::decode_len().unwrap_or(0));
			info!("IPFS: DataQueue size: {}", DataQueue::<T>::decode_len().unwrap_or(0));
			info!("IPFS: DhtQueue size: {}", DhtQueue::<T>::decode_len().unwrap_or(0));

			Ok(())
		}

		fn deadline() -> Option<Timestamp> {
			Some(timestamp().add(Duration::from_millis(TIMEOUT_DURATION)))
		}

		fn failure_message(message: &str, ) {
			error!("IPFS: failed to {:?}", message)
		}

		/** callback to the on-chain validators to continue processing the CID  **/
		fn signed_callback(cid: Vec<u8>) -> Result<(), Error<T>> {
			let signer = Signer::<T, T::AuthorityId>::all_accounts();
			if !signer.can_sign() {
				error!("*** IPFS *** ---- No local accounts available. Consider adding one via `author_insertKey` RPC.");

				return Err(Error::<T>::RequestFailed)?
			}

			info!("attempting to sign a transaction");

			let results  = signer.send_signed_transaction(|_account| {
				Call::ocw_callback{ cid: cid.clone() }
			});

			info!("*** IPFS: result size: {:?}", results.len());
			for (account, result) in &results {
				info!("** reporting results **");

				match result {
					Ok(()) => { info!("callback sent: {:?}, cid: {:?}", account.public, cid.clone()); },
					Err(e) => { error!("Failed to submit transaction {:?}", e)}
				}
			}

			Ok(())
		}
	}
}
