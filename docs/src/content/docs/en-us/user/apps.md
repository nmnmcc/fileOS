---
title: Apps and assistants
description: Use your address book, calendar, Bitwarden and AI assistants with your homeserver.
---

Each app gets its own token from whoever runs the node
([`hs ctl token create`](../../admin/access/)). Ask for one per app, limited
to what it needs, so you can revoke it alone. In the examples, `NODE` is
your homeserver's address.

## Contacts and calendars

Your homeserver speaks CardDAV and CalDAV, so the address book and calendar
on your phone and computer can sync with it: DAVx⁵ on Android, the built-in
accounts on iOS and macOS, Thunderbird and others.

- **Server:** `https://NODE/` (apps that ask for a full URL: `https://NODE/dav/`)
- **User name:** anything
- **Password:** the token

Each space appears as one address book and one calendar. Contacts are stored
as `contact/…` items and events as `event/…` items, so they show up in
search and on the timeline like everything else.

## Passwords

Bitwarden apps and browser extensions can keep your vault on your
homeserver. The apps encrypt everything before they send it; the node only
holds ciphertext.

1. Ask whoever runs the node to open registration (see
   [Configuration](../../admin/configuration/)).
2. In the app, choose a self-hosted server and enter `https://NODE/bitwarden`.
3. Create the account. There is one vault per node.

Your master password never leaves the app. If you forget it, the vault
cannot be opened, by you or anyone else.

## AI assistants

An assistant that speaks the Model Context Protocol (MCP) can read and write
what its token allows, and nothing more.

- **URL:** `https://NODE/mcp` (HTTP transport)
- **Header:** `Authorization: Bearer TOKEN`

A token with the `reader` role, limited to one space, is a good start. You
can see everything an assistant may reach in `hs ctl token list` and revoke
it at any time.

## Other self-hosted apps

Your homeserver can also be the sign-in for other apps you or your family
run. That is set up by the administrator; see
[Sign-in for other apps](../../admin/sign-in/).
