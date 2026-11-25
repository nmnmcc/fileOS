---
title: Apps and assistants
description: Use your address book, calendar, Bitwarden and AI assistants with your homeserver.
---

Each app gets its own token from the person who runs the node
([`hs ctl token create`](../../admin/access/)). Ask for one token for each
app, limited to what that app needs. Then you can revoke (take back) that
token alone. In the examples, `NODE` is your homeserver's address.

## Contacts and calendars

Your homeserver talks CardDAV and CalDAV. So the address book and calendar
on your phone and computer can keep in step (sync) with it: DAVx⁵ on
Android, the accounts that come with iOS and macOS, Thunderbird and others.

- **Server:** `https://NODE/` (for apps that ask for a full URL:
  `https://NODE/dav/`)
- **User name:** anything
- **Password:** the token

Each space shows as one address book and one calendar. Contacts are kept as
`contact/…` items and events as `event/…` items. So they come up in search
and on the timeline like everything else.

## Passwords

Bitwarden apps and browser extensions can keep your vault (your store of
passwords) on your homeserver. The apps encrypt everything before they send
it; the node only has the encrypted data.

1. Ask the person who runs the node to open registration (see
   [Configuration](../../admin/configuration/)).
2. In the app, choose a self-hosted server and type `https://NODE/bitwarden`.
3. Make the account. There is one vault for each node.

Your master password never leaves the app. If you forget it, nobody can open
the vault: not you, and not anyone else.

## AI assistants

An assistant that talks the Model Context Protocol (MCP) can read and write
what its token lets it, and nothing more.

- **URL:** `https://NODE/mcp` (HTTP transport)
- **Header:** `Authorization: Bearer TOKEN`

A good start is a token with the `reader` role, limited to one space. You
can see everything an assistant can get to in `hs ctl token list`, and
revoke it at any time.

## Other self-hosted apps

Your homeserver can also be the sign-in for other apps that you or your
family run. The administrator sets this up; see
[Sign-in for other apps](../../admin/sign-in/).
