---
title: Architecture and technical route
description: How homeserver is built, the technology we chose, and why.
---

This page records the overall architecture of homeserver and the technology
behind it. Each decision says why we made it and, where it matters, what would
make us revisit it. Details will change as we build; the direction should not
change without a discussion.

Measurements on this page were taken on 2026-11-25 with Rust 1.100, on a
15-core Apple Silicon machine unless noted otherwise. "Crates" counts the
normal (non-dev) dependency graph. Crate counts are context, not a verdict:
a library split into small crates builds in parallel and rebuilds less, so
it can cost less than one big crate. We judge a dependency by clean and
incremental build time, binary size, and what it brings with it: a second
crypto library or HTTP stack, banned crates such as `async-trait`, heavy
procedural macros, build scripts, or C and C++ code.

## Where the architecture comes from

A few lines of the vision decide most of the architecture:

- One CPU core and 1 GB of memory serve a family of ten; idle memory stays
  under 150 MB.
- One program and one data directory; copying the directory is a backup.
- Data can still be read in full in twenty years.
- You always know who can see what, and you can take it back.
- A family member who is not technical can recover from a problem alone.
- A one-person server is first class, and its cost grows with your own life,
  not with the network.

The Web supplies the ideas that answer them: resources with URLs and several
representations; immutable things that anyone can cache, with intermediaries
that need no trust; clients that are user agents and work offline through
caches; links as relations; and the rule of least power.

## Rules that shape everything

1. **Built on the Web.** Everything has a URL, everything speaks HTTP, and
   every interface uses an open standard where one exists, between
   homeservers as well as at the edges.
2. **One executable, one data directory.** `hs` is a single file with
   everything linked in. Nothing is downloaded at run time, and no other
   program is needed, for storage or for anything else. When this conflicts
   with another rule, this one wins.
3. **The log is the truth; everything else is derived.** What a person keeps
   is recorded in signed, append-only logs and content-addressed blobs.
   Tables, search indexes and previews are built from them and can be thrown
   away and rebuilt at any time.
4. **Immutable things can live anywhere.** Sealed log segments and blobs
   never change, so they are cacheable HTTP resources. A local directory,
   another `hs` or an S3 bucket can hold them. Sync, backup, migration and
   federation are one operation: fetch what you lack.
5. **One protocol everywhere.** Every node speaks the same HTTP API and the
   same formats. Browsers and phones are user agents of their own
   homeserver: they cache, queue writes while offline, and hold device keys.
   They do not each run a copy of the server.
6. **Your nodes are your trust boundary.** Plaintext lives only on nodes that
   belong to you. Anything that leaves for someone else's machine (a cloud
   bucket, a relay, a mailbox, a friend's server that is not a member) is
   encrypted first.
7. **One permission model.** Devices, apps, plugins, AI assistants and
   friends all reach your data through the same HTTP API, with grants that
   you can list in one place and revoke in one place.
8. **Order, don't agree.** Each shared thing has one home server that orders
   its events. There is no global replicated state and no consensus across
   the network.
9. **Sans-IO cores.** Protocol logic is written as state machines with time,
   randomness, storage and the network injected. The same code runs in
   production and in a deterministic simulation.
10. **Mechanism in the core, policy outside it.** Whom to relay for, where to
    keep which data, how long to keep history, what to accept from
    strangers: these are policies with defaults, replaceable by the person or
    by a plugin.
11. **Compile time is a budget.** Every dependency is measured by build
    time, binary size and what it brings with it, not by how many crates it
    is split into. We write small things ourselves when a library would bring
    a parallel stack. No C or C++ that needs cmake; small C compiled by the
    `cc` crate, such as SQLite, is allowed.

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

The Web is the largest working decentralized system: anyone can publish,
anyone can link, and any client can read. homeserver adopts its ideas and its
standards instead of inventing parallel ones.

**Everything has a URL.** Every object, space, log, person and blob is an HTTP
resource. Paths start with the person's identity ID, not a server name, so
links survive moving servers: the identity log says which nodes currently
serve that identity. A domain is only an alias.

**HTTP semantics, used fully.** Methods mean what HTTP says they mean. Blobs
and sealed log segments never change, so their ETag is their hash and they
are served with `Cache-Control: immutable` and `Range`. Updates use
conditional requests (`If-Match`), so a stale write gets `412` instead of
silently overwriting. One resource has several representations, chosen by
content negotiation: a contact is JSON, vCard or a page; an event is JSON or
iCalendar; a photo is the original or a preview. Relations are links (`Link`
headers, RFC 8288), so clients follow links instead of building URLs.

