# fileOS

**English** | [简体中文](README.zh-CN.md)

fileOS is a replicated file system for people and the small groups they are part of. Its state is kept by a number of nodes, and a node keeps only the spaces it has been given and has agreed to keep. There is no global consensus, no ordering node and no global content DHT.

> [!NOTE]
> A prototype. Formats will change.

## System model

**Principals.** Devices, nodes and apps are named by ed25519 public keys. A group is a principal with no key; its members act for it, and its owner is a key or another group. A person is an identity (`did:fileos`) with a number of nodes.

**Spaces.** A space is the unit of replication, ownership and retention. Its ID is the hash of its first entry. Each writer has a signed, append-only log in the space; each entry names the entries it comes after, so the logs form one causal graph. The state is a function of the set of entries alone, so replicas that have the same entries are in the same state (strong eventual consistency), and every node may write while offline.

**Files.** Within a space, names are flat; a directory is a prefix. A file has a fixed ID, the hash of the entry that made it and the index of the act. A rename is a new version. Concurrent versions of a file are kept side by side, and none is dropped without the user knowing. Content is cut into chunks at points chosen by the content and addressed by BLAKE3 hash.

**Authorization.** A grant gives exact permissions, allow or deny, on a name prefix or on one file by ID, to a principal. Grants are given on only with the same or less, and taking one back takes back what was given on from it. Validity is decided from the causal past of each entry, with strong removal: acts concurrent with the removal of their author's access have no effect. No clock is used.

**Encryption.** Content is encrypted with a random key for each file, which is wrapped with the key of the prefix the file is in. Prefix keys are derived one way from the key of the prefix above, from a random root key for each epoch, so a read grant on a prefix gives the keys of that prefix only. Keys are sent to each device with HPKE (RFC 9180). Taking back access starts a new epoch. Keys that no kept data needs are dropped at checkpoints, so deleted content cannot be opened again. Names, sizes and authors are not encrypted, because every node checks every entry.

**Checkpoints.** The owner signs the hash of the state in the past of a checkpoint. Entries concurrent with a checkpoint are not taken; their authors write them again on top. Nodes compact to the checkpoint, start from a signed snapshot, and thin old versions by a retention policy, so storage does not grow without a limit.

**Layers.** The kernel is the spaces, sync over libp2p, the app runtime (wasmtime) and a local HTTP gateway. Apps are WebAssembly components that use the file system through WIT interfaces. Each app runs in a sandbox with its own key and is checked against its grants. The kernel may have plugins for parts of its own work; there are none today.

## Try it

With the devenv shell (`direnv allow`):

```sh
cargo build -p fileos                       # the program
apps                                     # the apps, in target/apps/
fileos node init                            # a node in ~/.fileos
fileos node run                             # the network, the apps and the gateway
```

In another terminal:

```sh
fileos space new /photos                    # a space, mounted at /photos
fileos app install target/apps/Photos.app   # the photos app
fileos fs put /photos/cat.png cat.png
fileos fs ls /photos
fileos node status
```

To share `/photos` with another node: `fileos space grant /photos <its node key> read-write` here, then `fileos space mount <space ID> /photos` there.

## What is there

- [x] **Spaces**
  - [x] A signed log for each writer, in one causal graph; no ordering node; writes while offline
  - [x] Files with fixed IDs, versions side by side, renames that keep the ID
  - [x] Content in chunks cut where the content says, named by hash
  - [x] Checkpoints, compaction, snapshots for new nodes, retention thinner with age
- [x] **Access**
  - [x] Grants: exact permissions or templates, on prefixes or on files, allow or deny
  - [x] Given on only smaller; taken back with everything that came from them; strong removal
  - [x] Groups, owned by keys or by groups, and groups in groups
  - [ ] Checks on writes by the app that manages the files; the picker for apps
- [ ] **Apps**
  - [x] Apps come as `.app` files (a zip with no compression: the manifest, and the rest in `tar.zst`)
  - [x] Each app has its own key, gets only what its manifest asks for, and runs with limits on fuel and memory
  - [x] Each app sees only its own view: mounts at names it knows, private data, and grants given at run time (`fileos app grant`)
  - [x] Apps answer HTTP at their own origin, `<app-id>.localhost`, with `wasi:http`
  - [x] The photos app: a gallery, upload, and an index of image sizes
  - [x] The chat app: rooms with one file for each message, in a space that you share
  - [x] Apps are workers: a new instance for each event and each request
  - [x] Schedules in the manifest: the kernel sends a request to the app's own handler
  - [ ] `wasi:filesystem`, watch options, events on the nodes that people pick
- [ ] **Network and keys**
  - [x] libp2p: TCP, Noise, Yamux, mDNS, relays, hole punching, AutoNAT
  - [x] Encryption: keys that follow prefixes, HPKE (RFC 9180), new keys when access is taken back, keys destroyed on delete
  - [ ] QUIC
- [ ] **Identity**
  - [x] One identity for a person: a DID (`did:fileos`, the `did:plc` method with no directory), apart from the keys of their nodes
  - [x] Rotation keys and a signing key; node certificates in an identity space
  - [x] An identity acting for all its devices: a grant to a `did:fileos` is a grant to each of its nodes
  - [ ] Domain names as other names of an identity (as AT Protocol handles)
  - [ ] Finding all the nodes of an identity through rendezvous nodes
- [x] **Running a node**
  - [x] One program (`fileos`) and one data directory
  - [x] A local gateway and a command line

## Documentation

Read the [fileOS documentation](https://docs.fileos.app/en-us/).

[User manual](docs/src/content/docs/en-us/user/index.md) · [Running a node](docs/src/content/docs/en-us/admin/index.md) · [Architecture](docs/src/content/docs/en-us/dev/fileos.md) · [Writing apps](docs/src/content/docs/en-us/dev/apps.md) · [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md). The app SDK (`apps/fileos-app`) and the interfaces in `wit/` are MIT OR Apache-2.0.
