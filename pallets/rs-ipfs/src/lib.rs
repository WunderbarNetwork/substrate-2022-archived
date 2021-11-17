#![feature(option_result_contains)]
#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/v3/runtime/frame>
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_system::pallet_prelude::*;
	use sp_io::{ offchain::timestamp };
	use frame_support::{ dispatch::DispatchResult, pallet_prelude::*, sp_runtime::traits::Hash };
	use sp_core::offchain::{ Duration, IpfsRequest, IpfsResponse, OpaqueMultiaddr, Timestamp };
	use sp_runtime::offchain:: {
		ipfs,
		storage::{ MutateStorageError, StorageRetrievalError, StorageValueRef },
		storage_lock::{ BlockAndTimeDeadline, StorageLock },
	};
	use std::collections::{ VecDeque };
	use log::{ error, info };

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
	}

	#[derive(PartialEq, Eq, Clone, Debug, Encode, Decode, TypeInfo)]
	pub enum ConnectionCommand {
		ConnectTo(OpaqueMultiaddr),
		DisconnectFrom(OpaqueMultiaddr),
	}

	#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
	pub enum DataCommand {
		AddBytes(Vec<u8>),
		CatBytes(Vec<u8>),
		InsertPin(Vec<u8>),
		RemoveBlock(Vec<u8>),
		RemovePin(Vec<u8>),
	}

	#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
	pub enum DhtCommand {
		FindPeer(Vec<u8>),
		GetProviders(Vec<u8>),
	}

	// The pallet's runtime storage items.
	// https://docs.substrate.io/v3/runtime/storage
	#[pallet::storage]
	#[pallet::getter(fn connection_queue)]
	pub(super) type ConnectionQueue<T: Config> = StorageValue<_, Vec<ConnectionCommand>>;

	/// TODO: this should be thread-safe and user-safe
	#[pallet::storage]
	#[pallet::getter(fn data_queue)]
	pub(super) type DataQueue<T: Config> = StorageValue<_, VecDeque<DataCommand>>;

	#[pallet::storage]
	#[pallet::getter(fn dht_queue)]
	pub(super) type DhtQueue<T: Config> = StorageValue<_, VecDeque<DhtCommand>>;

	// Pallets use events to inform users when important changes are made.
	// https://docs.substrate.io/v3/runtime/events-and-errors
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
	}

	// Errors inform users that something went wrong.
	#[pallet::error]
	pub enum Error<T> {
		CannotCreateRequest,
		RequestTimeout,
		RequestFailed,
		NoneValue,
		StorageOverflow,
	}

	// #[pallet::genesis_config]
	// pub struct GenesisConfig<T: Config> {
	// 	pub connection_queue: Vec<ConnectionCommand>,
	// }
	//
	// #[cfg(feature = "std")]
	// impl<T: Config> Default for GenesisConfig<T> {
	// 	fn default() -> GenesisConfig<T> {
	// 		GenesisConfig { connection_queue: vec![] }
	// 	}
	// }
	//
	// #[pallet::genesis_build]
	// impl<T: Config> GenesisBuild<T> for GenesisConfig<T> {
	// 	fn build(&self) {
	// 		for Some(vec![]) in &self.connection_queue {
	// 			let _ = vec![];
	// 		}
	// 	}
	// 	// fn build(&self) {
	// 	// 	&self.kitties = vec![];
	// 	// }
	// }

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/// synchronize with offchain_worker activities
		/// Use the ocw lock for this
/*		fn on_initialize(block_number: T::BlockNumber) -> Weight {

			ConnectionQueue::kill();
			DhtQueue::kill();

			if block_number % 2.into() == 1.into() {
				DataQueue::kill();
			}

			0
		}
*/
		/// offchain_worker configuration
		fn offchain_worker(block_number: T::BlockNumber) {
			// process connect/disconnect commands

			// Self::connection_housekeeping()

			// if let Err(e) = Self::connection_housekeeping() {
			// 	error!("IPFS: Encountered an error during connection housekeeping: {:?}", e);
			// }

/*			if let Err(e) = Self::handle_dht_requests() {
				error!("IPFS: Encountered an error while processing DHT requests {:?}", e);
			}

			// process Ipfs::{add, get} queues every block. TODO: refactor this!
			if block_number % 2.into() == 1.into() {
				if let Err(e) = Self::handle_data_requests() {
					error!("IPFS: Encountered an error while processing data requests: {:?}", e);
				}
			}

			if block_number % 5.into() == 0.into() {
				if let Err(e) = Self::print_metadata() {
					error!("IPFS: Encountered an error while obtaining data requests: {:?}", e);
				}
			}
*/		}
	}

	// Dispatchable functions allows users to interact with the pallet and invoke state changes.
	// These functions materialize as "extrinsics", which are often compared to transactions.
	// Dispatchable functions must be annotated with a weight and must return a DispatchResult.
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// fn deposit_event() = default;

		/// Mark a `multiaddr` as a desired connection target. The connection target will be established
		/// during the next run of the off-chain `connection_housekeeping` process.
		#[pallet::weight(100_000)]
		pub fn ipfs_connect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let command = ConnectionCommand::ConnectTo(OpaqueMultiaddr(address));

			ConnectionQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::ConnectionRequested(who)))
		}
