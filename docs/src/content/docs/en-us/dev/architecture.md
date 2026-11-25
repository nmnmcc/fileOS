---
title: Architecture and technology choices
description: How homeserver is built, the technology we chose, and the reasons.
---

This page records the general design of homeserver and the technology under
it. Each decision gives the reason for it. Where it matters, it also says what
would make us look at the decision again. Details will change as we build.
The direction should not change without a discussion first.

We took the measurements on this page on 2026-11-25, with Rust 1.100, on an
Apple Silicon machine with 15 cores, unless the text says something different.
"Crates" counts the normal (non-dev) dependency graph. Crate counts give
context; they do not make the decision. A library that is split into small
crates builds in parallel and rebuilds less, so it can cost less than one big
crate. We judge a dependency by its clean and incremental build time, its
binary size, and what it brings with it. Bad things to bring are: a second
crypto library or HTTP stack, banned crates such as `async-trait`, heavy
procedural macros, build scripts, or C and C++ code.

## Where the architecture comes from

A small number of lines in the vision decide most of the architecture:

- One CPU core and 1 GB of memory serve a family of ten. When the server is
  not busy, it uses less than 150 MB of memory.
- One program and one data directory. A copy of the directory is a backup.
- All the data can still be read in twenty years.
- You always know who can see what, and you can take it back.
- A family member who is not technical can get out of a problem without help.
- A server for one person is first class. Its cost grows with your own life,
  not with the size of the network.

The Web gives the ideas that answer these needs:

- resources have URLs and more than one representation;
- things that never change can be cached by anyone, and the go-betweens that
  hold them need no trust;
- clients are user agents, and they work offline through caches;
- links are relations;
- the rule of least power: use the simplest tool that does the job.

## Rules that shape everything

1. **Built on the Web.** Everything has a URL, and everything talks HTTP.
   Every interface uses an open standard when one exists. This is true between
   homeservers, and also at the edges.
2. **One executable, one data directory.** `hs` is a single file, and
   everything is linked into it. Nothing is downloaded while it runs. No other
   program is needed, for storage or for anything else. When this rule and
   another rule do not agree, this one wins.
3. **The log is the truth; everything else is derived.** What a person keeps
   is recorded in signed, append-only logs and content-addressed blobs.
   Tables, search indexes and previews are made from them. They can be thrown
   away and made again at any time.
4. **Immutable things can live anywhere.** Sealed log segments and blobs never
   change, so they are HTTP resources that can be cached. A local directory,
   another `hs` or an S3 bucket can keep them. Sync, backup, moving to a new
   place and federation are one operation: get what you do not have.
5. **One protocol everywhere.** Every node talks the same HTTP API and uses
   the same formats. Browsers and phones are user agents of their own
   homeserver. They keep a cache, keep writes in a queue while offline, and
   keep device keys. They do not each run a copy of the server.
6. **Your nodes are your trust boundary.** Plaintext is only on nodes that are
   yours. Anything that goes to someone else's machine is encrypted first. Such
   machines are a cloud bucket, a relay, a mailbox, or the server of a friend
   who is not a member.
7. **One permission model.** Devices, apps, plugins, AI assistants and friends
   all get to your data through the same HTTP API. They use grants, and you can
   list all grants in one place and take them back (revoke them) in one place.
8. **Order, don't agree.** Each shared thing has one home server, and that
   server puts its events in order. There is no global replicated state, and
   there is no consensus across the network.
9. **Sans-IO cores.** Protocol logic is written as state machines. Time,
   randomness, storage and the network are given to them from outside
   (injected). The same code runs in production and in a deterministic
   simulation.
10. **Mechanism in the core, policy outside it.** Some questions are policy:
    whom to relay for, where to keep which data, how long to keep history,
    what to take from strangers. Each policy has a default. The person or a
    plugin can put a different one in its place.
11. **Compile time is a budget.** We measure every dependency by build time,
    binary size and what it brings with it. We do not measure it by the number
    of crates it is split into. We write small things ourselves when a library
    would bring a parallel stack. No C or C++ that needs cmake. Small C code
    that the `cc` crate compiles, such as SQLite, is allowed.

## Overview

```
 Web app · phone shells · DAV, Bitwarden, OIDC and MCP clients · plugins
        │ HTTP: one API, one permission model (grants)
        ▼
┌──────────────────────────── hs ──────────────────────────────────────┐
│ HTTP API     resources and URLs · WebDAV · CalDAV/CardDAV ·           │
│              Bitwarden · OIDC provider · MCP · Web Push               │
│ Life         people, time, places, things · timeline · search ·       │
│              memory                                                   │
│ Derived      SQLite: current resources, links, FTS5, R*Tree           │
│              (rebuilt from the logs whenever needed)                  │
│ Truth        signed space logs · content-addressed blobs · grants     │
│ Spaces       ordering · membership · MLS                              │
│ Identity     person keys · node keys · passkeys · identity log (DID)  │
│ Replicas     local directory · another hs · S3 · encryption           │
│ Plugins      WebAssembly components (wasi:http) · connected apps      │
└───────────────────────────────────────────────────────────────────────┘
        │ HTTP/3 over QUIC (TCP fallback), signed requests
        ▼
 Your other nodes · friends' homeservers · relays · replicas
```