**Clients are user agents.** The web app renders as HTML first and enhances
with standard Web Components. Offline use works the way the Web already
works: a Service Worker keeps immutable resources in the Cache API, and
writes made offline are queued and sent when the network returns, each with
an idempotency key so a retry is never applied twice. The server stays the
one place where the data lives and is queried.

**HTTP between homeservers too.** Homeservers talk HTTP/3 over our QUIC
stack, falling back to HTTP/2 over TLS on TCP port 443. A space's log is a
resource read in ranges; new events are POSTed to an inbox. Requests are
signed with the node key using HTTP Message Signatures (RFC 9421), so a
signature still holds when a request passes through relays. What we define
ourselves is limited to media types and the meaning of the resources. Adding
`h3` and `h3-quinn` costs 9 crates.

**Web standards for sign-in, push and apps.**

- Sign-in: passkeys (WebAuthn), OAuth 2.1 and OpenID Connect, with DPoP
  (RFC 9449) to bind tokens to a device.
- Notifications: Web Push (RFC 8030, 8291 and 8292), with `hs` as the
  application server. It works for installed web apps on Android, on iOS
  16.4 and later, and on desktops. UnifiedPush speaks the same protocol.
- Plugins: WebAssembly, a W3C standard. The plugin interface is built on
  `wasi:http`, so a plugin is a handler that receives requests and returns
  responses, much like a Service Worker.
- Plugin UIs run in their own origin with a strict Content Security Policy,
  so the browser's same-origin model isolates them.
- Replicas speak a subset of the S3 API, which is plain HTTP with
  conditional writes.

**Formats.** HTTP APIs use JSON. Where a vocabulary exists we reuse it
(ActivityStreams 2.0, schema.org, vCard, iCalendar) instead of naming things
anew. Log events are deterministic CBOR (RFC 8949, section 4.2) signed with
COSE (RFC 9052), the same pair WebAuthn uses. Export writes each type in its
common open format.

## Data model

**Resources.** Everything a person keeps is a resource: a photo, a file, a
message, a note, a contact, an event, a password. A resource has a type, a
schema version, fields, and references to blobs.

**Entities and links.** People, places, times and things are resources too.
Other resources link to them. "Open a person and see everything with them" is
a query over links, not a feature of each app. This is where principle 02
lives in the code.

**Spaces.** A space is the unit of sharing and sync: your private space, your
family, a chat, a shared album, a shared folder. A space has members (keys
with roles), one home server, and one log.

**Logs.** A space's log is an append-only list of signed events. An event is
an operation on a resource, in HTTP's terms: create, replace, patch or
delete, with the new representation or a reference to it. The home server
gives each event a sequence number and chains it by hash. Logs are stored in
segments; a full segment is sealed, named by its hash and never changes
again. Grants, membership changes and MLS commits are events too, so they
are ordered and auditable like everything else.

