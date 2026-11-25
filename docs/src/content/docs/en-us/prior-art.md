---
title: Projects to learn from
description: Prior art we study before building our own, and what we take from each.
---

homeserver builds its own core: lighter, easier to extend and shaped by our
own goals. That only works if we study widely what came before: software,
protocols, papers, essays and the history of projects that tried and failed.
This page lists them by area, with what each one teaches us and, where it
matters, how we choose to go a different way.

**Learn, don't copy.** We read code and papers to understand ideas. Before
copying any code, check that its license is compatible with AGPL-3.0-only.
GPL-2.0-only code must never be copied.

To add an entry, give it a link, what to learn from it, and how we differ.

## Earlier attempts at the same vision

Others have tried to give every person their own server. Learn why each one
did not reach ordinary people.

- **[FreedomBox](https://freedombox.org/)**: a personal server on Debian,
  started after Eben Moglen's talk "Freedom in the Cloud". Shows what is
  hard about running a server at home.
- **[Solid](https://solidproject.org/)**: personal data pods with apps that
  ask for access. Learn from its data model, access control and slow
  adoption.
- **[Sandstorm](https://sandstorm.io/)**: grains, capability-based sharing and
  sandboxed apps. Learn from its design and from why it was hard to sustain.
- **[Cozy Cloud](https://github.com/cozy)**: a personal cloud with apps that
  share one data store.
- **[Urbit](https://urbit.org/)**: a personal server with its own identity
  system and networking. Learn from its ambition and its complexity. _We
  differ:_ no scarce identities and no tokens.
- **[Upspin](https://upspin.io/)**: a global namespace for personal files,
  from the Go team at Google.
- **[Mail-in-a-Box](https://mailinabox.email/)**: a self-hosted mail server you
  can set up without being an expert.
- **Helm**: a personal email server sold as hardware, which shut down in 2022.
  Shows how fragile a business built on hardware is.
- **Synology DSM** and **Apple iCloud**: closed, but they set the usability
  bar that ordinary people expect.

## Ideas and writing

- **[Local-first software](https://www.inkandswitch.com/local-first/)** (Ink &
  Switch): the ideals we share, written down well.
- **[Freedom in the Cloud](https://www.softwarefreedom.org/events/2010/isoc-ny/)**
  (Eben Moglen, 2010): the argument for personal servers.
- **_Tools for Conviviality_** (Ivan Illich): tools that serve people rather
  than make them dependent.
- **_Governing the Commons_** (Elinor Ostrom): how communities manage shared
  resources without a central authority or a market. Essential reading for
  mutual aid.
- **[Calm technology](https://calmtech.com/)**: technology that stays in the
  background until it is needed.
- **_A Pattern Language_** (Christopher Alexander): design as a shared
  vocabulary that people can use for themselves.
- **[IndieWeb](https://indieweb.org/)**: own your data, publish on your own
  site and syndicate elsewhere (POSSE). Also Webmention and Micropub.
- **[Small Technology](https://small-tech.org/)**: technology for one person,
  not for scale.
- **[Data Transfer Initiative](https://dtinit.org/)**: moving personal data
  between services, and the export formats of large platforms.

## Networking and NAT traversal

- **[iroh](https://github.com/n0-computer/iroh)**: dial a public key, not an
  IP address. We decided to write our own networking instead of depending on
  it, but it is the closest match to what `hs-net` needs, so study it in
  depth:
  - Hole punching, and falling back to a relay when a direct path fails.
  - The relay protocol over HTTPS and WebSocket, for networks that block UDP.
  - Network probing (`net_report`) and path selection across many paths.
  - Address lookup with signed DNS records (pkarr).
  - Access control for a relay (the `AccessControl` trait).

  _We differ:_ no default dependency on a company's relays or DNS, and a
  smaller dependency tree. A test build pulled in 231 crates.

- **[Tailscale](https://github.com/tailscale/tailscale)**: DERP relays,
  `magicsock`, and the essay
  [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works),
  the best introduction to the topic. _We differ:_ no central coordination
  server.
- **[quinn](https://github.com/quinn-rs/quinn)**: QUIC in Rust. Read it for
  how the protocol state machine is kept separate from I/O (`quinn-proto`).
- **[WireGuard](https://www.wireguard.com/papers/wireguard.pdf)**: a protocol
  you can read in an afternoon. Shows how much a small, fixed set of choices
  (Noise, no cipher negotiation) can do.
- **[Noise Protocol Framework](https://noiseprotocol.org/noise.html)**:
  handshake patterns for authenticated encryption between keys.
- **[libp2p](https://libp2p.io/)**: circuit relay v2, DCUtR hole punching,
  and the trouble a very general modular stack brings. _We differ:_ fewer
  options and less abstraction.
- **[Syncthing](https://syncthing.net/)**: a community relay pool, global and
  local discovery, and device IDs derived from keys. Years of work on
  peer-to-peer software that ordinary people use.
- **[Yggdrasil](https://yggdrasil-network.github.io/)** and
  **[cjdns](https://github.com/cjdelisle/cjdns)**: mesh networks where an
  address is derived from a public key.
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
- **Certificates**: [ACME](https://www.rfc-editor.org/rfc/rfc8555), which Let's
  Encrypt uses.
- **Group encryption**: [MLS](https://www.rfc-editor.org/rfc/rfc9420).
- **Mail, calendar and contacts**: [JMAP](https://jmap.io/),
  [CalDAV](https://www.rfc-editor.org/rfc/rfc4791) and
  [CardDAV](https://www.rfc-editor.org/rfc/rfc6352).
- **Sign-in**: [WebAuthn](https://www.w3.org/TR/webauthn-3/) (passkeys),
  [OpenID Connect](https://openid.net/developers/specs/) and
  [OAuth 2.1](https://oauth.net/2.1/): the protocols homeserver implements as
  an identity provider.

## Identity and trust

- **[pkarr](https://github.com/pubky/pkarr)**: DNS records signed by an
  Ed25519 key and published on the Mainline DHT. Lets a key be found without
  owning a domain.
- **[Keybase](https://book.keybase.io/docs/teams/sigchain)**: per-device keys,
  sigchains and social proofs. Shows how a person's identity can outlive any
  one device.
- **[Matrix cross-signing](https://spec.matrix.org/latest/client-server-api/#cross-signing)**:
  verifying devices and people, and how hard it is to make that usable.
- **[Secure Scuttlebutt](https://ssbc.github.io/scuttlebutt-protocol-guide/)**:
  identity is a key, and social links decide what you store and relay for
  others. The closest earlier version of "neighbors help each other". Also
  learn from its limits: append-only logs that cannot be pruned, and new
  users struggling to join.
- **[Dark Crystal](https://darkcrystal.pw/)**: recovering a lost key with the
  help of trusted friends (Shamir's secret sharing).
- **[Passkeys](https://passkeys.dev/)**: signing in without passwords, which
  ordinary people now use.
- **[Kanidm](https://kanidm.com/)**: an identity management server in Rust,
  with OpenID Connect and passkeys.
- **[Authelia](https://www.authelia.com/)** and
  **[Authentik](https://goauthentik.io/)**: the authentication gateways
  self-hosters have to maintain today. homeserver's built-in identity provider
  is meant to replace them.
- **[IndieAuth](https://indieauth.spec.indieweb.org/)**: signing in to other
  sites with your own URL as your identity.

## Encryption

- **[Signal Protocol](https://signal.org/docs/)**: the double ratchet, X3DH and
  sealed sender. The reference for private messaging.
- **[OpenMLS](https://github.com/openmls/openmls)**: MLS in Rust, for
  encrypted groups such as families.
- **[age](https://age-encryption.org/)**: simple file encryption with small,
  explicit keys.
- **[Magic Wormhole](https://magic-wormhole.readthedocs.io/)**: pairing with a
  short code using a PAKE. A model for adding a friend or a new device.
- **[CryptPad](https://cryptpad.org/)**: collaborative documents the server
  cannot read.

## Federation and social protocols

Existing federated protocols share one lesson: federation alone does not give
everyone a server of their own. Most people still gather on a few large
servers. See "Why federation alone is not enough" in the README for the
reasons and our answers.

- **[Matrix](https://spec.matrix.org/)**: room state resolution across
  servers. Despite federation, most users sit on a few large servers. Study
  why: identity is tied to the server's domain; every server in a room must
  replicate and resolve that room's whole state, so a small server joining a
  large room pays a lot; and running a server is real work. Its peer-to-peer
  experiments (Pinecone) are worth reading too. Note that "homeserver" is also
  a Matrix term.
- **[ActivityPub](https://www.w3.org/TR/activitypub/)** and Mastodon: the most
  widely used federation, built around servers rather than people. Identity
  is tied to an instance and moving loses your old posts. Small instances are
  quiet, and moderation and spam fall on the admin, so people gather on large
  ones.
- **[AT Protocol](https://atproto.com/)**: the Personal Data Server, signed
  repositories (a Merkle search tree) and moving between hosts. A PDS can be
  self-hosted, but most people's data sits on the official ones, and the relay
  and AppView are mostly run by one company. _We differ:_ no global relay or
  indexer is required.
- **[Nostr](https://github.com/nostr-protocol/nips)**: keys plus simple relays,
  and how far a minimal protocol can go. Identity belongs to no server, as in
  homeserver.
- **[Briar](https://briarproject.org/)** and its
  [Mailbox](https://briarproject.org/download-briar-mailbox/): holding messages
  for offline contacts, which is our "mailbox" role.
- **[Spritely Goblins](https://spritely.institute/goblins/)** and
  [OCapN](https://ocapn.org/): object capabilities across a network.
- **[ForgeFed](https://forgefed.org/)**: federation for code hosting, being
  built into Forgejo.

## Sync and local-first data

- **[Automerge](https://automerge.org/)**, **[Yjs](https://yjs.dev/)** and
  **[Loro](https://loro.dev/)**: CRDTs for collaborative documents. Compare
  their data formats, performance and compile cost.
- **[Willow](https://willowprotocol.org/)**: a sync protocol with capabilities
  and real deletion built in, from the Earthstar team.
- **[cr-sqlite](https://github.com/vlcn-io/cr-sqlite)**: CRDTs inside SQLite.
- **[Git](https://git-scm.com/book/en/v2/Git-Internals-Plumbing-and-Porcelain)**:
  content-addressed history that millions of people sync every day.

## Storage, databases and search

- **[SQLite](https://www.sqlite.org/)**: small, fast and reliable, with a file
  format meant to last for decades. Read
  [how it is tested](https://www.sqlite.org/testing.html).
- **[redb](https://github.com/cberner/redb)** and
  **[fjall](https://github.com/fjall-rs/fjall)**: embedded key-value stores in
  pure Rust.
- **[Tantivy](https://github.com/quickwit-oss/tantivy)** and
  [SQLite FTS5](https://www.sqlite.org/fts5.html): full-text search for every
  kind of data.
- **[Perkeep](https://perkeep.org/)**: content-addressed personal storage
  meant to last a lifetime. Very close to our goals.
- **[Tahoe-LAFS](https://tahoe-lafs.org/)**: storage among friends where the
  storage providers cannot read the data. The model for encrypted mutual
  backup.
- **[Garage](https://garagehq.deuxfleurs.fr/)**: lightweight S3 storage in
  Rust, made to be self-hosted across a few sites run by friends.
- **[restic](https://restic.net/)** and **[BorgBackup](https://www.borgbackup.org/)**:
  deduplication, encryption and repository formats for backups.
- **[BLAKE3](https://github.com/BLAKE3-team/BLAKE3)** and
  **[Bao](https://github.com/oconnor663/bao)**: fast hashing and verified
  streaming of content-addressed data.
- **[Litestream](https://litestream.io/)**: streaming SQLite replication.
- **[SQLite](https://sqlite.org/)**: the embedded database, with FTS5 and
  R*Tree, and a file format meant to stay readable until 2050. Our derived
  index uses it.
- **[DuckDB](https://duckdb.org/)** and **[DuckLake](https://ducklake.select/)**:
  an embedded analytical database, and tables kept as Parquet files in object
  storage with snapshots and time travel. We measured them and chose SQLite;
  see the architecture page.
- **[duckdb-tantivy](https://github.com/alitrack/duckdb-tantivy)**: BM25 search
  with Chinese segmentation as SQL functions in DuckDB, written in Rust.
- **[Turso](https://github.com/tursodatabase/turso)**: SQLite rewritten in
  Rust. We measured it; see the architecture page.

## The Web platform

- **[Solid](https://solidproject.org/)**: personal data stores on Web
  standards, from the Web's inventor. Close to our goals; we differ in using
  an ordered log per space and keys instead of WebIDs on domains.
- **[HTTP Message Signatures (RFC 9421)](https://www.rfc-editor.org/rfc/rfc9421)**
  and **[Web Push (RFC 8030)](https://www.rfc-editor.org/rfc/rfc8030)**:
  standards we use between servers and for notifications.
- **[WebTransport](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport)**:
  QUIC for browsers, which lets a browser reach a node by certificate hash.
- **[WinterTC](https://wintertc.org/)** and
  **[wasi:http](https://github.com/WebAssembly/wasi-http)**: one request and
  response API on servers and in browsers, the model for our plugin
  interface.

## Peer-to-peer sharing and incentives

- **[BitTorrent](https://www.bittorrent.org/beps/bep_0003.html)**: tit-for-tat,
  where peers reward those who give back. A simple local rule with large
  effects.
- **[IPFS](https://docs.ipfs.tech/)**: content addressing at scale, and the
  lesson that nobody keeps data unless someone chooses to pin it. _We differ:_
  no token economy (Filecoin).
- **[Hyphanet](https://www.hyphanet.org/)** (formerly Freenet) and its
  friend-to-friend mode: connecting only to people you trust.

## Self-hosting with zero operations

- **[Umbrel](https://umbrel.com/)**, **[YunoHost](https://yunohost.org/)** and
  **[Cloudron](https://www.cloudron.io/)**: app stores for self-hosting, and
  what goes wrong when updates break.
- **[Home Assistant](https://www.home-assistant.io/)**: a large ecosystem of
  integrations, and upgrades that ordinary people can live with. Home
  automation itself is a specialist need, connected through a plugin.
- **[Caddy](https://caddyserver.com/)**: TLS certificates that just work.
- **[NixOS](https://nixos.org/)** and **[OSTree](https://ostreedev.github.io/ostree/)**:
  atomic upgrades and rollback.
- **[Android A/B updates](https://source.android.com/docs/core/ota/ab)**,
  **[RAUC](https://rauc.io/)** and **[Mender](https://mender.io/)**: updating
  devices nobody watches, without turning them into bricks.
- **[PocketBase](https://pocketbase.io/)**: a whole backend in one file plus
  SQLite.

## Reliability and testing

- **[FoundationDB simulation testing](https://apple.github.io/foundationdb/testing.html)**:
  run the whole distributed system deterministically in one process and
  replay any failure.
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
  workspace organized for fast builds and clear boundaries.
- **[tokio](https://tokio.rs/)** and **[smol](https://github.com/smol-rs/smol)**:
  two approaches to async runtimes, large and small.
- **[hyper](https://hyper.rs/)**, **[axum](https://github.com/tokio-rs/axum)**
  and **[rustls](https://github.com/rustls/rustls)**: HTTP and TLS in Rust.

## Plugins, sandboxing and isolation

- **[Wasmtime](https://wasmtime.dev/)** and the
  [Component Model](https://component-model.bytecodealliance.org/) (WIT):
  sandboxed extensions with typed interfaces.
- **[Extism](https://extism.org/)**: a plugin system built on WebAssembly.
- **[Zed extensions](https://zed.dev/docs/extensions)**: a Rust application
  hosting WebAssembly extensions.
- **[OpenBSD pledge and unveil](https://man.openbsd.org/pledge.2)**,
  **[Linux Landlock](https://landlock.io/)** and
  **[Capsicum](https://www.cl.cam.ac.uk/research/security/capsicum/)**: a
  program giving up permissions it does not need.
- **[Fuchsia components](https://fuchsia.dev/fuchsia-src/concepts/components/v2)**:
  an operating system built on capabilities from the start.

## Clients, notifications and accessibility

- **[UnifiedPush](https://unifiedpush.org/)** and **[ntfy](https://ntfy.sh/)**:
  push notifications to phones without depending on Google or Apple.
- **[Fluent](https://projectfluent.org/)** and
  **[ICU4X](https://github.com/unicode-org/icu4x)**: translation and
  localization, both available in Rust.
- **[WCAG](https://www.w3.org/WAI/standards-guidelines/wcag/)**: accessibility
  for everyone in the family, including the elderly and the disabled.

## Local AI

- **[llama.cpp](https://github.com/ggml-org/llama.cpp)** and
  **[whisper.cpp](https://github.com/ggml-org/whisper.cpp)**: language and
  speech models that run on ordinary hardware, with no cloud.
- **Immich machine learning**: face recognition and search by meaning (CLIP),
  done on your own server.

## Applications in scope

Projects that already serve one part of what homeserver covers. Study their
features and data models, not their architecture.

Only everyday needs that 80% of people share belong here. Specialist needs
such as media libraries, home automation and code hosting stay out of the
core; see the next section.

- **[Immich](https://immich.app/)** and **[PhotoPrism](https://www.photoprism.app/)**:
  photos and videos.
- **[Nextcloud](https://nextcloud.com/)**: files, calendar and contacts. Also
  shows what happens when apps are not isolated from each other.
- **[Stalwart](https://stalw.art/)**: an all-in-one mail server in Rust (IMAP,
  JMAP, SMTP).
- **[Radicale](https://radicale.org/)**: a small calendar and contacts server.
- **[Vaultwarden](https://github.com/dani-garcia/vaultwarden)**: passwords, in
  Rust.
- **[Paperless-ngx](https://docs.paperless-ngx.com/)**: scanned documents.
- **[Actual Budget](https://actualbudget.org/)**: local-first personal
  finance.
- **[OwnTracks](https://owntracks.org/)**: private location history.
- **[Obsidian](https://obsidian.md/)** and **[Logseq](https://logseq.com/)**:
  notes kept as plain files you own.
- **[Datasette](https://datasette.io/)** and
  **[Dogsheep](https://dogsheep.github.io/)**: turning your personal data from
  many services into one place you can explore.
- **[Conduit](https://conduit.rs/)**: a lightweight Matrix server in Rust.

## Specialist software connected through plugins

Specialist needs stay out of the core and connect through plugins. Study the
APIs and integration points of these projects to learn what the plugin
interface must offer: identity, permissions, events and data access.

- **[Jellyfin](https://jellyfin.org/)**: media libraries and transcoding.
- **[Home Assistant](https://www.home-assistant.io/)**: home automation.
- **[Forgejo](https://forgejo.org/)**: code hosting.
- **[Miniflux](https://miniflux.app/)**: feed reading.

## Community and sustainability

- **[Debian Social Contract](https://www.debian.org/social_contract)**: a
  project's promises to its users, in writing.
- **[Rust RFCs](https://rust-lang.github.io/rfcs/)** and
  **[Python PEPs](https://peps.python.org/)**: deciding in the open.
- **[NLnet](https://nlnet.nl/)**: public funding for open internet software.
- **[Software Freedom Conservancy](https://sfconservancy.org/)** and the
  **[Signal Foundation](https://signalfoundation.org/)**: nonprofit homes for
  free software.
