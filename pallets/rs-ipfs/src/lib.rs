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
	use sp_io::{ offchain::timestamp , hashing::blake2_128 };
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
	use sp_std::str;

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
	#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
	#[scale_info(skip_type_params(T))]
	pub struct CommandRequest<T: Config> {
		pub requester: T::AccountId,
		pub uuid: Vec<u8>,
		pub ipfs_command: IpfsCommand,
	}

	impl<T: Config> CommandRequest<T> {


		fn new(requester: T::AccountId, ipfs_command: IpfsCommand) -> Self {
			let mut tmp_req = CommandRequest::<T> {
				uuid: vec![],
				requester,
				ipfs_command,
			};

			let hash_id = T::Hashing::hash_of(&tmp_req).into();

			tmp_req.uuid = hash_id;

			info!("{:?} {:?}", tmp_req.uuid, hash_id);

			tmp_req
		}
	}

	// The pallet's runtime storage items.
	// https://docs.substrate.io/v3/runtime/storage
	#[pallet::storage]
	#[pallet::getter(fn commands)]
	pub(super) type Commands<T: Config> = StorageValue<_, Vec<CommandRequest<T>>>;

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
	*/
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
			Commands::<T>::set(Some(self.commands.clone()));
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_block_number: BlockNumberFor<T>) -> Weight {
			Commands::<T>::set(Some(Vec::<CommandRequest<T>>::new()));

			0
		}

		/** synchronize with offchain_worker activities
			Use the ocw lock for this
			offchain_worker configuration **/
		fn offchain_worker(_block_number: T::BlockNumber) {
			if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker started with ***") { error!("IPFS: Error occurred during `print_metadata`"); }

			if let Err(_err) = Self::process_command_requests() { error!("IPFS: command request"); }

			if let Err(_err) = Self::print_metadata(&"*** IPFS off-chain worker finished with ***") { error!("IPFS: Error occurred during `print_metadata`"); }
		}
	}


	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Mark a `multiaddr` as a desired connection target. The connection target will be established
		/// during the next run of the off-chain `connection` process.
		#[pallet::weight(100_000)]
		pub fn ipfs_connect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;

			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::ConnectTo(address));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::ConnectionRequested(requester)))
		}

		/** Queues a `Multiaddr` to be disconnected. The connection will be severed during the next
		run of the off-chain `connection` process.
		**/
		#[pallet::weight(500_000)]
		pub fn ipfs_disconnect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::DisconnectFrom(address));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::DisconnectedRequested(requester)))
		}

		/// Adds arbitrary bytes to the IPFS repository. The registered `Cid` is printed out in the logs.
		#[pallet::weight(200_000)]
		pub fn ipfs_add_bytes(origin: OriginFor<T>, received_bytes: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::AddBytes(received_bytes));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToAdd(requester)))
		}

		/** Fin IPFS data by the `Cid`; if it is valid UTF-8, it is printed in the logs.
		Otherwise the decimal representation of the bytes is displayed instead. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_cat_bytes(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::CatBytes(cid));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToCat(requester)))
		}

		/** Add arbitrary bytes to the IPFS repository. The registered `Cid` is printed in the logs **/
		#[pallet::weight(300_000)]
		pub fn ipfs_remove_block(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::RemoveBlock(cid));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToRemove(requester)))
		}

		/** Pin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_insert_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::InsertPin(cid));

			Commands::<T>::append( ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToPin(requester)))
		}

		/** Unpin a given `Cid` non-recursively. **/
		#[pallet::weight(100_000)]
		pub fn ipfs_remove_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::RemovePin(cid));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::QueuedDataToUnpin(requester)))
		}

		/** Find addresses associated with the given `PeerId` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_peer(origin: OriginFor<T>, peer_id: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::FindPeer(peer_id));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::FindPeerIssued(requester)))
		}

		/** Find the list of `PeerId`'s known to be hosting the given `Cid` **/
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_providers(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			let ipfs_command = CommandRequest::new(requester.clone(), IpfsCommand::GetProviders(cid));

			Commands::<T>::append(ipfs_command);
			Ok(Self::deposit_event(Event::FindProvidersIssued(requester)))
		}

		#[pallet::weight(0)]
		pub fn ocw_callback(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			info!("Offchain callback hit from: {:?}", str::from_utf8(&cid.clone()));

			let signer = ensure_signed(origin)?;

			info!("Offchain callback hit from: {:?} {:?}", signer , str::from_utf8(&cid.clone()));
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


		/**
			Iterate over all of the Active CommandRequests calling them.

			Im sure some more logic will go in here.
		 */
		fn process_command_requests() -> Result<(), Error<T>> {
			let commands: Vec<CommandRequest<T>> = Commands::<T>::get().unwrap_or(Vec::<CommandRequest<T>>::new());

			for command_request in commands {

				let result = Self::process_command(command_request);
			}

			Ok(())
		}

		/** Process each IPFS `command_request`
			1) lock the request for asynchronous processing
			2) Call the CommandRequest.ipfs_command initializing the request
		 */
		fn process_command(command_request: CommandRequest<T>) -> Result<(), Error<T>>{
			// TODO: Lock using the command_request_id

			let deadline = Self::deadline();

			match command_request.ipfs_command {
				IpfsCommand::ConnectTo(address) 	   	=> { Self::connect_to(address, deadline) },
				IpfsCommand::DisconnectFrom(address) 	=> { Self::disconnect_from(address, deadline) },
				IpfsCommand::AddBytes(bytes_to_add)  	=> { Self::add_bytes(bytes_to_add, deadline) },
				IpfsCommand::CatBytes(cid)				=> { Self::cat_bytes(cid, deadline) },
				IpfsCommand::InsertPin(cid) 			=> { Self::insert_pin(cid, deadline) },
				IpfsCommand::RemovePin(cid) 			=> { Self::remove_pin(cid, deadline) },
				IpfsCommand::RemoveBlock(cid) 			=> { Self::remove_block(cid, deadline) },
				IpfsCommand::FindPeer(peer_id) 		=> { Self::find_peer(peer_id, deadline) },
				IpfsCommand::GetProviders(cid) 		=> { Self::get_providers(cid, deadline) },
			}

			Ok(())
		}

		/** Connect to a given IPFS address
			TODO: add options and documentation */
		fn connect_to(address: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::Connect(OpaqueMultiaddr(address.clone())), deadline) {
				Ok(IpfsResponse::Success) => { info!("IPFS: connected to {}", str::from_utf8(&address).expect("UTF-8")) },
				_ => { Self::failure_message(&"connect_to") },
			}
		}

		/** Disconnect from a given IPFS address
		 	TODO: add options and documentation */
		fn disconnect_from(address: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::Disconnect(OpaqueMultiaddr(address.clone())), deadline) {
				Ok(IpfsResponse::Success) => { info!("IPFS: disconnected from {}", str::from_utf8(&address).expect("UTF-8")) },
				_ => { Self::failure_message(&"disconnect_from")}
			}
		}

		/** Upload u8 bytes to connected IPFS nodes */
		fn add_bytes(bytes_to_add: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::AddBytes(bytes_to_add.clone()), deadline) {
				Ok(IpfsResponse::AddBytes(cid)) => {
					info!("IPFS: added data with Cid: {:?}", str::from_utf8(&cid));
					let callback = Self::signed_callback(cid);
				},
				_ => { Self::failure_message(&"add bytes!") }
			}
		}

		/** Output the bytes of a given IPFS cid */
		fn cat_bytes(cid: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::CatBytes(cid.clone()), deadline) {
				Ok(IpfsResponse::CatBytes(bytes_received)) => {
					if let Ok(utf8_str) = str::from_utf8(&bytes_received) {
						info!("Ipfs: received bytes: {:?}", utf8_str);
					} else {
						info!("IPFS: received bytes: {:x?}", bytes_received)
					}
				},
				_ => { Self::failure_message(&"cat bytes!") },
			}
		}

		/** Pin an IPFS cid */
		fn insert_pin(cid: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::InsertPin(cid.clone(), false), deadline) {
				Ok(IpfsResponse::Success) => { info!("IPFS: pinned data wit cid {:?}", str::from_utf8(&cid.clone())); }
				_ => { Self::failure_message(&"pin cid" ) }
			}
		}

		/** Remove a Pin from an IPFS cid */
		fn remove_pin(cid: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::RemovePin(cid.clone(), false), deadline) {
				Ok(IpfsResponse::Success) => { info!("IPFS: removed pinned cid {:?}", str::from_utf8(&cid.clone())); }
				_ => { Self::failure_message( &"remove pin" )}
			}
		}

		/** Remove a block from IPFS */
		fn remove_block(cid: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::RemoveBlock(cid.clone()), deadline) {
				Ok(IpfsResponse::Success) => { info!("IPFS: removed block {:?}", str::from_utf8(&cid.clone())); }
				_ => { Self::failure_message(&"remove block") }
			}
		}

		/** Find the multiaddresses associated with a Peer ID
			TODO: documentation for peer_id */
		fn find_peer(peer_id: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::FindPeer(peer_id.clone()), deadline) {
				Ok(IpfsResponse::FindPeer(addresses)) => {
					let peers = addresses.iter().map(|address| str::from_utf8(&address.0)).collect::<Vec<_>>();
					info!("IPFS: Found peer {:?} with addresses {:?}", str::from_utf8(&peer_id), peers);
				},
				_ => { Self::failure_message(&"find DHT addresses")}

			}
		}

		/** Find the providers for a given cid */
		fn get_providers(cid: Vec<u8>, deadline: Option<Timestamp>) {
			match Self::ipfs_request(IpfsRequest::GetProviders(cid.clone()), deadline) {
				Ok(IpfsResponse::GetProviders(peer_ids)) => {
					let providers = peer_ids.iter().map(|peer| str::from_utf8(peer)).collect::<Vec<_>>();
					info!("IPFS: found the following providers of {:?}: {:?}", str::from_utf8(&cid.clone()), providers);
				},
				_ => { Self::failure_message(&"get DHT providers")} }
		}

		/** Output the current state of IPFS worker */
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
			info!("IPFS: CommandRequest size: {}", Commands::<T>::decode_len().unwrap_or(0));

			Ok(())
		}

		/** callback to the on-chain validators to continue processing the CID  **/
		fn signed_callback(cid: Vec<u8>) -> Result<(), Error<T>> {
			let signer = Signer::<T, T::AuthorityId>::all_accounts();
			if !signer.can_sign() {
				error!("*** IPFS *** ---- No local accounts available. Consider adding one via `author_insertKey` RPC.");

				return Err(Error::<T>::RequestFailed)?
			}

			info!("attempting to sign a transaction");

			let results  = signer.send_signed_transaction(|_account| { Call::ocw_callback{ cid: cid.clone() } });

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

		fn deadline() -> Option<Timestamp> {
			Some(timestamp().add(Duration::from_millis(TIMEOUT_DURATION)))
		}

		fn failure_message(message: &str, ) {
			error!("IPFS: failed to {:?}", message)
		}
	}
}
