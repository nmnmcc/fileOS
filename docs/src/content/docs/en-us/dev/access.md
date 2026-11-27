---
title: "The model: spaces, grants and views"
description: Seven ideas (content, files, spaces, principals, grants, views and events), what each one is, and why some things are kept apart and not made one.
---

:::note[Status: built in part]
Built: content, files, spaces, principals (keys and groups) and grants, with
strong removal, and people as principals (in `fileos-space`); views for apps,
with private data and grants given at run time from the command line. Not
built yet: the picker (an app asking for a file), sharing between apps,
checks on writes, and the `watch` options and `schedule` events.
:::

## The test for making things one

fileOS has one model, but things are made one only when they have **the same
rules, the same life and the same ways to go wrong**. When two things are
forced into one idea, that idea gets full of special cases. So this page says
both what is the same and what is kept apart, and why.

## Seven ideas

| Idea          | What it is                                                                                                  |
| ------------- | ----------------------------------------------------------------------------------------------------------- |
| **Content**   | Bytes that never change, cut into chunks named by their BLAKE3 hash                                         |
| **File**      | Something with a fixed ID that changes and has versions; its content is plain bytes or a CRDT               |
| **Space**     | The unit of sync, owner, keys, checkpoints and retention; inside, a flat set of names, each naming one file |
| **Principal** | A person, a device, a group or an app: something that acts and can be given access                          |
| **Grant**     | Access given to a principal: policies of permissions on a part of a space; kept in the space                |
| **View**      | The tree that a principal sees: the grants it has, mounted at names it picks                                |
| **Event**     | What wakes an app: an HTTP request, a change to files it watches, or a time                                 |

### Content

Content is the storage layer. Chunks are named by their hash, so the same
bytes are kept once, every chunk can be checked, sync sends only missing
chunks, and a range of a file can be read and checked (BLAKE3 is a tree hash).
Content is not the identity of a file: a file that changes keeps its identity,
and its bytes do not.

### Files

Everything that is kept is a file: people's files, the data of apps, cache
files, and each record of a chat or a feed (a directory with one file for each
record). There is no key-value store and no stream type.

- A file has a **fixed ID**: the hash of the log entry that made it, with its
  place in that entry (one entry can make many files). So the ID proves
  itself, as the ID of a space and a `did:fileos` do. A rename does not change it.
- A file has **versions**. When two devices change it at the same time, the
  versions are kept side by side, and nobody's change is lost in secret.
