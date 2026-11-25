# homeserver

**English** | [简体中文](README.zh-CN.md)

A personal server you own, for your notes, files, photos, contacts, calendars and passwords, shared straight with the homeservers of people you know.

**How it works:** every change is a signed entry added to the end of a log, and files are blobs named by their hash; search, sync, sharing and backup are all made from copying these logs and blobs between nodes.

> [!NOTE]
> Experimental. Version 0.1.0 is an MVP; data formats may still change, so keep an export.

## Features

**Your data**

- [x] Notes, files, photos, contacts, events and other items, in spaces
- [x] Search, timeline, places and links between items
- [x] Photo metadata (time, place, camera) and thumbnails
- [ ] Pages for people, days, places and projects
- [ ] Chat
- [ ] Documents that people make together
- [ ] Importers: Google Takeout, iCloud, chat exports, password managers
- [ ] A web app

**Identity and access**

- [x] Your identity is your own key (`did:hs`), with a recovery code
- [x] Recovery code in parts, kept by friends (Shamir's secret sharing)
- [x] A token for every app and person, limited by role, space and type, and able to be taken back
- [x] OpenID Connect provider, passkeys and forward auth for other self-hosted apps

**Sharing between homeservers**

- [x] Spaces shared with people on other homeservers, kept up to date as they change
- [ ] End-to-end encrypted spaces (MLS)
- [ ] Relays and mailboxes for people you trust
- [ ] QUIC and HTTP/3, with our own NAT traversal

**Works with the apps you have**

- [x] REST API with Server-Sent Events and an OpenAPI document
- [x] CardDAV and CalDAV
- [x] Bitwarden-compatible password vault
- [x] Model Context Protocol (MCP) for AI assistants
- [x] Web Push
- [ ] WebDAV for files
- [ ] WebAssembly plugins

**Backups and running a node**

- [x] One program (`hs`) and one data directory; no config file needed
- [x] Export to a plain tar file
- [x] Encrypted replicas in a directory, S3 or a friend's homeserver, synced every hour
- [x] Restore a node with only the recovery code
- [x] Port mapping on home routers (NAT-PMP/PCP)
- [ ] Signed automatic updates, going back to the old version when one goes wrong
- [ ] One identity on a number of your own nodes (home, VPS, laptop)
- [ ] Personal AI memory, and small tools made from a description

## Documentation

[User manual](docs/src/content/docs/en-us/user/index.md) · [Administration manual](docs/src/content/docs/en-us/admin/index.md) · [Developer manual](docs/src/content/docs/en-us/dev/index.md) · [Manifesto](MANIFESTO.md) · [Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md). The plugin SDK, `hs-plugin-api`, is MIT OR Apache-2.0. This software is based in part on the work of the Independent JPEG Group.
