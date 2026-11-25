---
title: User manual
description: What a homeserver keeps for you, and the few ideas you need to use it.
---

A homeserver keeps your notes, files, photos, contacts, calendars and
passwords on a machine that belongs to you or someone you trust. This manual
is for the people who use it. Setting up the node itself is covered in the
[administration manual](../admin/).

:::note[Version 0.1.0]
There is no web app yet. You use your homeserver with the `hs` command line
and with the apps you already have: address books, calendars, Bitwarden and
AI assistants.
:::

## Five ideas

**Identity.** You are a key, not an account on someone's server. Your
identity ID, a long string of letters and digits derived from that key,
stays the same when your homeserver moves to another machine or another
address. `hs identity` prints it as a DID document.

**Spaces.** A space is a collection that is kept and shared as a whole: your
private space, a family space, a shared album. Each space has members and
one node that keeps it in order.

**Items.** Everything in a space is an item with a type and an ID, written
`TYPE/ID`: `note/shopping`, `contact/ada`, `event/dentist`, `file/…`. Most
items are JSON; files keep their original bytes.

**History.** Every change is appended to the space's log and signed. Search,
the timeline and places are built from that log and can always be rebuilt.

**Grants.** Every app and person reaches your data with a token, and every
token is a grant you can list and revoke. Nothing has access you cannot see.

## Where to go next

- [Getting started](getting-started/): sign in and look around.
- [Spaces and items](data/): add, change and remove things.
- [Finding things](finding/): search, timeline, places and links.
- [Sharing](sharing/): spaces with people on other homeservers.
- [Apps and assistants](apps/): contacts, calendars, passwords and AI.
- [Your recovery code](recovery/): the one thing you must not lose.
