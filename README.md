# Substrate &middot; [![GitHub license](https://img.shields.io/badge/license-GPL3%2FApache2-blue)](#LICENSE) [![GitLab Status](https://gitlab.parity.io/parity/substrate/badges/master/pipeline.svg)](https://gitlab.parity.io/parity/substrate/pipelines) [![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](docs/CONTRIBUTING.adoc) [![Matrix](https://img.shields.io/matrix/substrate-technical:matrix.org)](https://matrix.to/#/#substrate-technical:matrix.org)

<p align="center">
  <img src="/docs/media/sub.gif">
</p>

Substrate is a next-generation framework for blockchain innovation 🚀.

## Trying it out

Simply go to [docs.substrate.io](https://docs.substrate.io) and follow the
[installation](https://docs.substrate.io/main-docs/install/) instructions. You can
also try out one of the [tutorials](https://docs.substrate.io/tutorials/).

## Contributions & Code of Conduct

Please follow the contributions guidelines as outlined in [`docs/CONTRIBUTING.adoc`](docs/CONTRIBUTING.adoc). In all communications and contributions, this project follows the [Contributor Covenant Code of Conduct](docs/CODE_OF_CONDUCT.md).

## Security

The security policy and procedures can be found in [`docs/SECURITY.md`](docs/SECURITY.md).

## License

- Substrate Primitives (`sp-*`), Frame (`frame-*`) and the pallets (`pallets-*`), binaries (`/bin`) and all other utilities are licensed under [Apache 2.0](LICENSE-APACHE2).
- Substrate Client (`/client/*` / `sc-*`) is licensed under [GPL v3.0 with a classpath linking exception](LICENSE-GPL3).

The reason for the split-licensing is to ensure that for the vast majority of teams using Substrate to create feature-chains, then all changes can be made entirely in Apache2-licensed code, allowing teams full freedom over what and how they release and giving licensing clarity to commercial teams.

In the interests of the community, we require any deeper improvements made to Substrate's core logic (e.g. Substrate's internal consensus, crypto or database code) to be contributed back so everyone can benefit.

### ❗ ✨✨✨ Polkadot APAC hackathon submission ✨✨✨❗
### ✨✨✨ Implementation of IPFS in Substrate Runtime! (Frame V3) ✨✨✨

It's a fully functional IPFS node in the runtime that's able to communicate P2P across its swarm of peers.

Using our IPFS pallet you can create a "Pocket Dimension" where data can be added through an IPFS command that's picked up and processed by an offchain worker. If things are successful, then the offchain worker will log and send the response from IPFS.

### Features :
**IPFS into substrates runtime**
- Add rusts implementation of IPFS into the substrate runtime, where an off-chain worker is able to interact with IPFS and connected peers.

**pallet-ipfs-core:**
- Provides scaffolding for other pallets to easily interface with IPFS via extrinsics.

**pallet-ipfs-example:**
- Almost full implementation of available IPFS commands with a matching extrinsic. Coupled to ipfs-core pallet.
- Each extrinsic calls a single IPFS command.

**pallet-pocket-mints:**
- Example minting pallet that verifies the existence and location of a CID before minting it to an address. Coupled to ipfs-core pallet.

For Implementing a new IPFS pallet see our ipfs-template [pull request](https://github.com/DanHenton/pocket-substrate/pull/10)

### How to run:
1) Download or clone the repository and navigate to it in the terminal.
2) Compile substrate using: (Make sure you have at least 5GB of available RAM :wink: )
```
    cargo build --release
    ./target/release/substrate --dev --tmp
    
    Or 
    
    cargo run --release -- --dev --tmp
```
3) Your node should start up with something similar to the bellow image. Note that we can see the IPFS PeerID . This means we have successfully launched substrate with IPFS in its runtime. ![node-start](https://user-images.githubusercontent.com/7565646/145338654-58595d55-bbcd-4882-95e7-b83751ee00f8.png)


4) Using https://polkadot.js.org/apps/#/explorer connect to locally running node
5) Navigate to https://polkadot.js.org/apps/#/extrinsics here you can interact with the `ipfsExample` and `PocketMints` pallets.
6) View events in the explorer:  https://polkadot.js.org/apps/#/explorer
7) See the updates to the chainstate:  https://polkadot.js.org/apps/#/chainstate

```
Using ipfsExample Pallet: 
Submit and sign ipfsAddBytes extrinsic with "Hello world"
In the Explorer tab, see event "ipfsExample.AddedCid" to find the returned CID "Qme2yvVdPdpHiGmoPMgTJubrJ7rud3SHSCU7PqNUvYBecA"

Submit and sign ipfsCatBytes extrinsic with the CID from the event
In the Explorer tab, see event "ipfsExample.CatBytes" to find the bytes "Hello world"
```
```
Using pocketMints Pallet:
Submit and sign mintCid with an empty address ("") and a CID. We will use the same CID from ipfsExample usecase above "Qme2yvVdPdpHiGmoPMgTJubrJ7rud3SHSCU7PqNUvYBecA"
In the Explorer tab, see event "pocketMints.MintedCid" with returned CID 

In the Developer tab, go to ChainState, selected state query pocketMints
Then select walletAssets for the original signer, click + button, find the returned CID from last event "Qme2yvVdPdpHiGmoPMgTJubrJ7rud3SHSCU7PqNUvYBecA" 
```

### Architecture overview:

**Architecture of the IPFS Pallets:**
![pocket-dimension-pallets](https://user-images.githubusercontent.com/7565646/145698271-4dc1a728-78e6-4310-9dc5-c0712a252490.png)

**Example process flow of interacting with the IPFS in substrate runtime:**
![Pocket-dimensions-interacting-with-ipfs](https://user-images.githubusercontent.com/7565646/145332202-fb829876-4b1f-44f0-8d06-d0878bd8cd53.png)
