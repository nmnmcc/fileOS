# Changelog

## 0.1.0 (2026-11-25)

First release: an experimental MVP that runs the core design end to end in
one executable. Data formats may still change; keep an export.

### What works

- **One program, one data directory.** `hs ctl init` creates a person's key
  (shown once as a recovery code), a node key, an identity log and a private
  space. `hs ctl serve` runs the node. Nothing else needs to be installed.
- **Signed logs as the truth.** Every change is a COSE-signed, deterministic
  CBOR entry in a space's append-only log; files and payloads are BLAKE3
  blobs. The SQLite index (search, places, timeline, links) is derived and
  can be rebuilt with `hs ctl rebuild-index`.
- **Spaces shared between nodes.** A space's primary orders writes; members
  on other homeservers join, receive pushed entries, forward their own
  writes to the primary and can take over as primary. Requests between
  nodes are authenticated with HTTP Message Signatures.
- **Identity.** `did:hs` DID documents from the identity log; the recovery
  code can be split among friends with Shamir's secret sharing
  (`hs ctl recovery split` / `combine`).
- **Grants.** Tokens for apps and people, limited by role (reader, writer,
  owner), spaces and resource types.
- **Standard protocols:** REST and Server-Sent Events, CardDAV and CalDAV,
  Model Context Protocol, a minimal Bitwarden-compatible vault, OpenID
  Connect provider, passkeys, forward auth for other self-hosted apps, Web
  Push, and an S3-compatible endpoint for holding a friend's replica.
- **Backups.** `hs ctl export` writes a plain tar of the data directory.
  Encrypted replicas go to a directory, any S3 service or another
  homeserver, sync hourly, and `hs ctl restore --from` brings a node back with
  only the recovery code.
- **Reachability.** HTTP/1.1 and HTTP/2, with TLS from a self-signed
  certificate or ACME (HTTP-01, renewed automatically), and NAT-PMP/PCP port
  mapping on home routers.
- **Media.** EXIF metadata (time, place, camera) and JPEG/PNG thumbnails.
- **Command line.** `hs` works with a running node, here or elsewhere,
  over its API after `hs login`: spaces and members, resources (`ls`, `get`,
  `put`, `edit`, `patch`, `rm`, `upload`), search, timeline, places, blobs
  and live changes (`watch`). `hs ctl` looks after a node: `init`,
  `serve`, `config`, `token`, `export`, `replica`, `restore`, `recovery`
  and `rebuild-index`. `hsctl`, a link to `hs`, behaves as `hs ctl`. `--json` prints what the API returns. `hs completion SHELL`
  sets up completion for both in bash, zsh, fish, elvish or PowerShell,
  including spaces, grants and configuration keys from the node.
- **A described API.** The REST API's requests and responses are shared
  Rust types (`hs_api::types`); its OpenAPI 3.1 document is served at
  `/api/openapi.json`, printed by `hs openapi` and kept in
  `docs/openapi.json`. `hs_api::client` is a typed client, used by `hs`.

### Not yet

Designed but not in 0.1.0: a web app, QUIC and HTTP/3, the path
layer, relays and mailboxes; end-to-end encrypted spaces (MLS); WebAssembly
plugins; checkpoints and segment compaction; signed self-updates.

### Notes

- A lock file keeps two processes from writing the same logs. Commands that
  can work either way use the data directory while the node is stopped and
  its API while it runs; `config`, `restore` and `rebuild-index` need it
  stopped.
- This software is based in part on the work of the Independent JPEG Group.