- A file whose content is a **CRDT** (made on [Loro](https://github.com/loro-dev/loro))
  is merged by the kernel, not kept as versions.
- A **cache file** is the same as a file, but on one node only: it does not
  sync, counts against the cache budget, and the kernel may delete it. This is
  only a place, so it is made one with files.

### Spaces

A space is what many systems have and need: Willow's namespace, a Matrix room,
a git repository, an S3 bucket, a Syncthing folder. (The prototype calls it a
"folder"; the new name keeps it apart from a directory.)

- A space is the unit of **sync, owner, keys, checkpoints and retention**. Its
  ID is the hash of its first entry, so it proves itself.
- Inside, a space is a **flat set of names**. A name can have `/` in it; a
  "directory" is only a prefix of names. Each file has one name in its space.
- The log of a space is made of the signed logs of every writer, in a causal
  graph (see [Data](../data/)).

### Principals

A principal acts; a file is acted on. They have different rules, so they are
kept apart. Inside, all principals share one model:

| Principal | Its authority                                                                         |
| --------- | ------------------------------------------------------------------------------------- |
| A person  | Its own rotation keys: a [`did:fileos`](../identity/) DID, which proves itself        |
| A device  | Its node key, with a certificate from its person                                      |
| A group   | Its owner, which is a person or another group; the chain ends at a person             |
| An app    | It belongs to a person, and acts with a key from a seed that the person's nodes share |

The state of a principal (a DID log, the members of a group) is kept and
synced as data, but it is signed by the principal's own keys and checked by
the kernel.

**A person acts through their nodes.** A grant can name a person by their
DID. A node acts for that person in a space once a **vouch** in the space
shows that it is one of the person's nodes: the operations of the DID log and
the node's certificate, signed by the signing key of the identity. A vouch
proves itself, so anyone may write it. When a node connects, it shows its
proof, and a node that has a space naming that person writes the vouch, so
the new node gets the space. To take one device back, the identity signs a
**revocation**; nodes write it into each space as an unvouch, and the old
certificate never counts again there. A new signing key ends every
certificate of the old one at once.

**Ownership is recursive.** Every space has exactly one owner, a person or a
group. Every group has exactly one owner and has members. The chain ends at a
person's owner root key (the rotation key with the highest priority, which can
never be removed). There are no cycles. An act "as a group" is valid when it is
signed by a key that leads down the chain to its owner, as with Keyhive's
nested groups; a group has no secret key of its own.

## Grants

A **grant** is the only form of access, for every kind of principal. It is
**kept in the space it is about**, so the owner can always see who has access.

- A grant is a list of **policies**. A policy is an effect (allow or deny) × a
  set of **permissions** × a set of **resources**. **Deny wins.** This is the
  shape of [Cloudflare API tokens](https://developers.cloudflare.com/fundamentals/api/get-started/create-token/).
- A resource is a **prefix** of names in the space, or **one file** (by its ID,
  so a rename does not change the grant).
- A grant says **where it came from**: the grant it was given on from, or the
  owner.

### Permissions

| Group  | Permission         | What it lets you do                                |
| ------ | ------------------ | -------------------------------------------------- |
| Files  | `files.list`       | See the names                                      |
|        | `files.read`       | Read the content, and get the keys for it          |
|        | `files.create`     | Make new files                                     |
|        | `files.change`     | Change files that are there                        |
|        | `files.delete`     | Delete files                                       |
|        | `files.history`    | Read old versions                                  |
|        | `files.watch`      | Watch for changes                                  |
| Access | `access.view`      | See the grants on the resource                     |
|        | `access.give`      | Give on what you have, never more                  |
|        | `access.take`      | Take back grants that came from yours              |
| Rules  | `rules.set`        | Put checks on writes; set the node lists of events |
| Share  | `share.send`       | Send a file to another app through the share sheet |
| Keep   | `data.keep`        | Keep and pass on the ciphertext, with no keys      |
| Space  | `space.checkpoint` | Sign checkpoints for the owner                     |

### Templates are for giving access to others

| Template       | Permissions                                                                                    |
| -------------- | ---------------------------------------------------------------------------------------------- |
| Read           | `files.list`, `files.read`, `files.watch`                                                      |
| Read and write | Read, and `files.create`, `files.change`, `files.delete`                                       |
| Manage         | Read and write, and `files.history`, `access.*`, `rules.set`, `share.send`, `space.checkpoint` |
| Relay          | `data.keep`                                                                                    |

A template is only a named set of permissions; a grant keeps the
permissions, not the name. **Apps never use templates**: they ask for exact
permissions.

### Rules

1. **Giving on:** with `access.give`, a principal gives another the same or
   less: fewer permissions, or a smaller resource.
2. **Taking back:** when a grant is taken back, the grants that came from it go
   too.
3. **Strong removal:** an act made at the same time as the taking back of the
   grant that allowed it is not valid. It stays in the graph with no effect,
   and so does what depends on it.
4. **Checking a write:** a write by principal P is valid when, in its causal
   past, there is a chain of valid grants from the owner of the space to P that
   allows it. Only grants and causal order are used, never time, so every node
   gets the same answer.
5. **Conflicts:** when two acts cannot both stand, the owner at the end of the
   chain wins.

### Checks on writes

A principal with `rules.set` (most often an app) can put **checks** on writes,
for rules that permissions cannot say. Every node must get the same answer, so,
as with CouchDB's `validate_doc_update` and Holochain's validation: a check only
says yes or no; it gets no clock, random numbers or network; it judges a write
by its causal past; its code is fixed by a hash signed into the space; when
several apply, all must say yes; and there are no checks on reads, because who
can read is decided by keys.

## Views

A **view** is the tree of files that a principal sees. It is made of
**mounts**: a grant that the principal holds, put at a name that **the
principal picks**.

Grants and mounts are kept apart on purpose. A grant is authority, which every
node must check and the owner must be able to list. A mount is only where the
holder puts it. So the giver never writes into the holder's view: the giver
makes a grant, and the holder (or the kernel, for an app) mounts it, as when an
invitation is taken.

| Whose view | What is in it                                                                                |
| ---------- | -------------------------------------------------------------------------------------------- |
| A person   | Their spaces, and what others gave them, at names they pick (`/photos/`, `/from-ming/2026/`) |
| A person   | Contacts: the people and groups they know                                                    |
| An app     | `/`: its private data; `/granted/<handle>/`: what it was given                               |

**A file can be at many paths**, in many views, while it has one name in its
space. The same file can be `/photos/2026/cat.jpg` for Ming,
`/from-ming/2026/cat.jpg` for Mom and `/granted/7Hq2/cat.jpg` for an app.

### What an app sees

- An app uses the standard **`wasi:filesystem`** (WASI 0.2); its view is given
  to it as **WASI preopened directories**. It can use the normal library of its
  language (`std::fs` in Rust on `wasm32-wasip2`).
- A **handle** is a random name that the kernel picks. It says nothing about the
  space or the name of the file. The same file has a different handle in each
  app, so apps cannot find out that they have the same file.
- The data of an app is not in the person's view by default (as Windows UWP).
  The owner can give themselves a grant, after a warning.

### Asking at run time

- **The manifest is the most an app may ask for**: exact permissions, the kind
  of resource (a file or a prefix), and an optional type hint such as
  `image/*`. It gives nothing by itself.
- **An app asks when it needs something**, by sending the person to the
  kernel's picker (a "powerbox", as in [Sandstorm](https://sandstorm.io)). The
  person picks, can untick any permission, or says no. The kernel makes the
  grant, mounts it in the app's view, and sends the person back.
- There are no system categories such as "the photo library".
- The person can take back a whole grant or single permissions at any time.

### Sharing between apps

As with Android intents: an app says in its manifest what it takes
(`[[receives]]` with an action, types and a path). An app with `share.send`
sends the person to the kernel's share sheet; the person picks the app; the
kernel gives the receiving app a grant from the sender's own (read only, one
file, until it is taken back), mounts it, and sends the person to the receiving
app's path, which is a normal `fetch` for it.

### The command line

The command line is a way in for the owner, as the picker is, and can see real
names:

```
fileos app grant <app> <path> <permissions>      for example: fileos app grant photos /photos/2026 files.read,files.create
fileos app grants [<app>]                        an app's grants, with every permission
fileos app revoke <app> <handle> [<permissions>] all of a grant, or only some permissions
fileos space grant <path> <principal> <template or permissions>
```

## Events

| Event      | What it is                                                                      |
| ---------- | ------------------------------------------------------------------------------- |
| `fetch`    | An HTTP request for the app's origin (`wasi:http`)                              |
| `watch`    | A change to files it watches, with options (see [Apps as workers](../workers/)) |
| `schedule` | A time that the manifest asks for, as Cloudflare's Cron Triggers                |

Changes to an app's grants, and messages (files that others write into a
directory where they have `files.create`), are data, so they are `watch`. A
time is not data, so it is not made into a file. Short-lived state (cursors,
"is typing") is not stored at all; it is for a separate channel of short
messages, later.

