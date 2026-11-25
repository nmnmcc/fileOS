# homeserver

**English** | [简体中文](README.zh-CN.md)

A personal server you own, for your notes, files, photos, contacts, calendars and passwords, shared straight with the homeservers of people you know.

**How it works:** every change is a signed entry added to the end of a log, and files are blobs named by their hash; search, sync, sharing and backup are all made from copying these logs and blobs between nodes.

> [!NOTE]
> Experimental. Version 0.1.0 is an MVP; data formats may still change, so keep an export.

## Features

A parent item is ticked only when everything under it is done.

- [ ] **Your data**
  - [x] Items in spaces: notes, files, photos, contacts, events, anything in JSON
    - [x] Add, change (JSON merge patch), list and read items; `hs edit` opens one in your editor
    - [x] Every change is signed and kept, so history is never lost
    - [x] Remove an item, or erase its older versions from every member's logs too
  - [x] Files and photos
    - [x] Files kept byte for byte, named by their content, so the same file is kept one time only
    - [x] Time, place and camera from the EXIF data of photos
    - [x] Thumbnails for JPEG and PNG
  - [x] Finding things
    - [x] Full-text search, with Chinese, Japanese and Korean text
    - [x] Timeline, newest first, with a day to start from
    - [x] Places: everything near a latitude and longitude
    - [x] Links: everything with a person, at a place or in a project
    - [x] Changes as they happen (`hs watch`, Server-Sent Events)
  - [ ] Ordered by your life
    - [ ] Pages for people, with every chat, photo, file and event with them
    - [ ] Pages for days, places and projects
  - [ ] Chat
  - [ ] Documents that people make together
  - [ ] Importers
    - [ ] Google Takeout
    - [ ] iCloud
    - [ ] Chat exports
    - [ ] Password managers
  - [ ] A web app
- [ ] **Identity and access**
  - [x] Identity
    - [x] Your identity is your own key, not an account on a server
    - [x] Identity log and `did:hs` DID documents
    - [x] Recovery code, shown one time, the only key to your backups
    - [x] Recovery code in parts, kept by friends (Shamir's secret sharing)
  - [x] Grants
    - [x] A token for every app and person
    - [x] Limited by role (reader, writer, owner), by space and by item type
    - [x] Tokens that end after a given time
    - [x] List every token, and take any of them back
  - [x] Sign-in for other apps
    - [x] OpenID Connect provider (authorization code flow with PKCE)
    - [x] Forward auth for apps behind Caddy, Traefik or nginx
    - [x] Passkeys
  - [ ] New devices added with a passkey, each with its own key
- [ ] **Sharing between homeservers**
  - [x] Spaces shared with people on other homeservers
    - [x] Members as reader, writer or owner
    - [x] Changes sent to members as they happen
    - [x] Members send their writes to the node that puts them in order
    - [x] Give a space to another node
    - [x] Nodes prove who they are with HTTP Message Signatures
  - [ ] End-to-end encrypted spaces (MLS, RFC 9420)
  - [ ] Mutual aid between people who trust each other
    - [x] Keep a friend's encrypted replica
    - [ ] Relays
    - [ ] Mailboxes for nodes that are offline
  - [ ] Networking
    - [x] HTTP/1.1 and HTTP/2, with a self-signed certificate or behind a reverse proxy
    - [x] Port mapping on home routers (NAT-PMP/PCP)
    - [ ] QUIC and HTTP/3
    - [ ] Our own NAT traversal, so no public IP address or domain is needed
- [ ] **Works with the apps you have**
  - [x] REST API with an OpenAPI 3.1 document and a typed Rust client
  - [x] CardDAV and CalDAV for address books and calendars
  - [x] A password vault that works with Bitwarden apps
  - [x] Model Context Protocol (MCP) for AI assistants
  - [x] Web Push notifications
  - [x] An S3-compatible endpoint for replicas
  - [ ] WebDAV for files
  - [ ] Plugins
    - [ ] WebAssembly components in a sandbox
    - [ ] Only the data you let them have, and sign-in with your identity
- [ ] **Backups**
  - [x] Export everything to a plain tar file
  - [x] Encrypted replicas, synced every hour
    - [x] To a directory (a USB disk, a NAS)
    - [x] To any S3-compatible store
    - [x] To a friend's homeserver
  - [x] Restore a node with only the recovery code
  - [x] Make the index again from the logs at any time
  - [ ] Checkpoints and compaction of old logs
- [ ] **Running a node**
  - [x] One program (`hs`) and one data directory, nothing else to install
  - [x] No config file needed; settings with `hs ctl config`
  - [x] Command line for the node (`hs ctl`) and for your data (`hs`), here or from another machine
  - [x] Completion for bash, zsh, fish, elvish and PowerShell
  - [ ] Signed automatic updates, going back to the old version when one goes wrong
  - [ ] One identity on a number of your own nodes (home, VPS, laptop)
- [ ] **Personal AI**
  - [x] Assistants get only what their token lets them have
  - [ ] A memory layer that you can see, change and delete
  - [ ] Small tools of your own, made from a description of one sentence
- [ ] **For the long term**
  - [ ] What you leave behind (digital legacy)

## Documentation

[User manual](docs/src/content/docs/en-us/user/index.md) · [Administration manual](docs/src/content/docs/en-us/admin/index.md) · [Developer manual](docs/src/content/docs/en-us/dev/index.md) · [Manifesto](MANIFESTO.md) · [Changelog](CHANGELOG.md) · [Contributing](CONTRIBUTING.md)

## License

[AGPL-3.0-only](LICENSE), with a [Plugin Exception](PLUGIN-EXCEPTION.md). The plugin SDK, `hs-plugin-api`, is MIT OR Apache-2.0. This software is based in part on the work of the Independent JPEG Group.
