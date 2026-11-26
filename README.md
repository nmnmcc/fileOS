# fileOS

**English** | [简体中文](README.zh-CN.md)

fileOS is a replicated file system for people and the small groups they are part of. Its state is kept by a number of nodes, and a node keeps only the folders it has agreed to keep. There is no global consensus and no global content DHT.

> [!NOTE]
> A prototype. Formats will change.

## System model

**Principals.** People, devices, nodes and apps are named by ed25519 public keys.

**Folders.** A folder is the unit of replication, ordering and authorization. Its state is fully given by one signed, append-only log, and its ID is the hash of the first record in it. The records are `put`, `delete`, `append`, `mount` and `policy`. File contents are blobs addressed by their BLAKE3 hash. A folder may mount other folders at paths, which gives a tree of names.

**Order.** Every folder has one ordering node, and the total order that it gives is the main line of the folder. The ordering node may give the job to another node. A write to a file takes the place of the earlier version, which stays in the history. Appends to a stream commute, so they never conflict.

**Authorization.** The kernel checks facts: signatures and hashes. The decisions are made by the engine of each node, which its owner chooses. Because nodes make their decisions independently, replicas may fork when their policies are different. The system does not force agreement.

**Layers.** The kernel is the file system, sync over libp2p and the app runtime (wasmtime). Apps are WebAssembly components that use the file system through WIT interfaces. Each app runs in a sandbox with its own key and sees only the paths given to it. Kernel plugins are WebAssembly components that do a part of the kernel's work. The only one today is the engine.

## Try it

With the devenv shell (`direnv allow`):

```sh
cargo build -p fos                                   # the program
apps                                                 # the engine and apps, in target/apps/
fos init --engine target/apps/Capability.app         # a node in ~/.fileos
fos run                                              # the network, the apps and the gateway
```

In another terminal:

```sh
fos new photos /photos                               # a folder, mounted at /photos
fos install target/apps/Photos.app                   # the photos app
fos put /photos/cat.png cat.png
fos kv photos                                        # what the photos app found
fos status
```

To share `/photos` with another node: `fos grant /photos <its node key> all` here, then `fos mount <folder ID> /photos` there.

## What is there

- [x] **Folders**
  - [x] A signed log for each folder: files with all their versions, streams, mounts
  - [x] File contents as blobs named by their hash
  - [x] One ordering node for each folder; it can give the job to another node
  - [x] A copy forks, and says why, when its node's rules do not take an entry
- [x] **Kernel plugins: the rules for access (the engine)**
  - [x] One engine for each node, chosen by its owner, with no limits
  - [x] The default engine: capabilities, given on with less power, taken back with everything that came from them
- [ ] **Apps**
  - [x] Apps are `.app` folders with a manifest, and they sync like any folder
  - [x] Each app has its own key, gets only what its manifest asks for, and runs with limits on fuel and memory
  - [x] The photos app: image sizes into an index on the node
  - [ ] Apps that answer HTTP (`handle-http`), extensions for file types, web interfaces
  - [ ] Files over WebDAV, a vault for Bitwarden clients, chat, contacts and calendars
- [ ] **Network**
  - [x] libp2p: TCP, Noise, Yamux, mDNS, relays, hole punching, AutoNAT
  - [x] Writes wait for their ordering node and go when it is back
  - [ ] QUIC
  - [ ] Folders encrypted with MLS (RFC 9420)
- [ ] **Identity**
  - [x] One identity for a person, apart from the keys of their nodes
  - [x] Node certificates, signed by the identity, in an identity folder
  - [ ] Finding all the nodes of an identity through rendezvous nodes
- [x] **Running a node**
  - [x] One program (`fos`) and one data directory
  - [x] A local gateway and a command line

## Documentation

[User manual](docs/src/content/docs/en-us/user/index.md) · [Running a node](docs/src/content/docs/en-us/admin/index.md) · [Architecture](docs/src/content/docs/en-us/dev/fileos.md) · [Writing apps and engines](docs/src/content/docs/en-us/dev/apps.md) · [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md). The app SDK (`apps/fos-app`) and the interfaces in `wit/` are MIT OR Apache-2.0.