## An example

Ming owns a space `S1`. Its names are `2026/beach.jpg`, `2026/cat.jpg` and
`private/diary.md`.

1. **Ming's view** mounts `S1` at `/photos/`.
2. **Ming shares 2026 with Mom.** A grant is added to `S1`: to Mom, Read, on the
   prefix `2026/`, from the owner. Mom mounts it at `/from-ming/2026/`. She sees
   `/from-ming/2026/cat.jpg`, the same file as Ming's `/photos/2026/cat.jpg`, and
   cannot reach `private/`.
3. **The photos app asks for photos.** Ming picks `/photos/2026/` and ticks
   `files.create` too. A grant is added to `S1`: to the app, `files.list`,
   `files.read`, `files.watch` and `files.create`, on `2026/`. The kernel mounts
   it at `/granted/7Hq2/` in the app's view. The app does not know `S1` or the
   names above `2026/`. When it uploads `dog.jpg`, the name `2026/dog.jpg` is
   made in `S1`, and Ming and Mom see it.
4. **The photos app shares `cat.jpg` with the chat app.** A grant is added to
   `S1`: to the chat app, `files.read` and `files.watch`, on the file `cat.jpg`
   (by its ID), from the photos app's grant. It is mounted at
   `/granted/K9pa/cat.jpg`.
5. **Ming renames `cat.jpg` to `best/cat.jpg`.** The chat app's grant is on the
   file's ID, so it still works. Mom's grant is on the prefix `2026/`, so she no
   longer sees it. Both follow from what was given: a file, or a place.
6. **Who has access?** Ming opens the access page of `S1` and sees all the
   grants, because they are kept in `S1`.
7. **Ming takes back the photos app's grant.** The chat app's grant came from it
   and goes too. Mom's came from the owner and stays. An upload that the photos
   app made at the same time on another device is not valid.
8. **A group.** The family group's owner is Mom; Ming and Dad are members. The
   family album `S2` is owned by the group. Ming and Dad each have a Read and
   write grant on `S2` from the group. In a conflict, the owner at the end of the
   chain, Mom, wins.

## What is the same, and what is kept apart

| Made one                                     | Why                                                                    |
| -------------------------------------------- | ---------------------------------------------------------------------- |
| All stored data is files (no kv, no streams) | Same rules: sync, versions, access, retention                          |
| Synced files and cache files                 | Only the place is different                                            |
| Access for people, devices, groups and apps  | One kind of grant, the same checks                                     |
| People's trees, app handles, contacts        | All are mounts in a view                                               |
| Data changes, grant changes, messages        | All are changes to files: `watch`                                      |
| IDs of principals, spaces and files          | Each is the hash of the signed entry that made it, so it proves itself |

| Kept apart              | Why                                                                    |
| ----------------------- | ---------------------------------------------------------------------- |
| File and content        | A file keeps its identity when it changes; bytes do not                |
| Grant and mount         | Authority that every node checks, against a name that the holder picks |
| Principal and file      | One acts, one is acted on; signed and checked by different rules       |
| Space and directory     | A unit of sync, owner and keys, against a prefix of names              |
| Data, time, short-lived | Only data is stored                                                    |
