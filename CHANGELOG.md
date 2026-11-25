# Changelog

## 0.1.0 (2026-11-25)

First release: an experimental MVP (a first, minimal version) that runs the
core design from start to end in one executable. Data formats may still
change; keep an export.

### What works

- **One program, one data directory.** `hs ctl init` makes a key for a person
  (shown one time only, as a recovery code), a node key, an identity log and a
  private space. `hs ctl serve` runs the node. You do not need to install
  anything else.
- **Signed logs are the truth.** Every change is a COSE-signed, deterministic
  CBOR entry in the log of a space; the log is only added to, never changed.
  Files and payloads are BLAKE3 blobs. The SQLite index (search, places,
  timeline, links) is made from the logs, and `hs ctl rebuild-index` makes it
  again.
- **Spaces shared between nodes.** The primary of a space puts the writes in
  order. Members on other homeservers join and get new entries sent to them.
  They send their own writes to the primary, and one of them can become the
  primary. Nodes prove who they are to each other with HTTP Message
  Signatures.
- **Identity.** `did:hs` DID documents come from the identity log. You can
  give parts of the recovery code to friends with Shamir's secret sharing
  (`hs ctl recovery split` / `combine`).
- **Grants.** Tokens for apps and people, limited by role (reader, writer,
  owner), by spaces and by resource types.
- **Standard protocols:** REST and Server-Sent Events, CardDAV and CalDAV,
  Model Context Protocol, a minimal vault that works with Bitwarden, an OpenID
  Connect provider, passkeys, forward auth for other self-hosted apps, Web
  Push, and an S3-compatible endpoint that keeps a friend's replica.
- **Backups.** `hs ctl export` writes a plain tar of the data directory.
  Encrypted replicas go to a directory, any S3 service or another homeserver,
  and sync every hour. `hs ctl restore --from` gets a node back with only the
  recovery code.
- **Reachability.** HTTP/1.1 and HTTP/2, with TLS from a self-signed
  certificate or from ACME (HTTP-01, renewed automatically). NAT-PMP/PCP port
  mapping on home routers.
- **Media.** EXIF metadata (time, place, camera) and JPEG/PNG thumbnails.
- **Command line.** After `hs login`, `hs` works with a running node, here or
  in another place, through its API: spaces and members, resources (`ls`,
  `get`, `put`, `edit`, `patch`, `rm`, `upload`), search, timeline, places,
  blobs and live changes (`watch`). `hs ctl` takes care of a node: `init`,
  `serve`, `config`, `token`, `export`, `replica`, `restore`, `recovery` and
  `rebuild-index`. `hsctl`, a link to `hs`, does the same as `hs ctl`.
  `--json` prints what the API gives back. `hs completion SHELL` sets up
  completion for both in bash, zsh, fish, elvish or PowerShell. Completion
  includes spaces, grants and configuration keys from the node.
- **A described API.** The requests and responses of the REST API are shared
  Rust types (`hs_api::types`). Its OpenAPI 3.1 document is served at
  `/api/openapi.json`, printed by `hs openapi` and kept in
  `docs/openapi.json`. `hs_api::client` is a typed client, and `hs` uses it.

### Not yet

These are in the design but not in 0.1.0: a web app, QUIC and HTTP/3, the
path layer, relays and mailboxes; end-to-end encrypted spaces (MLS);
WebAssembly plugins; checkpoints and segment compaction; signed
self-updates.

### Notes

- A lock file stops two processes from writing to the same logs. Some
  commands are able to work in two ways: they use the data directory when the
  node is stopped, and its API when it runs. `config`, `restore` and
  `rebuild-index` need the node to be stopped.
- This software is based in part on the work of the Independent JPEG Group.
