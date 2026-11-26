---
title: Projects to learn from
description: Earlier work we study before we build our own, and what we take from each.
---

fileOS writes its own core so that it stays light, is easy to extend and
fits our goals. To do that well, we study earlier work: software, protocols,
papers, essays and projects that did not succeed. This page lists them by
area. For each one it says what to learn from it and, where it matters, how
we are different.

**Learn, don't copy.** We read code and papers to understand ideas. Before
you copy any code, make sure its license can be used with AGPL-3.0-only. Do
not copy GPL-2.0-only code.

To add an entry, give a link, what to learn from it, and how we are
different.

## Earlier attempts at the same vision

Others have tried to give every person their own server. Learn why none of
them got to ordinary people.

- **[FreedomBox](https://freedombox.org/)**: a personal server on Debian. It
  started after Eben Moglen's talk "Freedom in the Cloud". It shows what is
  hard about running a server at home.
- **[Solid](https://solidproject.org/)**: personal data pods, with apps that
  ask for access. Learn from its data model, its access control and why it
  spread so slowly.
- **[Sandstorm](https://sandstorm.io/)**: grains, sharing based on
  capabilities, and apps in a sandbox. Learn from its design, and from why it
  was hard to keep going.
- **[Cozy Cloud](https://github.com/cozy)**: a personal cloud whose apps all
  use one data store.
- **[Urbit](https://urbit.org/)**: a personal server with its own identity
  system and networking. Learn from how much it tried to do, and how complex
  that made it. _We differ:_ no scarce identities and no tokens.
- **[Upspin](https://upspin.io/)**: one global namespace for personal files,
  from the Go team at Google.
- **[Mail-in-a-Box](https://mailinabox.email/)**: a self-hosted mail server
  that you can set up without being an expert.
- **Helm**: a personal email server sold as hardware. It shut down in 2022.
  It shows the risks of a business built on hardware.
- **Synology DSM** and **Apple iCloud**: closed, but they set how easy to use
  ordinary people expect things to be.

## Ideas and writing

- **[Local-first software](https://www.inkandswitch.com/local-first/)** (Ink &
  Switch): a clear statement of principles that we share.
- **[Freedom in the Cloud](https://www.softwarefreedom.org/events/2010/isoc-ny/)**
  (Eben Moglen, 2010): the case for personal servers.
- **_Tools for Conviviality_** (Ivan Illich): tools that serve people, and do
  not make people dependent on them.
- **_Governing the Commons_** (Elinor Ostrom): how communities manage shared
  resources without a central authority and without a market. Background for
  mutual aid.
- **[Calm technology](https://calmtech.com/)**: technology that stays in the
  background until you need it.
- **_A Pattern Language_** (Christopher Alexander): design as a shared set of
  words that people can use for themselves.
- **[IndieWeb](https://indieweb.org/)**: own your data, put it out on your
  own site and send copies to other places (POSSE). Also Webmention and
  Micropub.
- **[Small Technology](https://small-tech.org/)**: technology for one person,
  not for scale.
- **[Data Transfer Initiative](https://dtinit.org/)**: moving personal data
  between services, and the export formats of large platforms.

## Networking and NAT traversal

- **[iroh](https://github.com/n0-computer/iroh)**: dial a public key, not an
  IP address. We decided to write our own networking and not depend on it.
  But it is the nearest match to what `hs-net` needs, so study it in depth:
  - Hole punching, and going back to a relay when a direct path does not
    work.
  - The relay protocol over HTTPS and WebSocket, for networks that block UDP.
  - Network probing (`net_report`), and picking one of many paths.
  - Finding addresses through signed DNS records (pkarr).
  - Access control for a relay (the `AccessControl` trait).

  _We differ:_ by default we do not depend on a company's relays or DNS, and
  our dependency tree is smaller. A test build pulled in 231 crates.

- **[Tailscale](https://github.com/tailscale/tailscale)**: DERP relays,
  `magicsock`, and the essay
  [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works),
  a good first reading on the subject. _We differ:_ no central coordination
  server.
- **[quinn](https://github.com/quinn-rs/quinn)**: QUIC in Rust. Read it to see
  how the protocol state machine is kept apart from I/O (`quinn-proto`).
- **[WireGuard](https://www.wireguard.com/papers/wireguard.pdf)**: a protocol
  you can read in an afternoon. It shows how much a small, fixed set of
  choices can do (Noise, no cipher negotiation).
- **[Noise Protocol Framework](https://noiseprotocol.org/noise.html)**:
  handshake patterns for authenticated encryption between keys.
- **[libp2p](https://libp2p.io/)**: circuit relay v2, DCUtR hole punching, and
  the costs of a very general, modular stack. _We differ:_ fewer options and
  less abstraction.
- **[Syncthing](https://syncthing.net/)**: a relay pool run by the community,
  global and local discovery, and device IDs made from keys. Many years of
  experience with peer-to-peer software that ordinary people use.
- **[Yggdrasil](https://yggdrasil-network.github.io/)** and
  **[cjdns](https://github.com/cjdelisle/cjdns)**: mesh networks where an
  address is made from a public key.
- **[Reticulum](https://reticulum.network/)** and
  **[Meshtastic](https://meshtastic.org/)**: networking over radio and other
  slow links, with no internet at all.

## Standards to read

Read the specifications themselves, not only the libraries that implement
them.

- **QUIC**: [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000) (transport) and
  [RFC 9001](https://www.rfc-editor.org/rfc/rfc9001) (TLS).
- **NAT traversal**: [ICE](https://www.rfc-editor.org/rfc/rfc8445),
  [STUN](https://www.rfc-editor.org/rfc/rfc8489),
  [TURN](https://www.rfc-editor.org/rfc/rfc8656) and port mapping with
  [PCP](https://www.rfc-editor.org/rfc/rfc6887).
- **Group encryption**: [MLS](https://www.rfc-editor.org/rfc/rfc9420).
- **Mail, calendar and contacts**: [JMAP](https://jmap.io/),
  [CalDAV](https://www.rfc-editor.org/rfc/rfc4791) and
  [CardDAV](https://www.rfc-editor.org/rfc/rfc6352).
- **Sign-in**: [WebAuthn](https://www.w3.org/TR/webauthn-3/) (passkeys),
  [OpenID Connect](https://openid.net/developers/specs/) and
  [OAuth 2.1](https://oauth.net/2.1/). The login app of fileOS will
  implement these protocols as an identity provider.

## Identity and trust

- **[pkarr](https://github.com/pubky/pkarr)**: DNS records signed by an
  Ed25519 key and put on the Mainline DHT. Others can find a key this way,
  without the owner having a domain.
- **[Keybase](https://book.keybase.io/docs/teams/sigchain)**: a key for each
  device, sigchains and social proofs. It shows how a person's identity can
  live longer than any one device.
- **[Matrix cross-signing](https://spec.matrix.org/latest/client-server-api/#cross-signing)**:
  checking devices and people, and how hard it is to make that easy to use.
- **[Secure Scuttlebutt](https://ssbc.github.io/scuttlebutt-protocol-guide/)**:
  identity is a key, and social links decide what you store and relay for
  others. It is the nearest earlier form of "neighbors help each other". Also
  learn from its limits: append-only logs that cannot be pruned, and new
  users who have a hard time joining.
- **[Dark Crystal](https://darkcrystal.pw/)**: getting back a lost key with
  the help of trusted friends (Shamir's secret sharing).
- **[Passkeys](https://passkeys.dev/)**: signing in without passwords. Ordinary
  people use them now.
- **[Kanidm](https://kanidm.com/)**: an identity management server in Rust,
  with OpenID Connect and passkeys.
- **[Authelia](https://www.authelia.com/)** and
  **[Authentik](https://goauthentik.io/)**: authentication gateways that
  self-hosters now run and keep up to date on their own. The login app of
  fileOS is meant to take their place.
- **[IndieAuth](https://indieauth.spec.indieweb.org/)**: signing in to other
  sites with your own URL as your identity.

## Encryption

- **[Signal Protocol](https://signal.org/docs/)**: the double ratchet, X3DH and
  sealed sender. The reference for private messaging.
- **[OpenMLS](https://github.com/openmls/openmls)**: MLS in Rust, for
  encrypted groups such as families.
- **[age](https://age-encryption.org/)**: simple file encryption with small,
  clear keys.
- **[Magic Wormhole](https://magic-wormhole.readthedocs.io/)**: pairing with a
  short code, using a PAKE. A model for adding a friend or a new device.
- **[CryptPad](https://cryptpad.org/)**: documents that people edit together
  and that the server cannot read.

## Federation and social protocols

The federated protocols we have today teach one lesson: federation alone does
not give everyone a server of their own. Most people still come together on a
few large servers.

- **[Matrix](https://spec.matrix.org/)**: room state resolution across
  servers. Matrix is federated, but most users are on a few large servers.
  Study why. Identity is tied to the server's domain. Every server in a room
  must copy and resolve that room's whole state, so a small server that joins
  a large room pays a lot. And running a server takes work that does not end.
  Its peer-to-peer experiments (Pinecone) are also worth reading.
- **[ActivityPub](https://www.w3.org/TR/activitypub/)** and Mastodon: the
  federation that most people use. It is built around servers, not people.
  Identity is tied to an instance, and when you move you lose your old posts.
  Small instances are quiet, and the admin has to deal with moderation and
  spam. So people come together on large instances.
- **[AT Protocol](https://atproto.com/)**: the Personal Data Server, signed
  repositories (a Merkle search tree), and moving between hosts. You can host
  a PDS yourself, but most people's data is on the official ones. One company
  runs most of the relay and AppView. _We differ:_ no global relay or indexer
  is needed.
- **[Nostr](https://github.com/nostr-protocol/nips)**: keys plus simple
  relays, and how far a very small protocol can go. Identity belongs to no
  server, as in fileOS.
- **[Briar](https://briarproject.org/)** and its
  [Mailbox](https://briarproject.org/download-briar-mailbox/): keeping
  messages for contacts who are offline. This is our "mailbox" role.
- **[Spritely Goblins](https://spritely.institute/goblins/)** and
  [OCapN](https://ocapn.org/): object capabilities across a network.
- **[ForgeFed](https://forgefed.org/)**: federation for code hosting. It is
  being built into Forgejo.

## Sync and local-first data

- **[Automerge](https://automerge.org/)**, **[Yjs](https://yjs.dev/)** and
  **[Loro](https://loro.dev/)**: CRDTs for documents that people edit
  together. Compare their data formats, speed and compile cost.
- **[Willow](https://willowprotocol.org/)**: a sync protocol from the
  Earthstar team, with capabilities and real deletion built in.
- **[cr-sqlite](https://github.com/vlcn-io/cr-sqlite)**: CRDTs inside SQLite.
- **[Git](https://git-scm.com/book/en/v2/Git-Internals-Plumbing-and-Porcelain)**:
  content-addressed history that millions of people sync every day.

## Storage, databases and search

- **[SQLite](https://www.sqlite.org/)**: a small, fast and reliable embedded
  database, with FTS5 and R*Tree. Its file format is meant to stay readable
  until 2050. Our derived index uses it. Read
  [how it is tested](https://www.sqlite.org/testing.html).
- **[redb](https://github.com/cberner/redb)** and
  **[fjall](https://github.com/fjall-rs/fjall)**: embedded key-value stores in
  pure Rust.
- **[Tantivy](https://github.com/quickwit-oss/tantivy)** and
  [SQLite FTS5](https://www.sqlite.org/fts5.html): full-text search for every
  kind of data.
- **[Perkeep](https://perkeep.org/)**: content-addressed personal storage,
  made to last a lifetime. Very near to our goals.
- **[Tahoe-LAFS](https://tahoe-lafs.org/)**: storage among friends, where the
  storage providers cannot read the data. The model for encrypted mutual
  backup.
- **[Garage](https://garagehq.deuxfleurs.fr/)**: light S3 storage in Rust,
  made to be self-hosted across a few sites that friends run.
- **[restic](https://restic.net/)** and **[BorgBackup](https://www.borgbackup.org/)**:
  deduplication, encryption and repository formats for backups.
- **[BLAKE3](https://github.com/BLAKE3-team/BLAKE3)** and
  **[Bao](https://github.com/oconnor663/bao)**: fast hashing, and verified
  streaming of content-addressed data.
- **[Litestream](https://litestream.io/)**: streaming SQLite replication.
- **[DuckDB](https://duckdb.org/)** and **[DuckLake](https://ducklake.select/)**:
  an embedded database for analysis, and tables kept as Parquet files in
  object storage, with snapshots and time travel. We measured them and chose
  SQLite; see the architecture page.
- **[duckdb-tantivy](https://github.com/alitrack/duckdb-tantivy)**: BM25 search
  with Chinese word splitting, as SQL functions in DuckDB, written in Rust.
- **[Turso](https://github.com/tursodatabase/turso)**: SQLite written again in
  Rust. We measured it; see the architecture page.

## The Web platform

- **[Solid](https://solidproject.org/)**: personal data stores on Web
  standards, from the person who made the Web. Near to our goals. We differ:
  we use an ordered log for each space, and keys in place of WebIDs on
  domains.
- **[HTTP Message Signatures (RFC 9421)](https://www.rfc-editor.org/rfc/rfc9421)**
  and **[Web Push (RFC 8030)](https://www.rfc-editor.org/rfc/rfc8030)**:
  standards we use between servers and for notifications.
- **[WebTransport](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport)**:
  QUIC for browsers. It lets a browser get to a node by certificate hash.
- **[WinterTC](https://wintertc.org/)** and
  **[wasi:http](https://github.com/WebAssembly/wasi-http)**: one request and
  response API on servers and in browsers. It is the model for our plugin
  interface.

## Peer-to-peer sharing and incentives

- **[BitTorrent](https://www.bittorrent.org/beps/bep_0003.html)**: tit-for-tat,
  where peers give more to those who give back. A simple local rule that has
  effects across the whole network.
- **[IPFS](https://docs.ipfs.tech/)**: content addressing at scale, and the
  lesson that nobody keeps data unless someone chooses to pin it. _We differ:_
  no token economy (Filecoin).
- **[Hyphanet](https://www.hyphanet.org/)** (formerly Freenet) and its
  friend-to-friend mode: connecting only to people you trust.

## Self-hosting with zero operations

- **[Umbrel](https://umbrel.com/)**, **[YunoHost](https://yunohost.org/)** and
  **[Cloudron](https://www.cloudron.io/)**: app stores for self-hosting, and
  what goes wrong when updates break things.
- **[Home Assistant](https://www.home-assistant.io/)**: a large set of
  integrations, and upgrades that ordinary people can live with. Home
  automation itself is a specialist need; it connects through a plugin.
- **[Caddy](https://caddyserver.com/)**: TLS certificates that it gets and
  renews by itself.
- **[NixOS](https://nixos.org/)** and **[OSTree](https://ostreedev.github.io/ostree/)**:
  atomic upgrades and rollback.
- **[Android A/B updates](https://source.android.com/docs/core/ota/ab)**,
  **[RAUC](https://rauc.io/)** and **[Mender](https://mender.io/)**: updating
  devices that nobody is watching, without leaving them unable to boot.
- **[PocketBase](https://pocketbase.io/)**: a whole backend in one file, plus
  SQLite.

## Reliability and testing

- **[FoundationDB simulation testing](https://apple.github.io/foundationdb/testing.html)**:
  run the whole distributed system in one process, the same way every time,
  and replay any failure.
- **[TigerBeetle](https://github.com/tigerbeetle/tigerbeetle)** and
  [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md):
  deterministic simulation, assertions everywhere, and few dependencies.
- **[Jepsen](https://jepsen.io/analyses)**: how real distributed systems lose
  data, and how to find out.
- **[Erlang/OTP](https://www.erlang.org/doc/system/design_principles.html)**:
  supervision trees, "let it crash", and systems that keep running for years.

## Rust engineering and compile time

- **[Fast Rust Builds](https://matklad.github.io/2021/09/04/fast-rust-builds.html)**
  (matklad): practical ways to keep compile times low.
- **[rust-analyzer](https://github.com/rust-lang/rust-analyzer)**: a large Rust
  workspace, organized for fast builds and clear boundaries.
- **[tokio](https://tokio.rs/)** and **[smol](https://github.com/smol-rs/smol)**:
  two ways to make an async runtime, one large and one small.
- **[hyper](https://hyper.rs/)**, **[axum](https://github.com/tokio-rs/axum)**
  and **[rustls](https://github.com/rustls/rustls)**: HTTP and TLS in Rust.

## Plugins, sandboxing and isolation

- **[Wasmtime](https://wasmtime.dev/)** and the
  [Component Model](https://component-model.bytecodealliance.org/) (WIT):
  extensions in a sandbox, with typed interfaces.
- **[Extism](https://extism.org/)**: a plugin system built on WebAssembly.
- **[Zed extensions](https://zed.dev/docs/extensions)**: a Rust application
  that runs WebAssembly extensions.
- **[OpenBSD pledge and unveil](https://man.openbsd.org/pledge.2)**,
  **[Linux Landlock](https://landlock.io/)** and
  **[Capsicum](https://www.cl.cam.ac.uk/research/security/capsicum/)**: a
  program gives up the permissions it does not need.
- **[Fuchsia components](https://fuchsia.dev/fuchsia-src/concepts/components/v2)**:
  an operating system built on capabilities from the start.

## Clients, notifications and accessibility

- **[UnifiedPush](https://unifiedpush.org/)** and **[ntfy](https://ntfy.sh/)**:
  push notifications to phones without depending on Google or Apple.
- **[Fluent](https://projectfluent.org/)** and
  **[ICU4X](https://github.com/unicode-org/icu4x)**: translation and
  localization. Both can be used from Rust.
- **[WCAG](https://www.w3.org/WAI/standards-guidelines/wcag/)**: accessibility
  for everyone in the family, old people and disabled people too.

## Local AI

- **[llama.cpp](https://github.com/ggml-org/llama.cpp)** and
  **[whisper.cpp](https://github.com/ggml-org/whisper.cpp)**: language and
  speech models that run on ordinary hardware, with no cloud.
- **Immich machine learning**: face recognition and search by meaning (CLIP),
  done on your own server.

## Applications in scope

These projects already do one part of what fileOS does. Study their
features and data models, not their architecture.

Only everyday needs that 80% of people have go here. Specialist needs, such
as media libraries, home automation and code hosting, stay out of the core;
see the next section.

- **[Immich](https://immich.app/)** and **[PhotoPrism](https://www.photoprism.app/)**:
  photos and videos.
- **[Nextcloud](https://nextcloud.com/)**: files, calendar and contacts. It
  also shows what happens when apps are not kept apart from each other.
- **[Stalwart](https://stalw.art/)**: a mail server in Rust that does it all
  (IMAP, JMAP, SMTP).
- **[Radicale](https://radicale.org/)**: a small calendar and contacts server.
- **[Vaultwarden](https://github.com/dani-garcia/vaultwarden)**: passwords, in
  Rust.
- **[Paperless-ngx](https://docs.paperless-ngx.com/)**: scanned documents.
- **[Actual Budget](https://actualbudget.org/)**: local-first personal
  finance.
- **[OwnTracks](https://owntracks.org/)**: private location history.
- **[Obsidian](https://obsidian.md/)** and **[Logseq](https://logseq.com/)**:
  notes kept as plain files that you own.
- **[Datasette](https://datasette.io/)** and
  **[Dogsheep](https://dogsheep.github.io/)**: putting your personal data from
  many services in one place where you can look through it.
- **[Conduit](https://conduit.rs/)**: a light Matrix server in Rust.

## Specialist software connected through plugins

Specialist needs stay out of the core and connect through plugins. Study the
APIs and integration points of these projects. They show what the plugin
interface must give: identity, permissions, events and data access.

- **[Jellyfin](https://jellyfin.org/)**: media libraries and transcoding.
- **[Home Assistant](https://www.home-assistant.io/)**: home automation.
- **[Forgejo](https://forgejo.org/)**: code hosting.
- **[Miniflux](https://miniflux.app/)**: reading feeds.

## Community and sustainability

- **[Debian Social Contract](https://www.debian.org/social_contract)**: a
  project's promises to its users, in writing.
- **[Rust RFCs](https://rust-lang.github.io/rfcs/)** and
  **[Python PEPs](https://peps.python.org/)**: making decisions in the open.
- **[NLnet](https://nlnet.nl/)**: public money for open internet software.
- **[Software Freedom Conservancy](https://sfconservancy.org/)** and the
  **[Signal Foundation](https://signalfoundation.org/)**: nonprofit homes for
  free software.
