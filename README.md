# fileOS

**English** | [简体中文](README.zh-CN.md)

A file system that your computers and the computers of people you know keep together. Everything you use on it (photos, chat, passwords) is an app.

> [!NOTE]
> A prototype. Formats will change.

## How it is made

fileOS has two parts: the **kernel** and **apps**. The kernel can take **kernel plugins**.

- **The kernel** gives one thing: files that mean the same on every node. A folder is a signed log of changes, and files are blobs named by their hash. A shared folder is a folder that a number of nodes each agree to keep.
- **Apps** are user space. People use fileOS only through apps. An app is a WebAssembly component. It runs in a sandbox, has its own key, and sees only the folders you give it.
- **Kernel plugins** change how the kernel does its work, not what apps see. The first one is the rules for access (in the code, the _engine_). Places to keep data and the encryption of data on disk may come next.

Over all of this is one rule: **do not break user space.** The kernel keeps two contracts: with apps (the `app` world in [`wit/fos.wit`](wit/fos.wit)) and with other nodes (the formats and the protocol). We only add to them. We do not change or take away what is in them. A kernel plugin may only pick between results that the contract already has: it may say "no" to a write, but it may not change what a write is. The same app, with the same data and the same rules, does the same thing on every node, with any kernel plugins.

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

To share `/photos` with another node: `fos grant /photos <its node key> all` here, then `fos mount <folder ID> /photos` there. See the [user manual](docs/src/content/docs/en-us/user/index.md) and the [administration manual](docs/src/content/docs/en-us/admin/index.md).

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

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md): apps and kernel plugins that use only the Plugin Interface may be under any license. The app SDK (`apps/fos-app`) and the interfaces in `wit/` are MIT OR Apache-2.0.