## Built on the Web

The Web is the largest decentralized system that works. Anyone can publish,
anyone can make a link, and any client can read. homeserver takes its ideas
and its standards. It does not make parallel ones.

**Everything has a URL.** Every object, space, log, person and blob is an HTTP
resource. Paths start with the person's identity ID, not with a server name.
So links still work when the person moves to a new server. The identity log
says which nodes serve that identity now. A domain is only an alias.

**HTTP semantics, used fully.** Methods mean what HTTP says they mean. Blobs
and sealed log segments never change. So their ETag is their hash, and they are
served with `Cache-Control: immutable` and `Range`. Updates use conditional
requests (`If-Match`). A write based on old data gets `412`; it does not
silently overwrite newer data. One resource has more than one representation,
and content negotiation picks one. A contact is JSON, vCard or a page. An event
is JSON or iCalendar. A photo is the original or a preview. Relations are links
(`Link` headers, RFC 8288). Clients follow links; they do not build URLs
themselves.

**Clients are user agents.** The web app first renders as HTML, and then
standard Web Components add to it. Offline use works the way the Web already
works. A Service Worker keeps immutable resources in the Cache API. Writes
made offline go into a queue, and they are sent when the network comes back.
Each write has an idempotency key, so a retry never takes effect twice. The
server stays the one place where the data lives and where queries run.

**HTTP between homeservers too.** Homeservers talk HTTP/3 over our QUIC stack.
When that does not work, they use HTTP/2 over TLS on TCP port 443. A space's
log is a resource that is read in ranges. New events are sent with POST to an
inbox. The node key signs requests with HTTP Message Signatures (RFC 9421). So
a signature is still good when a request goes through relays. The only things
we define ourselves are media types and the meaning of the resources. Adding
`h3` and `h3-quinn` costs 9 crates.

**Web standards for sign-in, push and apps.**

- Sign-in: passkeys (WebAuthn), OAuth 2.1 and OpenID Connect. DPoP (RFC 9449)
  binds tokens to a device.
- Notifications: Web Push (RFC 8030, 8291 and 8292), with `hs` as the
  application server. It works for installed web apps on Android, on iOS 16.4
  and later, and on desktops. UnifiedPush uses the same protocol.
- Plugins: WebAssembly, a W3C standard. The plugin interface is built on
  `wasi:http`. So a plugin is a handler that gets requests and gives back
  responses, much like a Service Worker.
- Plugin UIs run in their own origin with a strict Content Security Policy.
  So the browser's same-origin model keeps them apart.
- Replicas use a subset of the S3 API. That subset is plain HTTP with
  conditional writes.

**Formats.** HTTP APIs use JSON. Where a vocabulary exists, we use it again
(ActivityStreams 2.0, schema.org, vCard, iCalendar); we do not give things new
names. Log events are deterministic CBOR (RFC 8949, section 4.2), signed with
COSE (RFC 9052). WebAuthn uses the same pair. Export writes each type in its
common open format.

## Data model

**Resources.** Everything a person keeps is a resource: a photo, a file, a
message, a note, a contact, an event, a password. A resource has a type, a
schema version, fields, and references to blobs.

**Entities and links.** People, places, times and things are resources too.
Other resources link to them. "Open a person and see everything with them" is
a query over links. It is not a feature that each app makes again. This is
where principle 02 lives in the code.

**Spaces.** A space is the unit of sharing and sync. Examples are your private
space, your family, a chat, a shared album and a shared folder. A space has
members (keys with roles), one home server, and one log.

**Logs.** A space's log is an append-only list of signed events. An event is
an operation on a resource, in HTTP's terms: create, replace, patch or delete.
It has the new representation or a reference to it. The home server gives each
event a sequence number and chains it to the one before by hash. Logs are
stored in segments. When a segment is full, it is sealed and named by its hash,
and it never changes again. Grants, membership changes and MLS commits are
events too. So they are in order and can be checked (audited) like everything
else.

Why an ordered log, and not CRDTs everywhere? There are three reasons.
Permission changes need a total order in any case; Matrix's state resolution
shows the cost of not having one. An ordered log is cheap for small servers.
And MLS needs an ordering service for its commits, and the home server already
gives one. We use CRDTs only inside resources that people edit together, such
as shared documents. There, the CRDT data goes inside event payloads. Edits
made offline wait in a queue on the client, and they are put in order when
they get to the home server.

A home server may go away for ever. Then the members can move the space to
another home server with a signed handover. This works the same way as when a
person moves their identity.

