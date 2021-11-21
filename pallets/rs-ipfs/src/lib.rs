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
	use log::{ error, info };
	use std::str;

	const TIMEOUT_DURATION: u64 = 1_000;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
	}

	/** Commands for interacting with IPFS connections'
		- ConnectTo(OpaqueMultiaddr)
		- DisconnectFrom(OpaqueMultiaddr) **/
	#[derive(PartialEq, Eq, Clone, Debug, Encode, Decode, TypeInfo)]
	pub enum ConnectionCommand {
		ConnectTo(OpaqueMultiaddr),
		DisconnectFrom(OpaqueMultiaddr),
	}

	/** Commands for interacting with IPFS `Content Addressed Data` functionality
	- AddBytes(Vec<u8>)
	- CatBytes(Vec<u8>)
	- InsertPin(Vec<u8>)
	- RemoveBlock(Vec<u8>)
	- RemovePin(Vec<u8>)**/
	#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
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
	pub(super) type DataQueue<T: Config> = StorageValue<_, Vec<DataCommand>>;

	#[pallet::storage]
	#[pallet::getter(fn dht_queue)]
	pub(super) type DhtQueue<T: Config> = StorageValue<_, Vec<DhtCommand>>;

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

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		/** synchronize with offchain_worker activities
			Use the ocw lock for this
			offchain_worker configuration **/
		fn offchain_worker(block_number: T::BlockNumber) {
			if let Err(err) = Self::connection() {
				error!("IPFS: Error occurred during `connection` {:?}", err);
			}

			if let Err(err) = Self::data_request() {
				error!("IPFS: Error occurred during `data_request` {:?}", err);
			}

			if let Err(err) = Self::handle_dht_request() {
				error!("IPFS: Error occurred during `handle_dht_request` {:?}", err);
			}

			if let Err(err) = Self::print_metadata() {
				error!("IPFS: Error occurred during `print_metadata` {:?}", err);
			}
		}
	}

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
			let command = ConnectionCommand::ConnectTo(OpaqueMultiaddr(address));

			ConnectionQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::ConnectionRequested(who)))
		}

		/** Queues a `Multiaddr` to be disconnected. The connection will be severed during the next
		run of the off-chain `connection` process.
		**/
		#[pallet::weight(500_000)]
		pub fn ipfs_disconnect(origin: OriginFor<T>, address: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let command = ConnectionCommand::DisconnectFrom(OpaqueMultiaddr(address));

			// let mut exists = false;
			// let existing_connections= ConnectionQueue::<T>::get().unwrap();
			// for connection in existing_connections {
			// 	match connection {
			// 		ConnectionCommand::ConnectTo(OpaqueMultiaddr(address)) => {
			// 			println!("Found matching connection");
			// 			exists = true;
			// 		}
			// 		_ => {}
			// 	}
			// }

			ConnectionQueue::<T>::append(command);
			Ok(Self::deposit_event(Event::DisconnectedRequested(who)))
		}

		/// Adds arbitrary bytes to the IPFS repository. The registered `Cid` is printed out in the logs.
		#[pallet::weight(200_000)]
		pub fn ipfs_add_bytes(origin: OriginFor<T>, data: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::AddBytes(data));
			Ok(Self::deposit_event(Event::QueuedDataToAdd(who)))
		}

		/// Fin IPFS data by the `Cid`; if it is valid UTF-8, it is printed in the logs.
		/// Otherwise the decimal representation of the bytes is displayed instead.
		#[pallet::weight(100_000)]
		pub fn ipfs_cat_bytes(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::CatBytes(cid));
			Ok(Self::deposit_event(Event::QueuedDataToCat(who)))
		}

		/// Add arbitrary bytes to the IPFS repository. The registered `Cid` is printed in the logs
		#[pallet::weight(300_000)]
		pub fn ipfs_remove_block(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::RemoveBlock(cid));
			Ok(Self::deposit_event(Event::QueuedDataToRemove(who)))
		}

		/// Pin a given `Cid` non-recursively.
		#[pallet::weight(100_000)]
		pub fn ipfs_insert_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::InsertPin(cid));
			Ok(Self::deposit_event(Event::QueuedDataToPin(who)))
		}

		/// Unpin a given `Cid` non-recursively.
		#[pallet::weight(100_000)]
		pub fn ipfs_remove_pin(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DataQueue::<T>::append( DataCommand::RemovePin(cid));
			Ok(Self::deposit_event(Event::QueuedDataToUnpin(who)))
		}

		/// Find addresses associated with the given `PeerId`
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_peer(origin: OriginFor<T>, peer_id: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::append( DhtCommand::FindPeer(peer_id));
			Ok(Self::deposit_event(Event::FindPeerIssued(who)))
		}

		/// Find the list of `PeerId`'s known to be hosting the given `Cid`
		#[pallet::weight(100_000)]
		pub fn ipfs_dht_find_providers(origin: OriginFor<T>, cid: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			DhtQueue::<T>::append( DhtCommand::GetProviders(cid));
			Ok(Self::deposit_event(Event::FindProvidersIssued(who)))
		}
	}

	impl<T: Config> Pallet<T> {
		// `private?` helper functions

		/// Send a request to the local IPFS node; Can only be called in an offchain worker.
		fn ipfs_request(request: IpfsRequest, deadline: impl Into<Option<Timestamp>>) -> Result<IpfsResponse, Error<T>> {
			let ipfs_request = ipfs::PendingRequest::new(request).map_err(|_| Error::CannotCreateRequest)?;

			ipfs_request.try_wait(deadline)
				.map_err(|_| Error::<T>::RequestTimeout)?
				.map(|req| req.response)
				.map_err(|error| { Error::<T>::RequestFailed })
		}

		/// TODO: refactor this
		fn connection() -> Result<(), Error<T>> {
			for command in ConnectionQueue::<T>::get().unwrap() {
				let deadline = Self::deadline(); //Some(timestamp().add(Duration::from_millis(1_000))); // TODO: Refactor this into a constant

				match command {
					ConnectionCommand::ConnectTo(address) => {
						match Self::ipfs_request(IpfsRequest::Connect(address.clone()), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: connected to {}", str::from_utf8(&address.0).expect("Our own calls can be trusted to be UTF-8"));
							},
							Ok(_) => { unreachable!("only Success can be a reponse for that request type") },
							Err(e) => { Self::failure_message(&"connect") }, // TODO: Add error to error message
						}
					},

					ConnectionCommand::DisconnectFrom(address) => {
						match Self::ipfs_request(IpfsRequest::Disconnect(address.clone()), deadline) {
							Ok(IpfsResponse::Success) => {
								info!("IPFS: disconnected from {}", str::from_utf8(&address.0).expect("Our own calls can be trusted to be UTF-8"));
							},
							Ok(_) => {unreachable!("only Success can be a response for that request type")},
							Err(e) => { Self::failure_message(&"disconnect", ) }, // TODO: Add error to error message
						}
					},
				}
			}

			Ok(())
		}

		fn data_request() -> Result<(), Error<T>> {
			let data_messages = DataQueue::<T>::get().unwrap();
			let queue_length = data_messages.len();

			if queue_length > 0 {
				info!("IPFS: has {} number of data requests to process", queue_length);
			}


			for command in data_messages {
				let deadline = Self::deadline();

				match command {
					DataCommand::AddBytes(bytes_to_add) => {
						match Self::ipfs_request(IpfsRequest::AddBytes(bytes_to_add.clone()), deadline) {
							Ok(IpfsResponse::AddBytes(cid)) => { info!("IPFS: added data with Cid: {:?}", str::from_utf8(&cid)) }
							_ => { Self::failure_message(&"add bytes!") }, // TODO: handle error message
						}
					},
					DataCommand::CatBytes(bytes_to_cat) => {
						match Self::ipfs_request(IpfsRequest::CatBytes(bytes_to_cat.clone()), deadline) {
							Ok(IpfsResponse::CatBytes(bytes_received)) => {
								if let Ok(str) = str::from_utf8(&bytes_received) {
									info!("Ipfs: received bytes: {:?}", str::from_utf8(&bytes_received))
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
							_ => { Self::failure_message(&format!("pin cid: {:?}", utf8_cid)); }
						}
					},
					DataCommand::RemovePin(cid) => {
						let utf8_cid= str::from_utf8(&cid).unwrap();

						match Self::ipfs_request(IpfsRequest::RemovePin(cid.clone(), false), deadline) {
							Ok(IpfsResponse::Success) => { info!("IPFS: removed pinned cid {:?}", utf8_cid); }
							_ => { Self::failure_message(&format!("remove pin {:?}", utf8_cid)); }
						}
					},
					DataCommand::RemoveBlock(cid) => {
						let utf8_cid = str::from_utf8(&cid).unwrap();

						match Self::ipfs_request(IpfsRequest::RemoveBlock(cid.clone()), deadline) {
							Ok(IpfsResponse::Success) => { info!("IPFS: removed block {:?}", utf8_cid); }
							_ => { Self::failure_message(&format!("remove block {}", utf8_cid)); }
						}

					}
				}
			}

			Ok(())
		}

		fn handle_dht_request() -> Result<(), Error<T>> {
			for command in DhtQueue::<T>::get().unwrap() {
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

		fn print_metadata() -> Result<(), Error<T>> {
			let deadline = Self::deadline();

			let peers = if let IpfsResponse::Peers(peers) = Self::ipfs_request(IpfsRequest::Peers, deadline)? {
				peers
			} else {
				vec![]
			};

			let peers_count= peers.len();

			info!("IPFS: Is currently connected to {} peers", peers_count);

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
