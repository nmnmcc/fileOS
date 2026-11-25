---
title: Developer manual
description: For people who build homeserver or build on it.
---

This manual is for two kinds of developer: people who work on homeserver
itself, and people who write apps, scripts and services that talk to a
homeserver.

## Building on homeserver

You can get to everything a node does over HTTP. Every client has the same
permission model.

- [REST API](api/): the typed REST API, its OpenAPI document and the Rust
  client.
- Standard protocols, for apps that already use them: CardDAV and CalDAV,
  the Bitwarden API, Model Context Protocol, OpenID Connect and forward
  auth, Web Push, and an S3 subset. See
  [Apps and assistants](../user/apps/) and
  [Sign-in for other apps](../admin/sign-in/).
- Plugins (WebAssembly components) have a design, but they are not in
  0.1.0.

## Working on homeserver

- [Contributing](contributing/): how to set up, the rules a change must
  keep, and how to send it.
- [Architecture](architecture/): how homeserver is built, the technology we
  picked and why.
- [Projects to learn from](prior-art/): projects we study, and what we take
  from each.

Principles come first. The [manifesto](https://github.com/nmnmcc/homeserver/blob/main/MANIFESTO.md) lists them, the most
important first. We judge every change by them.