**Blobs.** File contents are stored by their BLAKE3 hash. The same content is
stored only once. Transfers are checked while they stream, in the style of
Bao, so a node that is not trusted can serve a blob.

**Forgetting.** An append-only log must still let people truly delete things.
The hash chain covers the hash of each event's payload, not the payload
itself. So a payload can be erased later, and the chain can still be checked.
A delete event erases the payloads and blobs that it makes unreachable. It does
this on every node that follows the log. How long older versions are kept is
the person's policy: for ever, a year, or not at all.

**Checkpoints.** Replaying years of events on every start would be slow. So the
home server writes signed checkpoints. A checkpoint is the current state of a
space at one sequence number, stored as an immutable object next to the
segments. A node builds its tables from the latest checkpoint and the events
after it.

**Twenty years.** The docs give the log and checkpoint formats, and we keep the
formats small. So a person can read their data with the spec and a CBOR
library, without `hs`. Export changes the same data into vCard, iCalendar,
Markdown, JSON and the original files.

## Storage

### The data directory

```
data/
  logs/       one directory per space: sealed segments, the open tail, checkpoints
  blobs/      file contents by BLAKE3 hash
  keys/       node and storage keys (the person key is not kept here)
  index.db    SQLite: derived, safe to delete
```

A copy of `logs/`, `blobs/` and `keys/` is a complete backup. `hs` makes
`index.db` again when it is missing, damaged, or from an older version of
`hs`.

### Replicas

**`hs` is all you need for storage.** A replica keeps copies of segments,
checkpoints and blobs. It is one of these:

- A local directory, such as an external disk. `hs` writes the files itself:
  it writes to a temporary file, runs `fsync`, then renames the file.
- Another `hs`: a box at home, a VPS you rent, or a friend's homeserver. It
  serves segments and blobs as normal HTTP resources. It takes writes that
  your node key signed (RFC 9421). A rented VPS or a friend's homeserver is
  normally a blind node: it only ever keeps ciphertext.
- An S3-compatible bucket, if you want one: a cloud provider, or Garage or
  MinIO if you already run them.

Sealed segments and blobs never change. So every replica operation is the
same: list what the other side has, and copy what it does not have.

- Sync and backup: keep a replica up to date.
- Move: sync, make the replica primary, and then delete the old copy if you
  want.
- Mixed placement: logs everywhere, and photos only on the replica with a local
  cache; or the other way round. Which data lives where is the person's policy.
- Federation is the same mechanism. A member's homeserver follows a shared
  space's log, and gets the blobs it needs.

Only one thing can change: the head of each log. The head is the last sequence
number and hash, signed by the home server. Writing a new head uses a
conditional write (`If-Match` on S3, compare-and-swap on another `hs`). So two
writers can never both win, and no lock service is needed.

