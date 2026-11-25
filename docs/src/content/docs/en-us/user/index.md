---
title: User manual
description: What a homeserver keeps for you, and the few ideas you need to use it.
---

A homeserver keeps your notes, files, photos, contacts, calendars and
passwords on a machine that is yours, or belongs to someone you trust. This
manual is for the people who use it. To set up the node itself, see the
[administration manual](../admin/).

:::note[Version 0.1.0]
There is no web app yet. You use your homeserver with the `hs` command line
and with the apps you already have: address books, calendars, Bitwarden and
AI assistants.
:::

## Five ideas

**Identity.** You are a key, not an account on someone's server. Your
identity ID is a long string of letters and digits made from that key. It
stays the same when your homeserver goes to another machine or another
address. `hs identity` prints it as a DID document.

**Spaces.** A space is a group of things that is kept and shared as one
unit: your private space, a family space, a shared album. Each space has
members, and one node keeps it in order.

**Items.** Everything in a space is an item with a type and an ID, written
`TYPE/ID`: `note/shopping`, `contact/ada`, `event/dentist`, `file/…`. Most
items are JSON; files are kept byte for byte, unchanged.

**History.** Every change is signed and added to the end of the space's log.
Search, the timeline and places are made from that log, and you can always
make them again.

**Grants.** Every app and person gets to your data with a token. Every token
is a grant that you can list and take back (revoke). Nothing has access that
you cannot see.

## Where to go next

- [Getting started](getting-started/): sign in and look around.
- [Spaces and items](data/): add, change and remove things.
- [Finding things](finding/): search, timeline, places and links.
- [Sharing](sharing/): spaces with people on other homeservers.
- [Apps and assistants](apps/): contacts, calendars, passwords and AI.
- [Your recovery code](recovery/): the one thing you must not lose.
