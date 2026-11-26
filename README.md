# fileOS

**English** | [简体中文](README.zh-CN.md)

fileOS is a file system that a number of nodes keep together: your computers, and the computers of people you know.

> [!NOTE]
> A prototype. Formats will change.

## How it is made

- **Kernel.** Every folder is a signed log of changes, put in order by one node. File contents are blobs named by their BLAKE3 hash. People, devices, nodes and apps are ed25519 keys. Nodes sync folders over libp2p.
- **Apps.** WebAssembly components. Each app runs in a sandbox, has its own key and sees only the folders it is given.
- **Kernel plugins.** WebAssembly components that do a part of the kernel's work. Today there is one: the engine, which says who may do what.

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
- [x] **Running a node**
  - [x] One program (`fos`) and one data directory
  - [x] A local gateway and a command line

## Documentation

[User manual](docs/src/content/docs/en-us/user/index.md) · [Running a node](docs/src/content/docs/en-us/admin/index.md) · [Architecture](docs/src/content/docs/en-us/dev/fileos.md) · [Writing apps and engines](docs/src/content/docs/en-us/dev/apps.md) · [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md). The app SDK (`apps/fos-app`) and the interfaces in `wit/` are MIT OR Apache-2.0.
