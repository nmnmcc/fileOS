---
title: "Architecture: fileOS"
description: The core is only a file system that syncs between nodes. Everything else, and the rules for access too, is a WebAssembly app.
---

:::note[Status: prototype]
A prototype of this design is in the repository. The section
[What the prototype has](#what-the-prototype-has) says which parts are built
and which are not yet. The name "fileOS" is a working name.
:::

:::caution[The design is changing]
Three parts of this page are old: one ordering node for each folder, MLS for
encryption, and apps installed on each node. The new design is in
[Data: sync, conflicts, storage and keys](../data/),
[Apps as workers](../workers/) and [HTTP and the browser](../http/).
:::

## In one sentence

**The core keeps only facts: data, signatures, keys and transport. Every rule
and every feature is WebAssembly code that the user picks: the authorization
engine makes the rules, and apps make the features.**

## Why one base

When a system has many separate ideas (groups, items, grants, tokens,
replicas, relays, plugins), each new feature brings one more idea. Each idea
needs its own code, its own tests and its own rules for sync.

fileOS has one base, and everything is made on top of it. It took its first
idea from IPFS: files are named by their hash. But it is much smaller than
IPFS, because it is made for the computers of people and families, not for a
global network:

- No global DHT for content. Nodes find each other through people.
- Nothing is public by default.
- A node does not keep anything that it did not agree to keep.

## The five ideas

The whole system has five ideas. Everything else is made from them.

| Idea   | What it is                                                 |
| ------ | ---------------------------------------------------------- |
| Folder | A signed log and a set of blobs                            |
| Key    | A person, a device, a node and an app are each a key       |
| Node   | A place that keeps folders, syncs them and runs code       |
| Engine | The rules of a node for access, picked by its owner (WASM) |
| App    | A program that reads and writes folders (WASM)             |

Things that people know are made from these five:

| What you see                  | What it is                                                  |
| ----------------------------- | ----------------------------------------------------------- |
| A group                       | A folder that a number of nodes agree to keep               |
| A chat                        | A stream in a folder                                        |
| An album, a shared drive      | Files in a folder                                           |
| A backup                      | A node that keeps the encrypted bytes of a folder           |
| A relay or a mailbox          | A node that keeps some entries for a time, for someone else |
| Using someone else's node     | The engine of that node says yes to you                     |
| Installing an app             | Putting an app folder into `/apps/`                         |
| Updating an app               | Syncing that folder                                         |
| Changing the rules for access | Changing the engine of your node                            |
| Moving to a new server        | Letting a new node sync your folders                        |

## Layers

```
┌───────────────────────────────────────────────┐
│ Apps (WASM)  photos chat vault search login … │
├───────────────────────────────────────────────┤
│ Engine (WASM, one per node, the owner picks)  │
├───────────────────────────────────────────────┤
│ WIT: fos:fs fos:watch fos:authz + wasi:http … │
├───────────────────────────────────────────────┤
│ Core: file system · signatures and hashes ·   │
│       sync · keys (MLS) · app runtime ·       │
│       gateway                                 │
├───────────────────────────────────────────────┤
│ wasmtime · libp2p · disk                      │
└───────────────────────────────────────────────┘
```

The core has no "should" in it. It does not know what a photo, a chat or a
password is. It does not know who the owner of a group is. It does not know
Bitwarden, CalDAV or OpenID Connect.

## The file system

### Folders

A **folder** is the unit of sync, order, access and encryption.

- The ID of a folder is the hash of the entry that made it. It never changes.
- All of the state of a folder comes from **one signed log**.
- File contents are blobs, named by their hash. This part is the same as in
  IPFS.
- A folder can **mount** another folder at a path. This makes a tree.

The root of a person, a group, an album and an app are all folders.

### Three kinds of object

| Object    | What it does                                 | Examples                      |
| --------- | -------------------------------------------- | ----------------------------- |
| Directory | Maps names to objects                        | `/photos/2026/`               |
| File      | Points to a blob, and keeps all its versions | A photo, a document           |
| Stream    | A list of records that only gets longer      | A chat, changes to a password |

### Writes are entries in the log

```
put(path, hash) · delete(path) · append(path, data)
mount(path, folder-id) · policy(bytes)
```

Every entry is signed by its writer. The core does not give meaning to
`policy(bytes)`. The engine reads it (see below).

### Order

- The log of a folder names one **ordering node**. The ordering node can give
  this job to another node.
- Other writers send their writes to the ordering node. It puts them in order.
  This order is the **main line** of the folder.
- **Streams:** two appends never have a conflict.
- **Files:** the write that comes later in the order is the current version.
  The older version stays in the history, so nothing is lost.
- A node that is offline keeps its writes in a queue and sends them later.

The ordering rule is a policy. Another rule, for example a CRDT, can take its
place for a folder.

### Paths

```
/<key>/...                  the root of a person
/<key>/groups/family/       a shared folder, mounted
/<key>/apps/Photos.app/     an app, mounted
/<key>/appdata/<app-id>/    private data of an app; it syncs
/local/<app-id>/            cache of an app on this node only; it does not sync
/node/                      the settings and the engine of this node
```

## The authorization engine

### The core keeps facts; the engine makes rules

| The core (mechanism, not replaceable)           | The engine (policy, WASM, replaceable)           |
| ----------------------------------------------- | ------------------------------------------------ |
| Checks signatures: who wrote this entry         | Says if this person may do this thing            |
| Checks hashes: the content was not changed      | Gives meaning to `policy` entries                |
| Asks the engine, and does what it says          | Says who may read, which nodes may keep a folder |
| Gives keys to the readers that the engine names | Says who may use this node, and how much         |
| Keeps logs, syncs, counts resources             | Says what each app may use                       |

"Who" is a fact, so it is in the core. "May they" is a rule, so it is in the
engine.

### One engine for each node, with no limits

Each node runs the engine that its owner picked. The core puts **no limits**
on it. It may read the clock, use the network or call apps, if the owner lets
it. The person who controls a node is able to make it do anything, and the
system says so openly. It does not act as if it can stop them.

The engine is an app like any other: `/node/Engine.app/`. To change the rules,
change that folder.

### What no code can do

On your own node you can do anything. But three things are not possible for
any code. They come from mathematics, not from rules:

1. Nobody can make a signature with someone else's key.
2. Nobody can read encrypted data without the key.
3. Nobody can make someone else's node do anything.

So it is safe to keep other people's data on your node. Encryption keeps it
safe, not the system keeping you in control.

### A shared folder

**A shared folder is a folder that a number of nodes, each with its own
engine, agree to take writes into.** Nobody can force another node.

When nodes do not agree, their copies **fork**. An example:

- Alice, Bob and Carol share `/family/`. Alice made it. Later Alice takes Dave
  out of the group.
- The nodes of Alice and Bob use the default engine. They see that Dave was
  taken out, so they no longer take his writes.
- Carol put a different engine on her node: "Dave is my friend; I take all his
  writes."
- Dave puts a photo in the folder. Carol sees it. Alice and Bob do not.

There are two kinds of fork:

- **A filter.** One node takes an entry and another does not. This is like
  two people with different spam filters on the same mail thread. They are
  still in the same thread.
- **A different state.** Later entries depend on the entry that only some
  nodes took, or the nodes do not agree about members or the current version.
  Then the two copies go different ways, like a fork in git.

The ordering node keeps the main line. If its engine does not take Dave's
photo, the photo does not get into the main line. Carol may keep it on her
node, but only as her own side branch. If Carol's engine says no to an entry
on the main line, her copy stops there. She can wait, or start her own line.

To keep forks small:

1. Entries do not depend on each other when they do not need to. Chat
   messages and photos stand alone. Only changes to members and file versions
   depend on the order of the main line.
2. The user sees a fork: "Your copy is different from the main line in 3
   entries." They can follow the main line, keep their branch, or make it a
   new folder.
3. Most of the time there is no fork. When everyone uses the default engine,
   or engines that agree, every copy is the same.

**The system does not force agreement, but agreement is the default. A person
who changes the rules gets a copy that is different, and that was their
choice.**

### The default engine: capabilities

Most people will use the default engine. It works with capabilities:

```
capability = {
  issuer:   the key that signs it
  audience: the key that gets it (a person, device, node or app)
  resource: a folder ID and a path prefix
  ability:  read | write | append | grant | store | serve | use-node
  limits:   end time, quota, types of object
  proof:    the capability above it (a chain of signatures)
}
```

- The root of the chain for a folder is the key that made the folder.
- A capability can be given on to someone else, but only with less power,
  never more.
- To take a capability back, a `policy` entry goes into the log of the folder
  and syncs with it.

With capabilities, familiar ideas become simple:

| Idea                  | With the default engine                                           |
| --------------------- | ----------------------------------------------------------------- |
| Member of a group     | Has `read`, `write` or `append` on its folder                     |
| Owner of a group      | Has `grant`                                                       |
| A node keeps a folder | The key of the node has `store`                                   |
| A friend's backup     | Their node has `store` but not `read`, so it sees only ciphertext |
| Relay, mailbox        | `store` with an end time and a quota                              |
| Using someone's node  | The key of that node gives you `use-node`, with quotas            |
| What an app may do    | A smaller capability that you give to the app                     |

Other people may write other engines, for example:

| Engine          | Rule                                                          |
| --------------- | ------------------------------------------------------------- |
| Roles (RBAC)    | Owner, member, guest. Good for families and small teams       |
| Many signatures | k of n people sign a change. Good for a family password vault |
| Vote            | The members of a group vote to add or take out a person       |
| Legacy          | After a time with no activity, access goes to a named person  |

### Engine interface

```wit
interface authz {
  // The state of a new folder.
  init: func(folder: string, creator: key) -> list<u8>;
  // Read one policy entry and update the engine's own state.
  apply: func(state: list<u8>, author: key, policy: list<u8>) -> result<list<u8>, string>;
  // Say if an operation is allowed.
  check: func(state: list<u8>, principal: key, op: op) -> decision;
  // Questions that the core needs answered.
  readers: func(state: list<u8>) -> list<key>;   // for giving out keys
  storers: func(state: list<u8>) -> list<key>;   // for sync
  // The policy entry that gives `to` abilities on `path`.
  grant: func(to: key, abilities: list<ability>, path: string) -> list<u8>;
}
```

With `grant`, the core can give access to an app or a person without knowing
the format of policy entries. The full interface is in `wit/fos.wit`. Later,
the core will keep the answers in a cache: the same state, the same person and
the same operation are worked out one time only.

## Encryption

- Each folder has a content key. The readers of the folder share it through
  MLS (RFC 9420).
- A node that only has `store` never gets the key. It keeps only ciphertext.
- When a reader is taken out, the key is changed.

So "who can read" is not done by saying no to requests. It is done by **who
gets the key**. The engine gives the list of readers, and the core does the
encryption.

## Sync and the network: libp2p

We use [libp2p](https://libp2p.io) for the network and do not make our own.
It is tested and used widely, and it does what we need:

| What we need                       | What libp2p gives                                      |
| ---------------------------------- | ------------------------------------------------------ |
| Node identity                      | PeerId, made from the ed25519 key of the node          |
| Encrypted transport                | TCP with Noise and Yamux (QUIC later; see below)       |
| No public IP address and no domain | AutoNAT, DCUtR (hole punching), Circuit Relay v2, UPnP |
| Relays for friends (mutual aid)    | Circuit Relay v2; the engine says whom to help         |
| Finding nodes on the home network  | mDNS                                                   |
| Changes sent as they happen        | gossipsub, one topic for each folder                   |
| Our own sync protocol              | request-response and streams                           |
| Browsers and phones                | WebTransport, WebRTC, WebSocket                        |

We use libp2p, but not the IPFS parts of it:

- **No bitswap and no global content DHT.** A node gets blobs only from the
  nodes that keep that folder.
- **No DHT at all.** Nodes are found through rendezvous nodes that anyone can
  run, and through the nodes of people you know. See
  [Identity and finding nodes](../identity/).

We still make these parts ourselves, because they are fileOS: the sync
protocol for folder logs, the order of entries, and giving out keys with MLS.

How sync works:

- Nodes send each other the ends of their folder logs, and get the entries
  that they do not have.
- A node gets a blob by its hash from any node that keeps the folder.
- The engine of each node says which entries it takes.
- The sync protocol is sans-IO. It does not depend on libp2p. `fos-net`
  connects it to libp2p, and `fos-sim` connects it to an in-memory transport.

## The app runtime: wasmtime

We use [wasmtime](https://wasmtime.dev) and do not make our own:

| What we need                    | What wasmtime gives                           |
| ------------------------------- | --------------------------------------------- |
| Interfaces for apps and engines | The Component Model and WIT                   |
| Limits on resources             | Fuel or epoch interruption, `ResourceLimiter` |
| Many app instances              | The pooling allocator                         |
| Fast start                      | Compiled `.cwasm` kept in `/local/`           |
| Working with async I/O          | Async support                                 |
| Platforms with no JIT (iOS)     | The Pulley interpreter                        |

### The app interface (WIT)

Where a WASI 0.2 standard fits, we use it, so that the tools and libraries of
WASI work: `wasi:http`, `wasi:clocks`, `wasi:random`, `wasi:logging`. We do
**not** use `wasi:filesystem`, because it has POSIX meaning. We make our own
interfaces for the file system.

What the core gives to an app (imports):

| Interface                                    | What it does                                                       |
| -------------------------------------------- | ------------------------------------------------------------------ |
| `fos:fs`                                     | Read, write, append and list, only in the paths the app may use    |
| `fos:watch`                                  | Get told when a path changes                                       |
| `fos:cap`                                    | See who is calling; ask for or give on capabilities                |
| `fos:kv`                                     | An ordered key-value store on this node only, for indexes          |
| `wasi:http`                                  | Outgoing requests, only to hosts in the manifest that were allowed |
| `fos:crypto`                                 | Sign with the key of the app instance, never the user's key        |
| `wasi:clocks`, `wasi:random`, `wasi:logging` | The basics                                                         |

What an app gives to the core (exports):

| Interface                | When the core calls it                    |
| ------------------------ | ----------------------------------------- |
| `on-change(path, entry)` | A path that the app watches has changed   |
| `handle-http(request)`   | A request matches a route in the manifest |
| `run-job(name)`          | A timed job or a background job           |

### Rules for apps

- Apps run when an event comes. They have no threads. The core plans when
  they run.
- Every instance has limits on memory, fuel and time. The resources are taken
  from the `use-node` quota.
- **Apps work together only through the file system.** One app writes; another
  app watches. There is no other way for apps to talk.
- Our own apps and other people's apps use the same interface and the same
  sandbox. There is no special way in for our apps.

## App bundles

An app is a folder, as on macOS. Put it in place and it works. Delete it and it
is gone. There is no package manager, and there is no separate list of
"installed" apps. A bare `.wasm` file is not enough, so we learn from the
macOS `.app` bundle.

### How a macOS `.app` is made

```
Photos.app/
  Contents/
    Info.plist                  ID, version, main program, file types, URL schemes,
                                reasons shown when asking for access
    MacOS/Photos                the main program (can have code for more than one CPU)
    Resources/                  icons, interface files, en.lproj/ zh-Hans.lproj/ …
    Frameworks/                 libraries that come with the app
    PlugIns/                    extensions (.appex); each is a bundle of its own
    XPCServices/                helper programs, with less power than the main one
    _CodeSignature/CodeResources  the hash of every file in the bundle (the seal)
```

Around it, macOS has: entitlements (what the app may do, signed into the
program), notarization (Apple checks the app), Gatekeeper (checks before an app
opens), Launch Services (a list of which app opens which type of file), and
containers (`~/Library/Containers/<id>/` for the private data of an app).

### A fileOS `.app`

```
Photos.app/
  manifest.toml              like Info.plist and entitlements
  bin/
    app.wasm                 the main component
    thumbnailer.wasm         a helper component, with less power (like XPCServices)
  lib/
    exif.wasm                a shared component (like Frameworks)
  extensions/
    Thumbnail.ext/           an extension (like .appex): thumbnails for image/*
      manifest.toml
      ext.wasm
    SearchExtractor.ext/     like a Spotlight importer: text for search from photos
  ui/                        web interface
  resources/
    icon.svg
    locales/en-us.toml
    locales/zh-cn.toml
```

There is no `_CodeSignature/`. The app is a folder, and a folder is a signed
log that names every file by its hash. The publisher signs the end of the log.
That is the seal, and we get it for nothing.

### The manifest

```toml
id       = "photos"           # with the publisher's key, a unique ID; no domain needed
name     = "Photos"
version  = "1.4.0"            # the version people see
build    = 142                # the build number
requires = "fos:app@1"        # the WIT version it needs
main     = "bin/app.wasm"

[[permissions]]               # like entitlements and usage descriptions
path   = "/photos/"
access = "read-write"
reason = "reason.photos"      # the text is in locales/, in both languages

[[permissions]]
path   = "/contacts/"
access = "read"
reason = "reason.faces"

[[opens]]                     # like CFBundleDocumentTypes
types = ["image/jpeg", "image/png", "image/heic"]
role  = "editor"

[[types]]                     # like UTExportedTypeDeclarations
id   = "photos.album"
from = "application/cbor"

[[routes]]                    # like CFBundleURLTypes, but for HTTP
prefix = "/photos"

[[watch]]
path = "/photos/"

[[jobs]]
name  = "rebuild-index"
every = "1d"

[[helpers]]                   # helpers have their own, smaller permissions
main        = "bin/thumbnailer.wasm"
permissions = [{ path = "/photos/", access = "read" }]
```

People write the manifest, so it is TOML. The core reads it and keeps it as
CBOR.

### What we take from macOS, and what we change

| macOS                               | fileOS                                                                                                                                                             | Why                                                                           |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| Code for more than one CPU          | **Not needed.** WASM runs everywhere. Each node keeps compiled `.cwasm` in `/local/`                                                                               | A strong point of WASM                                                        |
| Reverse-DNS bundle ID               | **Publisher key and a name**                                                                                                                                       | No domain is needed                                                           |
| Notarization by Apple               | **No center.** The engine may ask for signatures from reviewers the user trusts, or not                                                                            | The user makes the rules                                                      |
| Gatekeeper                          | **The engine**                                                                                                                                                     | The same reason                                                               |
| Launch Services                     | The core looks at `/apps/` and keeps an index in `/local/`. It can be made again any time                                                                          | Derived data is not state                                                     |
| `~/Library/Containers/<id>/`        | `appdata/<id>/` (syncs) and `/local/<id>/` (cache). Both go when the app goes                                                                                      | Cleaner than macOS                                                            |
| User files are outside the app      | **The same.** Photos are in `/photos/`, not in the app                                                                                                             | Data lives longer than apps                                                   |
| `.appex` extensions                 | **The same, and more important.** Thumbnails, search text, previews, sharing and MCP tools are extensions for a file type                                          | A new file type needs one extension, and every app can then use it            |
| Asking the first time (TCC)         | Asking the first time a path is used. The engine keeps the answer                                                                                                  |                                                                               |
| Open panel gives access to one file | The same: opening a file with an app gives that app the path for a time                                                                                            | Access by choosing                                                            |
| `.dmg` or `.zip`                    | Share the folder. To move it as one file, `fos app pack` makes an `.app` file: a zip with no compression, the manifest as it is, and the rest in `payload.tar.zst` | The box for moving is not the app; the metadata can be read with no unpacking |

The most important thing to take is **extensions**. Spotlight and Quick Look
work with any file type, because each type has a small extension. Here the
search app does not need to know every format. It calls the `SearchExtractor`
for that type. A file browser does not need to know photos. It calls the
`Thumbnail` extension. Each extension has its own sandbox and less power, so
other people can write them safely.

The runtime connects the main component, the shared components and the
extensions **when they run**, through WIT. It does not join them into one file
at build time with `wac`. So each part can be updated and changed on its own.

### The life of an app is file operations

| Action                      | What happens                                                   |
| --------------------------- | -------------------------------------------------------------- |
| Install                     | Copy or mount the app folder into `/apps/`                     |
| Run                         | The core sees a `.wasm`; the engine says yes; wasmtime runs it |
| Update                      | Sync the folder from the publisher, or change the `.wasm`      |
| Give it to a friend         | Share the folder                                               |
| Have it on all your devices | `/apps/` is in your root, so it syncs to all your nodes        |
| Uninstall                   | Delete the folder; the core also deletes `/local/<id>/`        |

- Access goes with the **ID of the app folder**, not the hash of the `.wasm`.
  After an update, the user does not need to give access again. If the new
  manifest asks for more, the engine says if the user is asked.
- Only the publisher key that goes with the `id` can sign the app. A bundle
  that someone else changed has a different signature, so it is a different
  app. The engine says if it may run.

## The gateway

The gateway is one HTTP port for clients. Sync between nodes does not use it;
it goes over libp2p. The gateway does two things only:

1. A small file system API, so clients can read and write paths.
2. **Every other request goes to the `handle-http` of an app**, by the routes
   in the manifests.

The gateway checks who is calling: a device key signature or a capability
token. Then it gives the caller to the app. Apps never handle keys. In the
prototype, the gateway listens on `127.0.0.1` only, and checks one token from
the data directory.

## Everyday features are apps

| App        | Paths it uses                                   | What it gives                               |
| ---------- | ----------------------------------------------- | ------------------------------------------- |
| **photos** | Watches `/photos/`, writes `/local/photos/`     | EXIF, thumbnails, albums                    |
| **search** | Watches every path it may use, writes `/local/` | Full-text search, timeline, places, links   |
| **people** | Reads chats, photos and calendars               | Pages for people, days and places           |
| **chat**   | One stream for each conversation                | Chat                                        |
| **vault**  | Streams under `/vault/`                         | The Bitwarden API                           |
| **dav**    | `/contacts/`, `/calendar/`, `/files/`           | CardDAV, CalDAV, WebDAV                     |
| **login**  | Gives capabilities (an app with much power)     | Passkeys, OpenID Connect, forward auth      |
| **mcp**    | The paths that the user gives to AI             | MCP                                         |
| **s3**     | One folder                                      | An S3-compatible endpoint                   |
| **push**   | Watches paths                                   | Web Push                                    |
| **import** | Writes the right paths                          | Google Takeout, iCloud, chat exports        |
| **backup** | Only `store` capabilities                       | Backups to a directory, S3, a friend's node |

These apps will come with fileOS (the photos app is built now). They use the same interface and sandbox as
everyone else's apps.

## Examples

**A new photo.** A client does `put /photos/2026/a.jpg`. The core writes the
log and keeps the blob. The photos app gets `on-change`, makes a thumbnail and
writes it to `/local/photos/`. The search app gets `on-change` and updates its
index.

**A family group.** Make a folder and mount it at `/groups/family/`. Give each
family member `write`. Their nodes start to sync the folder. The chat app makes
a stream in it. That stream is the group chat.

**A friend keeps your backup.** Give your friend's node `store`, with a quota.
It syncs your folders, but it gets only ciphertext. When your node breaks, you
make your identity again with the recovery code and get the data back from
your friend.

**Someone uses your node.** The key of your node gives them `use-node`
(50 GB). They put their folders on your node. For you, those folders are only
ciphertext.

## Crate layout

```
crates/
  fos            the program; only joins the parts together
  fos-node       puts the core together into one node
  fos-gateway    the gateway
  fos-runtime    the app runtime (wasmtime): .app files and folders, engines, apps
  fos-net        libp2p
  fos-store      disk: the node key, blobs, logs, local key-value stores
  fos-sync       the sync protocol (sans-IO, portable)
  fos-fs         folders, logs, order, forks, mounts (sans-IO, portable)
  fos-id         identities: did:fos, node certificates, identity links (portable)
  fos-core       formats: CBOR, ed25519, BLAKE3 (portable, at the bottom)
  fos-sim        deterministic simulation of many nodes; tests the core only
apps/            a second workspace, built for wasm32-unknown-unknown
  fos-app        the Rust SDK for apps and engines (MIT OR Apache-2.0)
  capability     the default engine
  photos         the photos app
  chat           the chat app
wit/
  fos.wit        the interfaces (MIT OR Apache-2.0)
```

Dependencies still go down only. Apps and engines depend only on `fos-app`.
Only `fos-runtime` uses wasmtime, and only `fos-net` uses libp2p, so the rest
of the code still builds fast.

## What the design gives

| Principle             | How this design does it                                                                                        |
| --------------------- | -------------------------------------------------------------------------------------------------------------- |
| Yours alone           | Your data is your folders, your identity is your key, your rules are your engine                               |
| Centered on people    | The base has no data types. Apps such as people put things in order by person, across all folders              |
| Zero operations       | The core is small and does not change much. When a feature has a problem, change one app                       |
| Long-lived            | Only two formats, logs and blobs. When an app is gone, the data is still there                                 |
| In control            | Every access is a record in the engine. You can see it and take it back                                        |
| Memory you own        | The memory of an AI is a folder too. You can see it, change it and delete it                                   |
| Person to person      | No server at the center, only nodes and the relations between them                                             |
| Personal tools        | A small tool made from a sentence is an app, and it runs in the sandbox                                        |
| Open and replaceable  | Even the rules for access can be changed. Nothing is locked                                                    |
| Mutual aid            | Keeping, relaying and backups are all "my node agrees to keep this for you"                                    |
| Mechanism, not policy | The core is only mechanism. The default engine and the apps that come with it are defaults that can be changed |

It also answers why federation alone is not enough. Identity is not fixed to a domain, because identity is a key. Cost
does not get bigger with the network, because a node syncs only its own
folders. No admin is needed, because each person's engine is their own rules.

## What the prototype has

| Part                                                          | Built                                        |
| ------------------------------------------------------------- | -------------------------------------------- |
| Folders: signed log, files, streams, mounts, order, forks     | Yes (`fos-fs`)                               |
| Sync, with writes that wait for the ordering node             | Yes (`fos-sync`, tested in `fos-sim`)        |
| Engine as a WASM component, with no limits                    | Yes; the default engine is `apps/capability` |
| Apps as WASM components, with their own keys and limits       | Yes; the photos app is `apps/photos`         |
| `.app` folders that sync like any folder                      | Yes                                          |
| libp2p: TCP, Noise, Yamux, mDNS, relays, `DCUtR`, `AutoNAT`   | Yes; tested between two nodes on one machine |
| The `fos` command and the gateway                             | Yes, for local clients only                  |
| QUIC                                                          | No, see below                                |
| Encryption of folders with MLS                                | No                                           |
| `handle-http` for apps, extensions, helpers, web interfaces   | No                                           |
| Signatures of app publishers; asking the user the first time  | No                                           |
| A cache of the engine's answers; compiled `.cwasm` in a cache | No                                           |
| Blobs sent in parts                                           | No: a blob is one message, up to 64 MiB      |

## Costs and open questions

1. **Compile time.** wasmtime's compiler (cranelift) is most of a clean
   build. In debug builds, cranelift is built with no optimization, which
   makes it about a third faster to build. On the CI machine (4 cores), a
   clean build is 166 s (budget 360 s), a rebuild after a change to `fos` is
   1.0 s (budget 5 s), and after a change to `fos-core` is 4.2 s (budget
   20 s).
2. **QUIC.** `libp2p-quic` always turns on the `aws-lc-rs` backend of quinn,
   and aws-lc-sys is not allowed (`deny.toml`). So the prototype uses TCP.
   Relays and hole punching work over TCP too. We can add QUIC when libp2p
   lets us use `ring`, or if we change the rule.
3. **One version of every crate.** libp2p and wasmtime bring some crates in
   two versions (for example `sha2`, `curve25519-dalek` and `getrandom`).
   Each one is in `deny.toml` with its reason. None of them comes from our
   own code.
4. **Speed of WASM.** Thumbnails and full-text search may be 1.5 to 2 times
   slower than native code. We need measurements on home hardware.
5. **The engine runs for every write**, and for every entry in sync. The cache
   needs measurements.
6. **Forks that people can understand.** A person who is not technical has to
   understand "your copy is different from the main line".
7. **The login app** gives capabilities, so it has nearly as much power as the
   core. The way the user agrees to it needs its own design.
8. **The name.** The repository, the docs and the domain change if the name
   changes. We should check that the name is free first.

## Next steps

1. Encryption of folders with MLS, so that nodes with only `store` see only
   ciphertext.
2. `handle-http` for apps, and the first apps that people use every day:
   files over WebDAV, a vault for Bitwarden clients, chat.
3. Measurements of WASM speed and of the engine on home hardware.