`hs` also serves the subset of the S3 API that it uses itself: PUT, GET with
`Range`, HEAD, DELETE, ListObjectsV2, conditional writes, and Signature
Version 4. So other S3 tools, such as `rclone`, can read and write an `hs`
replica too. We write both the S3 client and this server on top of hyper and
`ring`; Signature Version 4 is a few hundred lines. The AWS SDK is far outside
our budget. The `s3s` server framework adds 115 crates to a tokio and hyper
base of 28. Most of them come from `async-trait` and a second crypto library
(RustCrypto's `hmac`, `sha2` and `md-5`).

**Encryption at the boundary.** `hs` encrypts every segment, checkpoint and
blob before it goes out of your nodes. It uses AES-256-GCM in `ring`, with a
key per object derived from the storage key. It names each object by a keyed
hash. So the holder does not see the contents, and does not see which files
are the same.

### Derived state: SQLite

Each node that keeps plaintext builds its own `index.db` from the logs. It
uses SQLx, with SQLite compiled in (the `bundled` feature of the SQLite
driver). SQLx is async and works directly with the node's tokio runtime. Its
macros are off, and every query is a static SQL string with bound parameters.
The index has:

- **Current resources:** the latest representation of each resource, so a read
  is one lookup.
- **Links:** an edge table between resources and entities. "Everything with
  this person", "this place over the years" and the timeline are indexed
  queries.
- **Search:** FTS5 with BM25 ranking (see below).
- **Places:** the R*Tree module, for "what happened near here".
- **Memory for local AI:** facts and summaries that the person can see, edit
  and delete. They are stored as resources, like everything else.

The index is never replicated, and it never goes out of the node. Its schema
can change freely. When a new version of `hs` needs a different schema, it
builds the index again from the logs; it does not migrate it. The index runs
in WAL mode. Writes go through a pool with one connection, so they happen one
at a time. Reads use a second pool of a few read-only connections. Extension
loading stays off.

**Search.** The tokenizers that come with FTS5 do not handle Chinese well. We
tested SQLite 3.51. With the `trigram` tokenizer, `人民公园` and `妈妈去` are
found. But two-character words such as `公园` are not found, because trigram
needs three characters. `LIKE '%公园%'` finds them, but only by reading the
whole table. What works is this: cut runs of CJK characters into overlapping
two-character pieces, both when indexing and when querying, and give other
text to the `unicode61` tokenizer. With that, `公园`, `妈妈`, `散步`,
`人民公园` and `walk` all found the right rows, ranked by BM25. We register
this as our own FTS5 tokenizer, so the original text is stored once. If the
quality is not good enough, we can add word segmentation, such as jieba, later.

**Analysis.** Personal data is small: a hundred thousand photos and a few
million messages fit in SQLite. For heavy analysis, export writes Parquet or
CSV. A tool such as DuckDB can then run as a connected app.

**What we measured and did not choose.**

- **DuckDB with DuckLake**, the earlier choice. It gives snapshots, time travel
  (reading the data as it was at an earlier time) and strong analytics. We got
  DuckDB linked statically into one binary of 56 MB. But there were problems:
  - Chinese search needed the tantivy extension and jieba. That is 133 crates,
    with no settled way to compile them in.
  - DuckLake does not run in the browser.
  - Building `ducklake` and `fts` into a static library needed changes to the
    Nix build of DuckDB.
  - The C++ build needed an exception to our rule against cmake.

  With the log as the truth, the database is only an index, and SQLite is
  enough.

- **Turso**: the stable 0.8 has no full-text search; its tantivy feature did
  not compile on Rust 1.100. The 0.9 pre-releases search Chinese only with the
  `ngram` tokenizer, behind an experimental flag. The base stack grew to 268
  crates and took 50 s with 4 jobs, against 13 s with SQLite. Its file format
  works with SQLite's, and the index can be built again, so a change later is
  cheap. We will look again when Turso gets to 1.0.
- **tantivy and jieba-rs** as our own search engine: 133 crates, which built
  without problems on Rust 1.100. It would be a second index next to SQLite,
  with its own files to keep in agreement. It is not needed while
  two-character pieces in FTS5 search well enough. We have not measured its
  build time yet.

## One protocol everywhere

Server, phone and browser share the protocol and the formats. They do not
share a copy of the whole server:

| Where           | What runs there                                                    | Offline                                       |
| --------------- | ------------------------------------------------------------------ | --------------------------------------------- |
| Server, desktop | `hs`: logs, blobs, index, the HTTP API                             | it is the home of the data                    |
| Phone           | a native shell (UniFFI) with the device key, uploads, a cache      | cached resources, queued writes, photo upload |
| Browser         | the web app; a Service Worker with the Cache API and a write queue | cached resources, queued writes               |

**Portable crates.** `hs-core` (formats, CBOR, COSE), `hs-identity` and
`hs-plugin-api` are marked `[package.metadata.hs] portable = true`. They must
build for `wasm32-unknown-unknown`. The reasons: plugins are WebAssembly, the
phone shells use the same crates through UniFFI, and the web app may use them
to check signatures. `check` compiles them for that target. We made a test
build of a portable stack (CBOR, BLAKE3, Ed25519 and `ring`). It ran in
WebAssembly, and its BLAKE3 hashes were the same as those of the native build.

**Roles are capabilities, not kinds of program.** A node says what it can do:
whether it is always on and can be reached publicly, how much it can store,
whether it may keep plaintext, and whether it may be the home server of a
space. A box at home can take every role. A rented VPS can be a blind node
that keeps only ciphertext. A phone can become a small node later, if a real
need comes up; the protocol already lets it.

**Why not run the server in the browser.** That was our first design. It
meant DuckDB-Wasm in a worker, OPFS, and a second storage path that could not
run DuckLake, only to answer queries locally. A cache answers most of what
people do offline: read what they looked at recently, and write things down.
A cache costs a few kilobytes, and it is how the Web already works. Some
things a cache cannot do, such as searching the whole archive offline. We left
those out on purpose.

## Identity

**Person key.** A person is an Ed25519 key pair. The identity ID is derived
from the public key, so it needs no registry. A domain is only an alias.

**Identity log.** This is a short, append-only log signed by the person key.
It gives the current node keys, devices, addresses, domain aliases and key
rotations. Moving to another server is one new entry. Anyone who has the log
can check it without asking a third party. Nodes also serve the log as a W3C
DID document, so other software can resolve it with standard tools. We learn
from KERI, `did:plc` and Keybase's sigchains, but we do not depend on any of
them.

**Recovery.** If the person key is lost, everything must not be lost with it.
The person picks one or more ways to recover: a printed recovery kit, a
passphrase, or social recovery. Social recovery splits a key between trusted
friends (Shamir secret sharing, as in Dark Crystal). Which ways to use is the
person's policy.

**Node keys.** Every node has its own Ed25519 key, listed in the identity log.
Between nodes, TLS 1.3 shows it as a raw public key (RFC 7250, supported by
rustls). Every node also uses it to sign HTTP requests. There are no
certificate authorities between nodes.

**Devices.** A new device is added with a passkey (WebAuthn). It gets a device
key and a grant. Devices do not keep the person key. We write the small part of
WebAuthn we need on top of `ring`. The reason: `webauthn-rs` does its
cryptography through RustCrypto (RSA, P-256 to P-521, argon2 and more), and
that is a second crypto library next to `ring`.

**Address discovery.** Signed address records, in the style of pkarr, are
published to the homeservers of your contacts, which already relay for you.
You can also publish them to DNS. Nothing depends on a company's servers.

**Built-in identity provider.** homeserver is an OpenID Connect provider for
everyone on it (authorization code with PKCE, following OAuth 2.1). Plugins,
connected apps and any other service that supports standard sign-in use the
same identity. Some apps do not support OIDC. For them, homeserver's reverse
proxy checks the session itself (forward auth). ID tokens are signed with
EdDSA and, for compatibility, ES256. Both are in `ring`. The provider is a
small number of endpoints, so we write it ourselves. `openidconnect` is a
client library, not a provider, and it too brings RustCrypto's RSA and
elliptic curves.

## Permissions

Principle 05 says that you always know who can see what, and that you can take
it back. That is only possible if there is one place to look.

**Everyone is a client of the same API.** Your devices, the web app, a
connected app, a plugin, an AI assistant over MCP, and a friend's homeserver
all get to your data through the same HTTP resources. None of them goes to the
database directly.

**Grants.** A grant says:

- who: a device key, an OAuth client, a plugin, or a person;
- what: URL prefixes, resource types, spaces;
- how: read, write, share;
- until when.

Grants are events in your private log, so they are in order, signed, and kept
in history. OAuth scopes, plugin capabilities and MCP access are all given as
grants. A shared space adds members with roles, and roles work the same way.

**Seeing and revoking.** "Who can see this photo?" and "what can this app get
to?" are queries over grants and memberships. Every grant has an access log
that is kept on your node. Revoking is one event. In spaces with end-to-end
encryption, removing a member also changes (rotates) the MLS group key. What
someone has already copied cannot be taken back, and the interface says so.

## Networking

**Stack.** We use:

- tokio, with only the features we use;
- quinn for QUIC;
- rustls with the `ring` backend;
- hyper for HTTP/1.1 and HTTP/2;
- `h3` for HTTP/3.

iroh uses the same building blocks, but we do not take iroh's own layers.
Without a database, this stack is 74 crates, and `h3` adds 9.

**Path layer.** `hs-net` puts a virtual UDP socket, keyed by peer, under quinn.
It does this through quinn's socket abstraction. For each peer, it picks the
best path: direct IPv6, direct IPv4 through a port mapping or a hole punch, or
a relay. When paths change, QUIC connection migration keeps the connection
open. This is the idea behind iroh's magicsock. iroh's QUIC fork, noq, moves
NAT traversal and multipath into QUIC itself, following IETF drafts. Upstream
quinn does not support them yet. We will take them when they are RFCs and in
upstream quinn. Until then, our layer stays small, and we can replace it.

**NAT traversal.** IPv6 first. We wrote port mapping with PCP, NAT-PMP and
UPnP-IGD ourselves. The reason: `igd-next` does only UPnP-IGD, and it brings
its own HTTP client (`attohttpc`) and two XML libraries next to hyper. Each
peer tells the other the address it sees, as in the QUIC address discovery
draft. Hole punching is organized over a relayed connection that is already
open.

**Relays are homeservers.** Any homeserver that can be reached can relay, keep
mail, and keep encrypted replicas for people its owner trusts. The owner's
policy controls this. Relayed traffic has end-to-end encryption, so a relay
sees only which keys talk to each other. There may be official relays, as one
more option. They may also be the first contact for someone who has no friends
on homeserver yet. An invite link from a friend has the friend's relay in it
instead. Nothing stops working without official relays. For networks that
block UDP, relays also take connections over TLS on TCP port 443.

**Reaching your homeserver from a browser.** Browsers need a normal HTTPS URL.
You need one, and so does a friend who opens a link to a shared album. There
are three ways:

1. With a public address and a domain, the owner runs a reverse proxy (Caddy,
   nginx) in front of `hs`. `hs` does not talk ACME. HTTPS for a public domain
   is a solved problem, and the project puts its effort into peer-to-peer
   reachability.
2. Without a public address, a homeserver that can be reached forwards TLS by
   SNI, and it does not terminate (end) the TLS. This can be a friend's
   homeserver, your own VPS, or an official one. TLS still ends at home, and
   the forwarder sees only ciphertext.
3. On the local network, or when the web app already knows the node's key, a
   browser can use WebTransport with `serverCertificateHashes`. Then it needs
   neither of the two ways above. WebTransport has been Baseline in all major
   browsers since March 2026.

## Encryption

- **In transit:** TLS 1.3 everywhere, with node keys between nodes, and signed
  HTTP requests on top.
- **At rest outside your nodes:** `hs` encrypts every segment, checkpoint and
  blob before it uploads them to a replica that you do not control (see
  Replicas).
- **Shared spaces:** end-to-end encryption with **MLS (RFC 9420)** between the
  members' homeservers. Each person's homeserver is one MLS member; each
  device is not. This keeps out most of the multi-device complexity that
  messengers with end-to-end encryption have to handle. The space's home
  server puts MLS commits in order, the same as other events. Relays,
  mailboxes and replicas keep only ciphertext.
- **Blind nodes:** a node you rent can be set to keep only ciphertext. It
  relays, puts encrypted events in order and stores replicas, but it never
  sees plaintext.
- **Library:** `ring` only, because rustls already needs it, and it builds for
  WebAssembly. Mixing crypto libraries means compiling two generations of the
  same crates. Our test graph had two copies of `digest`, `sha2`,
  `block-buffer` and others. We use **mls-rs** (MIT OR Apache-2.0, maintained
  by AWS). It is 63 crates over the base stack, against 95 for OpenMLS. We put
  a small crypto provider of our own on top of `ring` under it.

## Personal cluster

One person or family can run more than one node under one identity: a box at
home, a VPS, a laptop. One node is the home server of the personal spaces, and
it puts their events in order. The other nodes follow the logs, build their
own indexes, and serve reads. A blind node only keeps encrypted replicas.

The home server role moves by lease. A lease needs a majority of the voting
nodes. When there are only two voting nodes, a friend's homeserver can be a
witness. A witness only decides ties, and it sees no data. Every node already
has the logs, so moving the role does not move data. v1 comes with a single
server node, and encrypted replicas on another `hs` or a bucket.

## Plugins

Two kinds of plugin share the permission model above. A plugin gets exactly
the grants the person gives it, and the person can see them and revoke them.

**WebAssembly components.** They run inside the node, in a sandbox made by
wasmtime. Their interface is defined in WIT and built on `wasi:http`. A
plugin:

- handles the HTTP requests that are routed to it;
- reads and writes data through the same HTTP API as every other client;
- may send requests out only to the origins it has a grant for;
- can also run timers, subscribe to events, and serve a UI panel.

Any language that compiles to WebAssembly components can be used. Common uses:
importers, small personal tools, automations, mutual-aid policies, and bridges
to other protocols such as ActivityPub.

wasmtime with Cranelift adds 64 crates over the base stack, and about 7 MB to
the release binary. With only the Pulley interpreter, it adds 42 crates and
builds in about half the time. wasmi is much lighter (16 crates), but it does
not support components. wasmtime depends on `async-trait`, which `deny.toml`
bans. So the ban has an exception for wasmtime only. The guest bindings in
`hs-plugin-api` are generated once and committed. They are never generated in
`build.rs`.

**Connected apps.** Specialist software, such as Jellyfin, Home Assistant or
Forgejo, runs as its own program. The person or their operating system
installs it. homeserver is not a container platform. A small connector
describes the app; the connector is itself a WebAssembly plugin. homeserver
then gives the app:

- sign-in, as an OIDC client registration or through forward auth;
- reachability from anywhere, through homeserver's HTTPS and NAT traversal,
  with no ports to open;
- limited access to homeserver data through grants, such as read-only access
  to the photo library;
- events and backup hooks.

## Clients

- **One web app.** `hs` serves it. It first renders as HTML, and it uses
  standard Web Components and a small bundle. Nix builds the bundle, the same
  as the docs. People can install the web app.
- **Phone apps** are there for what a web app cannot do in the background,
  such as uploading new photos. They are thin native shells (Kotlin, Swift).
  They keep the device key, use the portable crates through UniFFI, and show
  the web app in a web view.
- **Photos:** the phone makes previews when it uploads, with its own hardware
  decoders. The server keeps the originals and those previews. The server
  decodes JPEG, PNG and WebP in pure Rust. For HEIC and video, it uses ffmpeg
  if the host has it. If not, it uses the previews from the client.
- **Notifications:** Web Push wherever a web app can get it. A native app on
  iOS needs Apple's push service. That goes through a registered gateway. The
  gateway only wakes the app, carries no content, and anyone can run one.
- **Standard protocols:** WebDAV, CalDAV/CardDAV, the Bitwarden API, OpenID
  Connect, and MCP for AI assistants. JMAP may come later.
- **AI:** the core does not include a model runtime. Local models (such as a
  llama.cpp server) connect as connected apps through the common OpenAI-style
  API. homeserver gives the memory, the grants and the MCP layer.

## Operations

**Process.** One executable and one data directory. One tokio runtime with a
few workers. Each space's log is appended under one async lock. SQLite has one
write connection and a few read connections. A blocking pool does hashing,
encryption and media work.

**Updates.** `hs` downloads a signed release: an Ed25519 signature over a
reproducible Nix build. It keeps the previous binary, restarts, and runs a
health check. If the check fails, it goes back (rolls back) to the previous
binary. Updates never migrate the logs in place. New versions read old event
formats, and the index is simply built again, so a rollback loses nothing.
Under systemd, it uses systemd's restart. An appliance image with A/B updates
comes later.

**Recovery.** If `hs` finds the index damaged, or not in agreement with the
logs, it deletes the index and builds it again. The health page says so. A
person never has to repair a database.

**Distribution.** There is one static executable per platform (Linux x86_64
and aarch64, musl), with SQLite and the web app inside. There is a NixOS
module. There is a container image for people who already run containers.
macOS builds are for development.

**Health.** A health page in plain language says what is wrong and what to do.
Logs stay local. There is no telemetry.

## Testing

- Deterministic simulation of many nodes, networks, replicas and failures in
  one process, built on the sans-IO cores. quinn's protocol core is sans-IO
  too, so QUIC runs in the simulation. The approach follows FoundationDB and
  TigerBeetle.
- Property tests for log ordering, membership, grants and replica sync. The
  key property: an index built one event at a time is equal to an index built
  again from nothing.
- Fuzzing for every parser (CBOR, WebDAV XML, WebAuthn data and S3
  responses).
- `check` builds the portable crates for `wasm32-unknown-unknown`.
- Compatibility tests against real clients and services: DAVx⁵, Thunderbird,
  the Bitwarden clients, common OIDC relying parties, and S3-compatible stores.
  The stores are the large cloud providers, Garage and MinIO, and `rclone` and
  the AWS CLI against `hs` as a server.

## Crates

This is the target layout. We add a crate when there is code for it. Heavy
dependencies stay inside the one crate that needs them.

| Crate            | Contents                                                                          | Portable | Heavy dependencies   |
| ---------------- | --------------------------------------------------------------------------------- | :------: | -------------------- |
| `hs-core`        | resource model, deterministic CBOR, COSE, event and segment formats               |   yes    | none                 |
| `hs-identity`    | person and node keys, identity log, DID documents, recovery                       |   yes    | ring                 |
| `hs-space`       | logs, ordering, checkpoints, membership, grants, MLS                              |    no    | mls-rs               |
| `hs-objects`     | blobs, replicas (local directory, another `hs`, S3 client and server), encryption |    no    | ring                 |
| `hs-store`       | the derived index in SQLite: current resources, links, search, places             |    no    | SQLx                 |
| `hs-api`         | the HTTP API as a request handler, WebDAV, CalDAV/CardDAV, MCP                    |    no    | none                 |
| `hs-idp`         | OpenID Connect provider, passkeys, forward auth                                   |    no    | none                 |
| `hs-node`        | puts the crates above together into one node                                      |    no    | none                 |
| `hs-net`         | QUIC, HTTP/3, path layer, relay, mailbox                                          |    no    | quinn, rustls, hyper |
| `hs-media`       | metadata, previews                                                                |    no    | image decoders       |
| `hs-plugin-host` | WebAssembly components                                                            |    no    | wasmtime             |
| `hs-plugin-api`  | Plugin Interface (WIT, guest bindings; MIT OR Apache-2.0)                         |   yes    | none                 |
| `hs-sim`         | deterministic simulation for tests                                                |    no    | none                 |
| `hs`             | the executable                                                                    |    no    | none                 |

Release 0.1.0 has part of this. `CHANGELOG.md` in the repository lists what is
not there yet.

Release 0.1.0 has no web app. People use the `hs` command line, which works
with a node through the same API that every client uses. If `HS_WEB_DIR` is
set, a build puts the static files in it inside the executable. If not, the
build serves a page that points to the command line.

## Dependency budget

| Stack                                                                              | Crates | Clean build                  | Release binary |
| ---------------------------------------------------------------------------------- | -----: | ---------------------------- | -------------: |
| Network base: tokio, quinn, rustls (ring), hyper, BLAKE3, Ed25519, serde, minicbor |     68 | 10 s with 4 jobs             |         0.6 MB |
| The same with rusqlite (bundled)                                                   |     77 | 10 s with 4 jobs             |         2.3 MB |
| The same with SQLx instead (sqlx-core and sqlx-sqlite, bundled)                    |    130 | 15 s with 4 jobs             |         3.0 MB |
| Base with rusqlite + wasmtime (Cranelift, components) + mls-rs + an ACME client    |    232 | 36 s with 4 jobs (dev build) |        10.2 MB |
| For comparison: an iroh endpoint alone (Linux)                                     |    231 | 40 s                         |          12 MB |

We measured these rows on one machine, with separate target directories and
no compiler cache. Each program really opens SQLite and runs a query; if it did
not, the unused code would be stripped out. We use SQLx through `sqlx-core` and
`sqlx-sqlite`, not through `sqlx`. The reason: `sqlx` forces its `migrate`
feature, and that brings SHA-2 from RustCrypto. The row with wasmtime and the
Turso comparison were measured earlier, with rusqlite.

Candidates we measured:

| Candidate                                   | Crates over base | Decision                                                                                                       |
| ------------------------------------------- | ---------------: | -------------------------------------------------------------------------------------------------------------- |
| SQLx (bundled)                              |               62 | Use, for the derived index: async, works with tokio directly                                                   |
| rusqlite (bundled)                          |                9 | Not used: lighter, but blocking; needs a thread pool                                                           |
| DuckDB, prebuilt                            |      71 in total | Not used: see Storage                                                                                          |
| tantivy + jieba-rs                          |     133 in total | Not used for now: a second index; FTS5 is enough for now                                                       |
| Turso                                       |    268 with base | Not used for now: 50 s against 13 s; look again at 1.0                                                         |
| h3 + h3-quinn                               |                9 | Use, for HTTP/3 between nodes                                                                                  |
| wasmtime, Cranelift                         |               64 | Use, for release builds                                                                                        |
| wasmtime, Pulley only                       |               42 | Build option for slow machines                                                                                 |
| wasmi                                       |      16 in total | Not used: no Component Model                                                                                   |
| mls-rs                                      |               63 | Use, with our own `ring` provider                                                                              |
| OpenMLS                                     |               95 | Not used: we need one MLS library; build times not compared                                                    |
| webauthn-rs                                 |              134 | Not used: a second crypto library (RustCrypto)                                                                 |
| openidconnect                               |              102 | Not used: a client library, and RustCrypto                                                                     |
| igd-next                                    |               33 | Not used: a second HTTP client; UPnP-IGD only                                                                  |
| s3s                                         |              115 | Not used: `async-trait` and RustCrypto; we write the subset                                                    |
| axum                                        |               14 | Not decided; a small router of our own on hyper may be enough                                                  |
| rmcp                                        |               23 | Not decided                                                                                                    |
| Automerge / Loro / Yrs                      |     13 / 80 / 17 | Decide when work on shared documents starts                                                                    |
| clap (derive)                               |                7 | Use, for the command line: help, errors, suggestions                                                           |
| clap_complete                               |                2 | Use: shell completions from the same clap definitions; asks `hs` for spaces and grants; brings `is_executable` |
| time                                        |                1 | Use: already built for SQLx and rcgen; only `time-macros` is new                                               |
| percent-encoding, form_urlencoded, httpdate |                0 | Use: already built for hyper and url                                                                           |
| data-encoding                               |                1 | Use, for hex and lowercase base32                                                                              |
| sfv                                         |                3 | Use, for the structured fields of HTTP Message Signatures                                                      |
| tar                                         |                2 | Use, for `hs ctl export`                                                                                       |
| unicode-width                               |                1 | Use, to line up CLI tables that have CJK text                                                                  |
| utoipa                                      |                2 | Use: makes the OpenAPI document from the API's types; serde, indexmap and syn are already built                |
| serde_urlencoded                            |                2 | Use: typed queries that the server and the client share                                                        |
| rustix (`termios`)                          |                0 | Use: already built for tempfile; hides a typed token                                                           |
| kamadak-exif + imagesize                    |                3 | Use: EXIF and image sizes from JPEG, PNG, WebP and HEIF                                                        |
| crab_nat                                    |      30 in total | Not used: `rand` and `toml_edit` for NAT-PMP; we write it                                                      |
| coset                                       |      15 in total | Not used: a second CBOR library; our COSE subset is small                                                      |
| blahaj                                      |      11 in total | Not used: `rand` for Shamir sharing; we use `ring`                                                             |
| calcard                                     |      25 in total | Not used: chrono, chrono-tz and a mail parser                                                                  |
| ical                                        |       7 in total | Not used: second major versions of `thiserror` and `syn`                                                       |

The "Crates over base" column gives context. Decisions rest on the reason in
the last column. The CI build-time budget grows only when we accept a measured
dependency, for a reason recorded in its pull request.

## Open questions

- The event, segment and checkpoint formats in detail, and how they get
  versions for twenty years.
- What erasing a payload means, exactly, for members who already got it; and
  how history retention is written as policy.
- Whether two-character pieces search Chinese well enough, or whether word
  segmentation is worth its size. The same question is open for Japanese and
  Korean.
- Vector search for AI memory: in SQLite, in a plugin, or not at all.
- How much the browser and phone cache, and how queued writes are shown when
  they conflict.
- The DID method for the identity log.
- The resources and media types between homeservers in detail: sync,
  backpressure, versioning.
- Home server leases in the personal cluster, and how a witness takes part.
- The grant format, and the WIT world for plugins on top of `wasi:http`.
- Which CRDT library to use for shared documents.
- How official relays are run and paid for, without becoming a central
  dependency.