Why an ordered log instead of CRDTs everywhere: permission changes need a
total order anyway (Matrix's state resolution shows the cost of avoiding one),
an ordered log is cheap for small servers, and MLS needs an ordering service
for its commits, which the home server already provides. CRDTs are used only
inside resources that are edited together, such as collaborative documents,
carried as event payloads. Offline edits queue on the client and are ordered
when they reach the home server.

If a home server is gone for good, members can move the space to another home
server with a signed handover, the same way a person moves their identity.

**Blobs.** File contents are stored by BLAKE3 hash. Identical content is
stored once. Transfers are verified as they stream, in the style of Bao, so
an untrusted node can serve a blob.

**Forgetting.** An append-only log must still allow real deletion. The hash
chain covers the hash of each event's payload, not the payload itself, so a
payload can be erased later while the chain still verifies. A delete event
erases the payloads and blobs it makes unreachable on every node that follows
the log. How long earlier versions are kept is the person's
policy: forever, a year, or not at all.

**Checkpoints.** Replaying years of events on every start would be slow, so
the home server writes signed checkpoints: the current state of a space at a
sequence number, as an immutable object next to the segments. A node builds
its tables from the latest checkpoint and the events after it.

**Twenty years.** The log and checkpoint formats are specified in the docs
and kept small, so a person can read their data with the spec and a CBOR
library, without `hs`. Export turns the same data into vCard, iCalendar,
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

Copying `logs/`, `blobs/` and `keys/` is a complete backup. `index.db` is
rebuilt when it is missing, damaged or from an older version of `hs`.

### Replicas

**`hs` is all you need to store.** A replica holds copies of segments,
checkpoints and blobs, and it is one of:

- A local directory, such as an external disk. `hs` writes the files itself:
  write to a temporary file, `fsync`, then rename.
- Another `hs`: a box at home, a VPS you rent, or a friend's homeserver. It
  serves segments and blobs as ordinary HTTP resources, and accepts writes
  signed with your node key (RFC 9421). A rented VPS or a friend's
  homeserver is usually a blind node and only ever holds ciphertext.
- An S3-compatible bucket, optional: a cloud provider, or Garage or MinIO if
  you already run them.

Because sealed segments and blobs never change, every replica operation is
the same: list what the other side has, and copy what it lacks.

- Sync and backup: keep a replica up to date.
- Migrate: sync, make the replica primary, and optionally delete the old
  copy.
- Mixed placement: logs everywhere and photos only on the replica with a
  local cache, or the other way round. Which data lives where is the
  person's policy.
- Federation is the same mechanism: a member's homeserver follows a shared
  space's log and fetches the blobs it needs.

The only mutable thing is the head of each log: its last sequence number and
hash, signed by the home server. Writing a new head uses a conditional write
(`If-Match` on S3, compare-and-swap on another `hs`), so two writers can
never both win and no lock service is needed.

`hs` also serves the subset of the S3 API that it uses itself (PUT, GET with
`Range`, HEAD, DELETE, ListObjectsV2, conditional writes, Signature
Version 4), so other S3 tools such as `rclone` can read and write an `hs`
replica too. We write both the S3 client and this server on top of hyper and
`ring` (Signature Version 4 is a few hundred lines). The AWS SDK is far
outside our budget, and the `s3s` server framework adds 115 crates to a
tokio and hyper base of 28, most of them from pulling in `async-trait` and
a second crypto library (RustCrypto's `hmac`, `sha2` and `md-5`).

**Encryption at the boundary.** `hs` encrypts every segment, checkpoint and
blob before it leaves your nodes (AES-256-GCM in `ring`, with a key per
object derived from the storage key) and names it by a keyed hash, so the
holder sees neither the contents nor which files are equal.

### Derived state: SQLite

Each node that holds plaintext builds its own `index.db` from the logs, with
SQLx and SQLite compiled in (the SQLite driver's `bundled` feature). SQLx is async and
works directly with the node's tokio runtime; its macros are off, and every
query is a static SQL string with bound parameters:

- **Current resources:** the latest representation of each resource, so
  reads are one lookup.
- **Links:** an edge table between resources and entities. "Everything with
  this person", "this place over the years" and the timeline are indexed
  queries.
- **Search:** FTS5 with BM25 ranking (see below).
- **Places:** the R*Tree module, for "what happened near here".
- **Memory for local AI:** facts and summaries the person can see, edit and
  delete, stored as resources like everything else.

The index is never replicated and never leaves the node. Its schema can
change freely: a new version of `hs` that needs a different schema rebuilds
it from the logs instead of migrating it. It runs in WAL mode: writes
go through a pool with one connection, so they are serial; reads use a
second pool of a few read-only connections. Extension loading stays off.

**Search.** FTS5's built-in tokenizers do not handle Chinese well. We tested
SQLite 3.51: with the `trigram` tokenizer, `人民公园` and `妈妈去` are found,
but two-character words such as `公园` are not, because trigram needs three
characters; `LIKE '%公园%'` finds them only by scanning the whole table.
What works is to split runs of CJK characters into overlapping
two-character pieces when indexing and when querying, and to leave other
text to the `unicode61` tokenizer. With that, `公园`, `妈妈`, `散步`,
`人民公园` and `walk` all found the right rows, ranked by BM25. We register
this as an FTS5 tokenizer of our own, so the original text is stored once.
Word segmentation, such as jieba, can be added later if the quality is not
good enough.

**Analysis.** Personal data is small: a hundred thousand photos and a few
million messages fit in SQLite. For heavy analysis, export writes Parquet
or CSV, and a tool such as DuckDB can run as a connected app.

**What we measured and did not choose.**

- **DuckDB with DuckLake**, the previous choice. It gives snapshots, time
  travel and strong analytics, and we got DuckDB linked statically into one
  56 MB binary. But Chinese search needed the tantivy extension and jieba
  (133 crates, with no settled way to compile them in), DuckLake does not run
  in the browser, building `ducklake` and `fts` into a static library
  needed changes to the Nix build of DuckDB, and the C++ build needed an
  exception to our rule against cmake. With the log as the truth, the
  database is only an index, and SQLite is enough.
- **Turso**: the stable 0.8 has no full-text search (its tantivy feature did
  not compile on Rust 1.100). The 0.9 pre-releases search Chinese only with
  the `ngram` tokenizer, behind an experimental flag. The base stack grew to
  268 crates and 50 s with 4 jobs, against 13 s with SQLite. Its file format is compatible with
  SQLite, and the index is rebuildable, so switching later is cheap; we look
  again when Turso reaches 1.0.
- **tantivy and jieba-rs** as a search engine of our own: 133 crates, which
  built cleanly on Rust 1.100. It would be a second index beside SQLite,
  with its own files to keep consistent, and it is not needed while
  two-character pieces in FTS5 search well enough. Its build time has not
  been measured yet.

## One protocol everywhere

Server, phone and browser share the protocol and the formats, not a copy of
the whole server:

| Where           | What runs there                                                    | Offline                                       |
| --------------- | ------------------------------------------------------------------ | --------------------------------------------- |
| Server, desktop | `hs`: logs, blobs, index, the HTTP API                             | it is the home of the data                    |
| Phone           | a native shell (UniFFI) with the device key, uploads, a cache      | cached resources, queued writes, photo upload |
| Browser         | the web app; a Service Worker with the Cache API and a write queue | cached resources, queued writes               |

**Portable crates.** `hs-core` (formats, CBOR, COSE), `hs-identity` and
`hs-plugin-api` are marked `[package.metadata.hs] portable = true` and must
build for `wasm32-unknown-unknown`: plugins are WebAssembly, the phone shells
use the same crates through UniFFI, and the web app may verify signatures
with them. `check` compiles them for that target. A test build of a portable
stack (CBOR, BLAKE3, Ed25519 and `ring`) ran in WebAssembly, and its BLAKE3
hashes matched the native build.

**Roles are capabilities, not kinds of program.** A node advertises what it
can do: whether it is always on and publicly reachable, how much it can
store, whether it may hold plaintext, and whether it may be the home server
of a space. A box at home can take every role. A rented VPS can be a blind
node that holds only ciphertext. A phone can become a small node later if a
real need shows up; the protocol already allows it.

**Why not run the server in the browser.** We designed that first. It meant
DuckDB-Wasm in a worker, OPFS, and a second storage path that could not run
DuckLake, only to answer queries locally. A cache answers most of what people
do offline (read what they looked at recently, write things down), costs a
few kilobytes, and is how the Web already works. What a cache cannot do, such
as searching the whole archive offline, is left out on purpose.

## Identity

**Person key.** A person is an Ed25519 key pair. The identity ID is derived
from the public key, so it needs no registry. A domain is only an alias.

**Identity log.** A short, append-only log signed by the person key declares
the current node keys, devices, addresses, domain aliases and key rotations.
Moving to another server is one new entry. Anyone holding the log can verify
it without asking a third party. Nodes also serve the log as a W3C DID
document, so other software can resolve it with standard tools. We learn
from KERI, `did:plc` and Keybase's sigchains, without depending on any of
them.

**Recovery.** Losing the person key must not mean losing everything. The
person chooses one or more ways to recover: a printed recovery kit, a
passphrase, or social recovery that splits a key between trusted friends
(Shamir secret sharing, as in Dark Crystal). Which ones to use is the
person's policy.

**Node keys.** Every node has its own Ed25519 key, listed in the identity
log. Between nodes it is presented in TLS 1.3 as a raw public key (RFC 7250,
supported by rustls), and every node uses it to sign HTTP requests. There are
no certificate authorities between nodes.

**Devices.** A new device is added with a passkey (WebAuthn) and gets a
device key and a grant. Devices do not hold the person key. We write the
small part of WebAuthn we need on top of `ring`: `webauthn-rs` does its
cryptography through RustCrypto (RSA, P-256 to P-521, argon2 and more), a
second crypto library beside `ring`.

**Address discovery.** Signed address records, in the style of pkarr, are
published to the homeservers of your contacts, which already relay for you,
and optionally to DNS. Nothing depends on a company's servers.

**Built-in identity provider.** homeserver is an OpenID Connect provider
(authorization code with PKCE, following OAuth 2.1) for everyone on it.
Plugins, connected apps and any other service that supports standard sign-in
use the same identity. For apps without OIDC support, homeserver's reverse
proxy checks the session itself (forward auth). ID tokens are signed with
EdDSA and, for compatibility, ES256. Both are in `ring`. The provider is a
handful of endpoints, so we write it ourselves. `openidconnect` is a client
library, not a provider, and it too brings RustCrypto's RSA and elliptic
curves.

## Permissions

Principle 05 asks that you always know who can see what and can take it
back. That is only possible if there is one place to look.

**Everyone is a client of the same API.** Your devices, the web app, a
connected app, a plugin, an AI assistant over MCP and a friend's homeserver
all reach your data through the same HTTP resources. None of them reaches the
database directly.

**Grants.** A grant says who (a device key, an OAuth client, a plugin, a
person), what (URL prefixes, resource types, spaces), how (read, write,
share) and until when. Grants are events in your private log, so they are
ordered, signed and kept in history. OAuth scopes, plugin capabilities and
MCP access are all expressed as grants. A shared space adds members with
roles, which work the same way.

**Seeing and revoking.** "Who can see this photo?" and "what can this app
reach?" are queries over grants and memberships. Every grant has an access
log kept on your node. Revoking is one event. For end-to-end encrypted
spaces, removing a member also rotates the MLS group key. What someone has
already copied cannot be taken back; the interface states this.

## Networking

**Stack.** tokio with only the features we use, quinn for QUIC, rustls with
the `ring` backend, hyper for HTTP/1.1 and HTTP/2, and `h3` for HTTP/3. These
are the same building blocks iroh uses, without iroh's own layers. Without a
database this stack is 74 crates, and `h3` adds 9.

**Path layer.** `hs-net` adds a peer-keyed virtual UDP socket underneath
quinn, through quinn's socket abstraction. For each peer it chooses the best
path: direct IPv6, direct IPv4 through a port mapping or a hole punch, or a
relay. When paths change, QUIC connection migration keeps the connection
alive. This is the idea behind iroh's magicsock. iroh's QUIC fork, noq, moves
NAT traversal and multipath into QUIC itself, following IETF drafts. Upstream
quinn does not support them yet. We will adopt them once they are RFCs and
in upstream quinn. Until then our layer stays small and replaceable.

**NAT traversal.** IPv6 first. Port mapping with PCP, NAT-PMP and UPnP-IGD,
written by us: `igd-next` covers only UPnP-IGD and brings its own HTTP
client (`attohttpc`) and two XML libraries beside hyper. Each peer tells the other
the address it observes, as in the QUIC address discovery draft. Hole punching
is coordinated over an existing relayed connection.

**Relays are homeservers.** Any reachable homeserver can relay, hold mail and
keep encrypted replicas for people its owner trusts, under the owner's
policy. Relayed traffic is end-to-end encrypted, so a relay sees only which
keys talk to each other. Official relays may exist as one more option and as
the first contact for someone with no friends on homeserver yet. An invite
link from a friend carries the friend's relay instead. Nothing stops working
without official relays. For networks that block UDP, relays also accept
connections over TLS on TCP port 443.

**Reaching your homeserver from a browser.** Browsers need an ordinary HTTPS
URL, for you and for a friend opening a link to a shared album:

1. With a public address and a domain, `hs` gets certificates itself with
   ACME. We write the ACME client on top of `ring` and hyper; `instant-acme`
   depends on `async-trait`.
2. Without a public address, a reachable homeserver (a friend's, your own
   VPS, or an official one) forwards TLS by SNI without terminating it. TLS
   still ends at home, and the forwarder sees only ciphertext.
3. On the local network, or when the web app already knows the node's key, a
   browser can use WebTransport with `serverCertificateHashes` and needs
   neither. WebTransport has been Baseline in all major browsers since March 2026.

## Encryption

- **In transit:** TLS 1.3 everywhere, with node keys between nodes, and
  signed HTTP requests on top.
- **At rest outside your nodes:** every segment, checkpoint and blob on a
  replica you do not control is encrypted by `hs` before upload (see
  Replicas).
- **Shared spaces:** end-to-end encrypted with **MLS (RFC 9420)** between the
  members' homeservers. Each person's homeserver is one MLS member, not each
  device. This avoids most of the multi-device complexity that end-to-end
  encrypted messengers have to handle. The space's home server orders MLS
  commits, just like other events. Relays, mailboxes and replicas hold only
  ciphertext.
- **Blind nodes:** a node you rent can be set to keep only ciphertext. It
  relays, orders encrypted events and stores replicas, but never sees
  plaintext.
- **Library:** `ring` only, because rustls already needs it and it builds for
  WebAssembly. Mixing crypto libraries means compiling two generations of the
  same crates; our test graph had duplicate `digest`, `sha2`, `block-buffer`
  and others. We use **mls-rs** (MIT OR Apache-2.0, maintained by AWS; 63
  crates over the base stack, against 95 for OpenMLS) with a small crypto
  provider of our own on top of `ring`.

## Personal cluster

One person or family can run several nodes under one identity: a box at
home, a VPS, a laptop. One node is the home server of the personal spaces and
orders their events. The others follow the logs, build their own indexes,
and serve reads; a blind node only keeps encrypted replicas.

The home server role moves by lease, which needs a majority of the voting
nodes. With only two voting nodes, a friend's homeserver can act as a witness
that only breaks ties and sees no data. Because every node already has the
logs, moving the role does not move data. v1 ships with a single server node
and encrypted replicas on another `hs` or a bucket.

## Plugins

Two kinds of plugin share the permission model above: a plugin gets exactly
the grants the person gives it, and the person can see and revoke them.

**WebAssembly components.** They run inside the node, sandboxed by wasmtime,
with an interface defined in WIT and built on `wasi:http`. A plugin handles
the HTTP requests routed to it, reads and writes data through the same HTTP
API as every other client, and may make outgoing requests only to the
origins it was granted. It can also run timers, subscribe to events and serve
a UI panel. Any language that compiles to WebAssembly components can be used.
Typical uses: importers, small personal tools, automations, mutual-aid
policies, and bridges to other protocols such as ActivityPub.

wasmtime with Cranelift adds 64 crates over the base stack and about 7 MB to
the release binary. With only the Pulley interpreter it adds 42 crates and
builds in about half the time. wasmi is much lighter (16 crates) but does not
support components. wasmtime depends on `async-trait`, which `deny.toml`
bans, so the ban gets an exception for wasmtime only. The guest bindings in
`hs-plugin-api` are generated once and committed, never generated in
`build.rs`.

**Connected apps.** Specialist software such as Jellyfin, Home Assistant or
Forgejo runs as its own program, installed by the person or their operating
system. homeserver is not a container platform. A small connector, itself a
WebAssembly plugin, describes the app. homeserver then provides:

- sign-in, as an OIDC client registration or through forward auth;
- reachability from anywhere, through homeserver's HTTPS and NAT traversal,
  with no ports to open;
- scoped access to homeserver data through grants, such as read-only access
  to the photo library;
- events and backup hooks.

## Clients

- **One web app.** It is served by `hs`, renders as HTML first, uses standard
  Web Components and a small bundle built in Nix like the docs, and can be
  installed.
- **Phone apps** exist for what a web app cannot do in the background, such
  as uploading new photos. They are thin native shells (Kotlin, Swift) that
  hold the device key, use the portable crates through UniFFI, and show the
  web app in a web view.
- **Photos:** the phone creates previews when it uploads, using its own
  hardware decoders. The server keeps the originals and those previews. It
  decodes JPEG, PNG and WebP in pure Rust. For HEIC and video it uses ffmpeg
  if the host has it, and otherwise relies on the client's previews.
- **Notifications:** Web Push wherever a web app can receive it. A native app
  on iOS needs Apple's push service, through a registered gateway that only
  wakes the app, carries no content, and can be run by anyone.
- **Standard protocols:** WebDAV, CalDAV/CardDAV, the Bitwarden API, OpenID
  Connect, and MCP for AI assistants. JMAP may come later.
- **AI:** the core does not bundle a model runtime. Local models (such as a
  llama.cpp server) connect as connected apps through the common
  OpenAI-style API. homeserver provides the memory, the grants and the MCP
  layer.

## Operations

**Process.** One executable and one data directory. One tokio runtime with a
few workers. Each space's log is appended under one async lock; SQLite has
one write connection and a few read connections; a blocking pool handles
hashing, encryption and media.

**Updates.** `hs` downloads a signed release (an Ed25519 signature over a
reproducible Nix build). It keeps the previous binary, restarts, and runs a
health check. If the check fails, it rolls back to the previous binary.
Updates never migrate the logs in place: new versions read old event
formats, and the index is simply rebuilt, so a rollback loses nothing. Under
systemd it uses systemd's restart. An appliance image with A/B updates comes
later.

**Recovery.** If `hs` finds the index damaged or inconsistent with the logs,
it deletes and rebuilds it, and the health page says so. A person never has
to repair a database.

**Distribution.** One static executable per platform (Linux x86_64 and
aarch64, musl), with SQLite and the web app inside; a NixOS module; and a
container image for people who already run containers. macOS builds are for
development.

**Health.** A plain-language health page says what is wrong and what to do.
Logs stay local. There is no telemetry.

## Testing

- Deterministic simulation of many nodes, networks, replicas and failures in
  one process, built on the sans-IO cores. quinn's protocol core is sans-IO
  too, so QUIC runs in the simulation. The approach follows FoundationDB and
  TigerBeetle.
- Property tests for log ordering, membership, grants and replica sync. The
  key property: an index built event by event equals an index rebuilt from
  scratch.
- Fuzzing for every parser (CBOR, WebDAV XML, WebAuthn data, ACME and S3
  responses).
- `check` builds the portable crates for `wasm32-unknown-unknown`.
- Compatibility tests against real clients and services: DAVx⁵, Thunderbird,
  the Bitwarden clients, common OIDC relying parties, and S3-compatible
  stores (the large cloud providers, Garage and MinIO, and `rclone` and the
  AWS CLI against `hs` as a server).

## Crates

Target layout. Crates are added when there is code for them, and heavy
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
| `hs-node`        | wires the crates above into one node                                              |    no    | none                 |
| `hs-net`         | QUIC, HTTP/3, path layer, relay, mailbox, ACME                                    |    no    | quinn, rustls, hyper |
| `hs-media`       | metadata, previews                                                                |    no    | image decoders       |
| `hs-plugin-host` | WebAssembly components                                                            |    no    | wasmtime             |
| `hs-plugin-api`  | Plugin Interface (WIT, guest bindings; MIT OR Apache-2.0)                         |   yes    | none                 |
| `hs-sim`         | deterministic simulation for tests                                                |    no    | none                 |
| `hs`             | the executable                                                                    |    no    | none                 |

Release 0.1.0 implements part of this; `CHANGELOG.md` in the repository lists what is not there yet.

Release 0.1.0 has no web app; people use the `hs` command line, which works
with a node through the same API every client uses. A build embeds the static
files in `HS_WEB_DIR` if it is set, and otherwise serves a page that points to
the command line.

## Dependency budget

| Stack                                                                              | Crates | Clean build                  | Release binary |
| ---------------------------------------------------------------------------------- | -----: | ---------------------------- | -------------: |
| Network base: tokio, quinn, rustls (ring), hyper, BLAKE3, Ed25519, serde, minicbor |     68 | 10 s with 4 jobs             |         0.6 MB |
| The same with rusqlite (bundled)                                                   |     77 | 10 s with 4 jobs             |         2.3 MB |
| The same with SQLx instead (sqlx-core and sqlx-sqlite, bundled)                    |    130 | 15 s with 4 jobs             |         3.0 MB |
| Base with rusqlite + wasmtime (Cranelift, components) + mls-rs + an ACME client    |    232 | 36 s with 4 jobs (dev build) |        10.2 MB |
| For comparison: an iroh endpoint alone (Linux)                                     |    231 | 40 s                         |          12 MB |

These rows were measured on one machine: separate target directories, no
compiler cache, and a program that really opens SQLite and runs a query (or
unused code is stripped). SQLx is used through `sqlx-core` and `sqlx-sqlite`,
not `sqlx`, which forces its `migrate` feature and with it SHA-2 from
RustCrypto. The row with wasmtime and the Turso comparison were measured
earlier, with rusqlite.

Candidates we measured:

| Candidate                                   | Crates over base | Decision                                                                                                           |
| ------------------------------------------- | ---------------: | ------------------------------------------------------------------------------------------------------------------ |
| SQLx (bundled)                              |               62 | Adopt, for the derived index: async, works with tokio directly                                                     |
| rusqlite (bundled)                          |                9 | Not adopted: lighter, but blocking; needs a thread pool                                                            |
| DuckDB, prebuilt                            |      71 in total | Not adopted: see Storage                                                                                           |
| tantivy + jieba-rs                          |     133 in total | Not adopted for now: a second index; FTS5 is enough for now                                                        |
| Turso                                       |    268 with base | Not adopted for now: 50 s against 13 s; look again at 1.0                                                          |
| h3 + h3-quinn                               |                9 | Adopt, for HTTP/3 between nodes                                                                                    |
| wasmtime, Cranelift                         |               64 | Adopt, for release builds                                                                                          |
| wasmtime, Pulley only                       |               42 | Build option for slow machines                                                                                     |
| wasmi                                       |      16 in total | Not adopted: no Component Model                                                                                    |
| mls-rs                                      |               63 | Adopt, with our own `ring` provider                                                                                |
| OpenMLS                                     |               95 | Not adopted: we need one MLS library; build times not compared                                                     |
| webauthn-rs                                 |              134 | Not adopted: a second crypto library (RustCrypto)                                                                  |
| openidconnect                               |              102 | Not adopted: a client library, and RustCrypto                                                                      |
| instant-acme                                |               28 | Not adopted: pulls in `async-trait`; we write ACME                                                                 |
| igd-next                                    |               33 | Not adopted: a second HTTP client; UPnP-IGD only                                                                   |
| s3s                                         |              115 | Not adopted: `async-trait` and RustCrypto; we write the subset                                                     |
| axum                                        |               14 | Not decided; a small router of our own on hyper may be enough                                                      |
| rmcp                                        |               23 | Not decided                                                                                                        |
| Automerge / Loro / Yrs                      |     13 / 80 / 17 | Decide when collaborative documents start                                                                          |
| clap (derive)                               |                7 | Adopt, for the command line: help, errors, suggestions                                                             |
| clap_complete                               |                2 | Adopt: shell completions from the same clap definitions, asking `hs` for spaces and grants; brings `is_executable` |
| time                                        |                1 | Adopt: already built for SQLx and rcgen; only `time-macros` is new                                                 |
| percent-encoding, form_urlencoded, httpdate |                0 | Adopt: already built for hyper and url                                                                             |
| data-encoding                               |                1 | Adopt, for hex and lowercase base32                                                                                |
| sfv                                         |                3 | Adopt, for the structured fields of HTTP Message Signatures                                                        |
| tar                                         |                2 | Adopt, for `hs ctl export`                                                                                         |
| unicode-width                               |                1 | Adopt, to align CLI tables that hold CJK text                                                                      |
| utoipa                                      |                2 | Adopt: derives the OpenAPI document from the API's types; serde, indexmap and syn are already built                |
| serde_urlencoded                            |                2 | Adopt: typed queries shared by the server and the client                                                           |
| rustix (`termios`)                          |                0 | Adopt: already built for tempfile; hides a typed token                                                             |
| kamadak-exif + imagesize                    |                3 | Adopt: EXIF and image sizes from JPEG, PNG, WebP and HEIF                                                          |
| crab_nat                                    |      30 in total | Not adopted: `rand` and `toml_edit` for NAT-PMP; we write it                                                       |
| coset                                       |      15 in total | Not adopted: a second CBOR library; our COSE subset is small                                                       |
| blahaj                                      |      11 in total | Not adopted: `rand` for Shamir sharing; we use `ring`                                                              |
| calcard                                     |      25 in total | Not adopted: chrono, chrono-tz and a mail parser                                                                   |
| ical                                        |       7 in total | Not adopted: second major versions of `thiserror` and `syn`                                                        |

The "Crates over base" column is context. Decisions rest on the reason in
the last column. The CI build-time budget grows only when a measured
dependency is accepted for a reason recorded in its pull request.

## Open questions

- The event, segment and checkpoint formats in detail, and how they are
  versioned for twenty years.
- Exactly what erasing a payload means for members who already received it,
  and how history retention is expressed as policy.
- Whether two-character pieces search Chinese well enough, or whether word
  segmentation is worth its size; and the same question for Japanese and
  Korean.
- Vector search for AI memory: in SQLite, in a plugin, or not at all.
- How much the browser and phone cache, and how queued writes are shown when
  they conflict.
- The DID method for the identity log.
- The resources and media types between homeservers in detail: sync,
  backpressure, versioning.
- Home server leases in the personal cluster, and how a witness takes part.
- The grant format, and the WIT world for plugins on top of `wasi:http`.
- Which CRDT library to use for collaborative documents.
- How official relays are run and paid for, without becoming a central
  dependency.
