---
title: "Architecture: fileOS"
description: The kernel is a file system that syncs between nodes, with no ordering node, and with access in grants. Every feature is a WebAssembly app.
---

:::note[Status: built in part]
The kernel on this page is built: spaces, files, grants, sync with no
ordering node, checkpoints, and the node, store, network, runtime and
gateway on top of it. [What is built](#what-is-built) says what is not yet.
The name "fileOS" is a working name.
:::

## In one sentence

**The kernel keeps only facts: data, signatures, grants and transport. Every
feature is WebAssembly code that the user picks, as an app.**

## Why one base

When a system has many separate ideas (groups, items, grants, tokens,
replicas, relays, plugins), each new feature brings one more idea. Each idea
needs its own code, its own tests and its own rules for sync.

fileOS has one base, and everything is made on top of it. It took its first
idea from IPFS: content is named by its hash. But it is much smaller than
IPFS, because it is made for the computers of people and families, not for a
global network:

- No global DHT for content. Nodes find each other through people.
- Nothing is public by default.
- A node does not keep anything that it did not agree to keep.

## The model

The model has seven ideas: **content, files, spaces, principals, grants,
views and events**. Each is on [The model](../access/), with what is made one
and what is kept apart, and why. In short:

| What you see               | What it is                                                    |
| -------------------------- | ------------------------------------------------------------- |
| A shared album or drive    | A space, with grants for the family                           |
| A chat                     | A directory with one small file for each message              |
| A group of people          | A group: a principal with members, owned by a key or a group  |
| A person with many devices | An identity (`did:fos`) and its nodes                         |
| A backup or a relay        | A node with `data.keep` on a space                            |
| Installing an app          | Putting the app into a space mounted at `/apps/<name>`        |
| What an app may do         | Grants to the app's key, from the permissions in its manifest |
| Taking access back         | A take-back; what was given on from it goes too               |
| Moving to a new server     | Letting a new node keep your spaces                           |

## Layers

```
┌───────────────────────────────────────────────┐
│ Apps (WASM)  photos chat …                    │
├───────────────────────────────────────────────┤
│ WIT: fos:app (fs, kv, log, events) + wasi:http│
├───────────────────────────────────────────────┤
│ Kernel: spaces · grants · sync · checkpoints ·│
│         app runtime · gateway                 │
├───────────────────────────────────────────────┤
│ wasmtime · libp2p · disk                      │
└───────────────────────────────────────────────┘
```

The kernel has no "should" in it about data. It does not know what a photo,
a chat or a password is. Where it must decide (when to make a checkpoint,
which old versions to keep), it has a policy with a default that can be
changed.

## The kernel

### Spaces and their logs

A **space** is the unit of sync, owners, keys, checkpoints and retention.
Its ID is the hash of its first entry, so it proves itself.

- Every writer (a node or an app, by its key) has its own **signed log** in a
  space. Each entry names the entries it comes after, so the logs make one
  **causal graph**. There is no ordering node: every node writes at any
  time, also offline, and every node with the same entries gets the same
  state, in any order.
- Inside, a space is a **flat set of names**, as S3 is. A "directory" is only
  a prefix. Names never move by themselves, so access by prefix never changes
  without someone asking.
- A **file** has a fixed ID: the place of the act that made it. A rename is a
  new version, and keeps the ID. When two devices change a file at the same
  time, both versions are kept, side by side.
- **Content** is cut into chunks at points that the content picks, and each
  chunk is named by its hash. A new version keeps the chunks that did not
  change, and sync sends only what is missing.

Details: [Data](../data/).

### Grants

Access is a **grant**: allow or deny, exact permissions (`files.read`,
`access.give`, `data.keep`, …), on a prefix of names or on one file by its
ID, to a key or a group. A grant is given on only the same or smaller.
Taking one back takes back what was given on from it. A deny wins.

With no ordering node, a take-back and a write can happen at the same time.
The kernel uses **strong removal**: what someone did at the same time as their
access was taken back has no effect. Every node gets the same answer from the
causal graph alone, with no clock.

A node may keep a space only when its key has `data.keep` or `files.read` on
the whole space, so a space never goes to a node that was not given it.

Details: [The model](../access/).

### Checkpoints

The owner of a space, or a principal with `space.checkpoint`, signs a
**checkpoint**: the hash of the state in its past. Then:

- at an owner-signed checkpoint, a node can **compact**. It keeps the state
  at that point as a base and drops the entries before it;
- a new node can start from an owner-signed **snapshot**. It needs the log
  to check the right to sign a checkpoint made with a grant;
- old versions get **thinner with age**, as the retention in the checkpoint
  says, and deleted files go after a time;
- changes made offline at the same time as a checkpoint are **written again
  on top** by the author's node, and the person is told.

A node makes a checkpoint of a space it owns after 1000 entries (a policy).

### Paths on a node

A node has a **home space**. Other spaces are mounted in it at names, so
`/photos/2026/a.jpg` is the name `2026/a.jpg` in the space mounted at
`photos`. A mount is a small file in the home space, under `.mounts/`.

```
/photos/...            a space, mounted
/apps/Photos.app/...   the space of an app
/identity/...          the identity space: the DID log and node certificates
```

## Sync and the network: libp2p

We use [libp2p](https://libp2p.io) for the network and do not make our own.

| What we need                       | What libp2p gives                                |
| ---------------------------------- | ------------------------------------------------ |
| Node identity                      | PeerId, made from the ed25519 key of the node    |
| Encrypted transport                | TCP with Noise and Yamux                         |
| No public IP address and no domain | AutoNAT, DCUtR (hole punching), Circuit Relay v2 |
| Finding nodes on the home network  | mDNS                                             |
| Our own sync protocol              | request-response                                 |

We do not use the IPFS parts: no bitswap and no DHT. Nodes are found through
rendezvous nodes and the nodes of people you know (see
[Identity and finding nodes](../identity/)).

How sync works:

- Two nodes say how long each log of each space is, and send each other the
  entries that the other does not have, only for spaces the other may keep.
- New entries go to every connected node that may keep the space, and are
  sent on from there.
- A node that is far behind can start from the last owner-signed snapshot.
- A node asks for content by hash. If the request fails or takes too long,
  the node can ask again.
- The sync protocol is sans-IO. `fos-net` connects it to libp2p, and
  `fos-sim` to a simulation in memory.

## The app runtime: wasmtime

We use [wasmtime](https://wasmtime.dev) and do not make our own. An app is a
component of the `app` world in `wit/fos.wit`:

| Interface                | What it does                                                   |
| ------------------------ | -------------------------------------------------------------- |
| `fos:app/fs`             | Read, write, add to and list paths, checked against the grants |
| `fos:app/kv`             | A cache on this node only, for indexes that can be made again  |
| `fos:app/log`            | Messages for people                                            |
| `wasi:clocks/wall-clock` | The time, for names that sort by time                          |
| `fos:app/events` (out)   | `on-change`: a path that the app watches changed               |
| `wasi:http` (out)        | The requests for the app's own origin, `<app-id>.localhost`    |

Rules for apps:

- Every instance has limits on memory and fuel.
- **Apps work together only through files.** One app writes; another app
  watches. There is no other way for apps to talk.
- Our own apps and other people's apps use the same interface and the same
  sandbox.

The next steps for apps (workers, `wasi:filesystem`, watch options, the
picker) are in [Apps as workers](../workers/) and [The model](../access/).

## App bundles

An app is a folder, as on macOS: `manifest.toml`, components in `bin/`, the
web interface in `ui/`, and `resources/`. To move it as one file,
`fos app pack` makes an `.app` file: a zip with no compression, the manifest
as it is, and the rest in `payload.tar.zst`.

```toml
id       = "photos"
name     = "Photos"
kind     = "app"
version  = "1.4.0"
build    = 142
requires = "fos:app@0.1.0"
main     = "bin/app.wasm"

[[permissions]]               # the most the app may get
path   = "/photos"
access = "read-write"         # a template: read, read-write, manage
reason = "reason.photos"      # the text is in resources/locales/

[[watch]]
path = "/photos"
```

When an app is installed, it goes into a space mounted at `/apps/<name>`,
and its key gets the grants that its manifest asks for. Its key comes from
the node key and the ID in the manifest, so an update does not need access
again.

## The gateway

The gateway is one HTTP port on this machine. Sync between nodes does not use
it. It gives the kernel API at `fos.localhost`, and each app's requests at
its own origin, `<app-id>.localhost`. Apps get no cookies and never handle
keys. See [HTTP and the browser](../http/).

## Everyday features are apps

| App        | What it uses                                   | What it gives                             |
| ---------- | ---------------------------------------------- | ----------------------------------------- |
| **photos** | Watches `/photos`, keeps an index in its cache | Sizes of photos, a gallery, upload        |
| **chat**   | One file for each message, in `/chat/<room>/`  | Chat                                      |
| **search** | Watches what it may read                       | Full-text search                          |
| **dav**    | `/contacts`, `/calendar`, `/files`             | CardDAV, CalDAV, WebDAV                   |
| **vault**  | Files under `/vault`                           | The Bitwarden API                         |
| **backup** | Only `data.keep`                               | Backups to a directory or a friend's node |

Photos and chat are built now. The others will come. They use the same
interface and sandbox as everyone else's apps.

## Crate layout

```
crates/
  fos            the program; only joins the parts together
  fos-node       puts the kernel together into one node
  fos-gateway    the gateway
  fos-runtime    the app runtime (wasmtime): .app files and apps
  fos-net        libp2p
  fos-store      disk: the node key, logs and bases of spaces, content
  fos-space      spaces: logs, files, grants, checkpoints, sync (sans-IO, portable)
  fos-id         identities: did:fos, node certificates, links (portable)
  fos-core       hashes, keys, signatures, CBOR (portable)
  fos-sim        a simulation of many nodes, for tests
apps/
  fos-app        the Rust SDK for apps (MIT OR Apache-2.0)
  photos, chat   the apps that come with fileOS
wit/fos.wit      the app interface
```

Dependencies go down only. Apps depend only on `fos-app`.

## What is built

| Part                                                              | Built                                  |
| ----------------------------------------------------------------- | -------------------------------------- |
| Signed logs in a causal graph, with no ordering node              | Yes (`fos-space`, tested in `fos-sim`) |
| Files with fixed IDs, versions side by side, content in chunks    | Yes                                    |
| Grants: permissions, prefixes and files, give on, take back, deny | Yes                                    |
| Strong removal                                                    | Yes                                    |
| Groups, owned by keys or by groups                                | Yes                                    |
| Checkpoints, compaction, snapshots, retention                     | Yes                                    |
| The node, store, network and gateway on the new kernel            | Yes                                    |
| Identities: `did:fos`, node certificates, an identity space       | Yes                                    |
| Apps over `wasi:http`, with the photos and chat apps              | Yes                                    |
| Encryption: keys that follow prefixes, HPKE, crypto-shredding     | No                                     |
| People as principals (an identity acts for its devices)           | Yes                                    |
| Views for apps, the picker, sharing between apps                  | No                                     |
| Apps as workers, watch options, `wasi:filesystem`, schedules      | No                                     |
| Sign-in for people (OAuth with PKCE and DPoP)                     | No                                     |

## Costs and open questions

1. **The state is worked out again** from all entries after each change. It
   is simple and right, and slow for large spaces; checkpoints keep spaces
   small, and the work can be made step by step later.
2. **Data is not encrypted at rest yet.** A node with `data.keep` sees the
   data. Keys that follow prefixes come next (see [Data](../data/)).
3. **Reading part of a space** (one prefix or one file) is not yet sent to
   other nodes: a node keeps a whole space or none.
4. Speed of WASM and of the kernel on home hardware is not measured yet.

## Next steps

1. Encryption: keys that follow prefixes, HPKE, crypto-shredding.
2. Views for apps, the picker, sharing between apps.
3. Apps as workers: a new instance for each event, watch options, schedules.
4. Sign-in for people at the gateway.