/*
		/// Queues a `Multiaddr` to be disconnected. The connection will be severed during the next
		/// run of the off-chain `connection_housekeeping` process.
		#[pallet::weight(500_000)]
		pub fn ipfs_disconnect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let command = ConnectionCommand::DisconnectFrom(OpaqueMultiaddr(address));

			ConnectionQueue::<T>::mutate( |queue| if !queue.unwrap().contains(&command) { queue.unwrap().push_back(command) });
			Ok(Self::deposit_event(Event::DisconnectedRequested(who)))
		}

		/// Adds arbitrary bytes to the IPFS repository. The registered `Cid` is printed out in the logs.
		#[pallet::weight(200_000)]
		pub fn ipfs_add_bytes(origin: OriginFor<T>, data: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::mutate( |queue| queue.unwrap().push_back(DataCommand::AddBytes(data)));
			Ok(Self::deposit_event(Event::QueuedDataToAdd(who)))
		}

		/// Fin IPFS data by the `Cid`; if it is valid UTF-8, it is printed in the logs.
		/// Otherwise the decimal representation of the bytes is displayed instead.
		#[pallet::weight(100_000)]
		pub fn ipfs_cat_bytes(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::mutate( |queue| queue.unwrap().push_back(DataCommand::CatBytes(cid)));
			Ok(Self::deposit_event(Event::QueuedDataToCat(who)))
		}

		/// Add arbitrary bytes to the IPFS repository. The registered `Cid` is printed in the logs
		#[pallet::weight(300_000)]
		pub fn ipfs_remove_block(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::mutate( |queue| queue.unwrap().push_back(DataCommand::RemoveBlock(cid)));
			Ok(Self::deposit_event(Event::QueuedDataToRemove(who)))
		}

		/// Pin a given `Cid` non-recursively.
		#[pallet::weight(100_000)]
		pub fn ipfs_insert_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::mutate( |queue| queue.unwrap().push_back(DataCommand::InsertPin(cid)));
			Ok(Self::deposit_event(Event::QueuedDataToPin(who)))
		}

		/// Unpin a given `Cid` non-recursively.
		#[pallet::weight(100_000)]
		pub fn ipfs_remove_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::mutate( |queue| queue.unwrap().push_back(DataCommand::RemovePin(cid)));
			Ok(Self::deposit_event(Event::QueuedDataToUnpin(who)))
		}

		/// Find addresses associated with the given `PeerId`
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_peer(origin: OriginFor<T>, peer_id: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::mutate( |queue|queue.unwrap().push_back(DhtCommand::FindPeer(peer_id)));
			Ok(Self::deposit_event(Event::FindPeerIssued(who)))
		}

		/// Find the list of `PeerId`'s known to be hosting the given `Cid`
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_providers(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::mutate( |queue| queue.unwrap().push_back(DhtCommand::GetProviders(cid)));
			Ok(Self::deposit_event(Event::FindProvidersIssued(who)))
		}
*/	}

/*	impl<T: Config> Pallet<T> {
		// `private?` helper functions

		/// Send a request to the local IPFS node; Can only be called in an offchain worker.
		fn ipfs_request(request: IpfsRequest, deadline: impl Into<Option<Timestamp>>) -> Result<IpfsResponse, Error<T>> {
			let ipfs_request = ipfs::PendingRequest::new(request).map_err(|_| Error::CannotCreateRequest)?;

			ipfs_request.try_wait(deadline)
				.map_err(|_| Error::<T>::RequestTimeout)?
				.map(|req| req.response)
				.map_err(|error| {
					if let ipfs::Error::IoError(err) = error {
						error!("IPFS: request failed with error: {}", str::from_utf8(&err).unwrap());
					} else {
						error!("IPFS: request failed with error: {:?}", error)
					}

					Error::<T>::RequestFailed
				})
		}

		/// TODO: refactor this
		fn connection_housekeeping() { //-> Result<(), Error<T>> {
			let mut deadline;

			for command in ConnectionQueue {
				deadline = Some(timestamp().add(Duration::from_millis(1_000))); // TODO: Refactor this into a constant

				match command {
					ConnectionCommand::ConnectTo(address) => {
						match Self::ipfs_request(IpfsRequest::Connect(address.clone()), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: connected to {}", str::from_utf8(&address.0).expect("Our own calls can be trusted to be UTF-8; qed"));
							}
							Ok(_) => unreachable!("only Success can be a reponse for that request type; qed"),
							Err(e) => error!("IPFS: Connection error"), // TODO: Add error to error message
						}
					}

					ConnectionCommand::DisconnectFrom(address) => {
						match Self::ipfs_request(IpfsRequest::Disconnect(address.clone()), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: disconnected from {}", str::from_utf8(&address.0).expect("Our own calls can be trusted to be UTF-8; qed"));
							}
							Ok(_) => unreachable!("only Success can be a reponse for that request type; qed"),
							Err(e) => error!("IPFS: Connection error"), // TODO: Add error to error message
						}
					}
				}
			}

		}
	}
*/
}
