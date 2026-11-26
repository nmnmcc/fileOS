---
title: "Access: namespaces, links and sharing"
description: Every principal has its own namespace; a path is a link with permissions; a file can have many paths; sharing, mounts, grants and contacts are all links.
---

:::note[Status: design]
Nothing on this page is built yet. The prototype has an authorization engine
(a kernel plugin) and gives apps the paths of their manifest when they are
installed. This page takes the place of both.
:::

## The idea: a path is a capability

- **Everything is a file.** Data, the data of apps, people, devices, groups,
  contacts, settings, messages, times and short-lived state are all files and
  folders.
- **Every principal has its own namespace**: the tree of files that it sees.
  A namespace is a folder too.
- **Every path in a namespace is a link**: it points to an object and has
  permissions. **The paths by which you can reach a file, and the permissions
  on them, are your access to it.** There is no other record of access.
- **A file can have many paths.** The same file can be `/photos/cat.jpg`
  (read and write) for one person, `/from-ming/cat.jpg` (read only) for
  another, and `/granted/7Hq2/cat.jpg` for an app. A change through any path
  is a change to the same file; what each path may do is different.
- **There is no authorization engine.** The kernel has one fixed way to check
  access, the same on every node, so all nodes come to the same answer. Rules
  are made by apps that manage files.

This puts together ideas that are well tested: the per-process namespaces of
Plan 9, the capabilities of Capsicum (rights go with what you hold, not with
the file), and the read and write capabilities of Tahoe-LAFS.

## The root of every file: `/{identity}/{hash}`

- A **folder** is the unit of sync, of the log and of storage. Its hash (its
  ID) is the hash of its first entry. Inside, it is a flat key space (see
  [Data](../data/)).
- **Every file has one root path, and only one form of it:**

  ```
  /{identity}/{hash}/{key}
  ```

  `{identity}` is the DID of the identity under which the folder was made,
  `{hash}` is the hash of the folder, and `{key}` is the key in the folder. For
  example `/did:fos:ming…/F1/2026/cat.jpg`. A directory is the same with a
  prefix: `/did:fos:ming…/F1/2026/`.

- **An object is this path.** It does not change when the paths of
  namespaces change. **Every other path is a link** to it.
- The identity in the path also says where to look for the data: the nodes of
  that identity, found through [rendezvous](../identity/).

## Principals are folders

A **principal** is anything that can hold links. Every principal is a folder,
and its namespace is in it.

| Principal | Where its authority comes from                                                                  |
| --------- | ----------------------------------------------------------------------------------------------- |
| A person  | Its own rotation keys ([`did:fos`](../identity/)); its DID is the ID of its folder              |
| A device  | Its node key; the certificate in its person's folder says whose device it is                    |
| A group   | Its owner (another principal), down the chain                                                   |
| An app    | It belongs to a person; it acts with a key that comes from a seed shared by that person's nodes |

- **The DID of a person is the ID of its folder:** `did:fos:<the folder ID in
base32>`. The log of the folder is the DID log; there is no second ID.
- **What a person shows to others** (the DID log, node certificates,
  rendezvous nodes, other names) is a directory in their folder, and contacts
  get a read link to that directory only.
- **Contacts** are links in your namespace to the folders of the people and
  groups you know, for example `/people/mom/`.

## Links

A link has:

| Field      | What it is                                                       |
| ---------- | ---------------------------------------------------------------- |
| target     | An object: a file or a directory                                 |
| policies   | A list of effect (allow or deny) × permissions; **deny wins**    |
| comes from | The link it was made from (empty for the root link of an object) |
| made by    | The principal that made it                                       |

"All of `2026/`, but not `2026/raw/`" is one link with an allow and a deny.
The answer does not depend on order, so every node gets the same one. This is
the shape of [Cloudflare API tokens](https://developers.cloudflare.com/fundamentals/api/get-started/create-token/),
with the resource being the target of the link.

### Permissions

A permission is the smallest unit.

| Group  | Permission          | What it lets you do                                           |
| ------ | ------------------- | ------------------------------------------------------------- |
| Files  | `files.list`        | See the names                                                 |
|        | `files.read`        | Read the content, and get the keys for it                     |
|        | `files.create`      | Make new files                                                |
|        | `files.change`      | Change files that are there                                   |
|        | `files.delete`      | Delete files                                                  |
|        | `files.history`     | Read old versions                                             |
|        | `files.watch`       | Watch for changes                                             |
| Access | `access.view`       | See which links come from this one                            |
|        | `access.give`       | Make links from this one in other namespaces, never with more |
|        | `access.take`       | Take back links that were made from this one                  |
| Rules  | `rules.set`         | Put checks on writes; set the node lists of events            |
| Share  | `share.send`        | Send a file to another app through the share sheet            |
| Keep   | `data.keep`         | Keep and pass on the ciphertext, with no keys                 |
| Folder | `folder.checkpoint` | Sign checkpoints for the owner                                |

### Templates are for giving access to others

When a person gives access to another person or to a device, they can start
from a **template**, a named set of permissions:

| Template       | Permissions                                                                                     |
| -------------- | ----------------------------------------------------------------------------------------------- |
| Read           | `files.list`, `files.read`, `files.watch`                                                       |
| Read and write | Read, and `files.create`, `files.change`, `files.delete`                                        |
| Manage         | Read and write, and `files.history`, `access.*`, `rules.set`, `share.send`, `folder.checkpoint` |
| Relay          | `data.keep`                                                                                     |

A template has no meaning of its own; "Manage" is only a set of permissions.
A link keeps the permissions, not the name of the template. **Apps do not use
templates**: an app asks for exact permissions.

### What links are used for

| What                               | The link                                                    |
| ---------------------------------- | ----------------------------------------------------------- |
| Giving someone access              | A link in their namespace                                   |
| Mounting a folder                  | A link to a directory of that folder                        |
| An app's private data              | The root of the app's namespace, with all permissions       |
| A file or folder given to an app   | A link under `/granted/<handle>/` in the app's namespace    |
| Sharing between apps               | A link in the receiving app's namespace                     |
| A contact                          | A link to their person or group folder                      |
| A member of a group                | A link from the group's root link in the member's namespace |
| An inbox                           | A link with only `files.create`                             |
| A helper node that keeps your data | A link with only `data.keep`                                |

## Rules

1. **Owner.** Every object has exactly one owner: the principal that holds its
   root link (the one with no "comes from"), at first the one that made it. An
   owner can be a group. A group also has exactly one owner, and the chain
   ends at a person's owner root key: the rotation key with the highest
   priority, which can never be removed. Ownership has no cycles: a change
   that would make one is not taken.
2. **Giving on.** With `access.give`, a principal makes a new link from one of
   its own, in another namespace, with the same or fewer permissions and the
   same or a smaller target.
3. **Taking back.** When a link is deleted, the links made from it go with it.
4. **Strong removal.** An action made at the same time as the removal of the
   link it used is not valid. It stays in the graph, but has no effect, and
   neither does anything that depends on it.
5. **Checking a write.** Principal P may do action A on object O when, in the
   causal past of the write, there is a chain of valid links from O's owner to
   a link that P holds, and every link on the chain allows A on O. This uses
   only link files and causal order, never time, so every node gets the same
   answer.
6. **Conflicts.** When two actions cannot both stand, the owner at the end of
   the chain wins.
7. **A group acts with no key of its own.** An act "as the group" is valid when
   it is signed by a key that leads down the chain to the group's owner, as
   with Keyhive's nested groups.

## Checks on writes

A principal with `rules.set` on some files (most often an app) can put a
**check** on writes to them, for rules that permissions cannot say: a size
limit, a format, who may post in a room. Every node must come to the same
answer, so, as with CouchDB's `validate_doc_update` and Holochain's validation:

1. A check only says yes or no, with a reason. It writes nothing, sends
   nothing, and has no network.
2. It gets no clock and no random numbers, and it judges a write by its causal
   past, not by the state of a node now.
3. The hash of its component is signed into the folder; a new version is an
   operation too, and a write is checked with the version in force at its
   place in the graph.
4. When several checks apply, all must say yes. There is no chain and no
   `next()`.
5. There are no checks on reads: who can read is decided by keys alone.

## Apps

### What an app sees

- **The file interface of an app is `wasi:filesystem` (WASI 0.2).** An app can
  use the normal library of its language (for Rust on `wasm32-wasip2`,
  `std::fs`).
- **Its namespace is given to it as WASI preopened directories**: `/` (its
  private data, really `appdata/<app>/` in the person's folder) and every
  `/granted/<handle>/`.
- A handle is a random name made by the kernel. It says nothing about where
  the file is. The same file has a different handle in every app, so two apps
  cannot tell that they have the same file. A handle stays good until its link
  is taken back.

### Asking at run time

- **The manifest is the most an app may ask for**: exact permissions, the kind
  of target (a file or a folder), and an optional type hint such as `image/*`.
  It gives nothing by itself, and an app cannot ask for more.
- **An app asks when it needs something.** It sends the person to the kernel's
  picker (a "powerbox", as in [Sandstorm](https://sandstorm.io)) at
  `fos.localhost/grant?…`. The person picks a file or folder, can untick any
  permission, or says no. The kernel makes the link and sends the person back.
- The person can take back a whole link, or single permissions, at any time.
- There are no system categories such as "the photo library": only files and
  folders that the person picks.

### Sharing between apps

As with Android intents:

1. An app that can take things says so in its manifest:
   ```toml
   [[receives]]
   action = "send"
   types = ["image/*"]
   path = "/receive"
   ```
2. An app with `share.send` sends the person to the kernel's share sheet with a
   file. The kernel lists the apps that take that type; the person picks one.
3. The kernel makes a link in the receiving app's namespace (read only, one
   file, until it is taken back), from the sharing app's own link, and sends
   the person to the receiving app's `path` with the handle. For the receiving
   app this is a normal `fetch` event.

### The person's own access to app data

The data of an app is not in the person's namespace by default, as with
Windows UWP. The owner can give themselves a link (for example with
`fos folder grant`), after a warning that direct changes can break the app.
The app still gets watch events and its checks still apply.

## Nodes

A node's own choices (whom it relays for, whose backups it keeps, how much
space it gives) are settings files under `/node/`, with defaults. A principal
with a link to them can change them.

## The command line

The command line is a way in for the owner, as the picker is. It can see real
paths, because the owner can see everything.

```
fos app grant <app> <path> <permissions>      for example: fos app grant photos /photos/2026 files.read,files.create
fos app grants [<app>]                        an app's links, with every permission
fos app revoke <app> <handle> [<permissions>] all of a link, or only some permissions
fos folder grant <path> <principal> <template or permissions>
```

## An example

Ming has a folder whose hash is `F1`. Below, `ming` is short for Ming's DID,
so the root of the folder is `/ming/F1/`. Its files are
`/ming/F1/2026/beach.jpg` (O1), `/ming/F1/2026/cat.jpg` (O2) and
`/ming/F1/private/diary.md` (O3). Ming made it, so he holds the root link.

**1. Ming's namespace**

```
/photos/        → /ming/F1/    all permissions             ← the root link
/people/mom/    → Mom's folder, read (Mom gave it)
/family/        → the family group's folder, read and write
```

**2. Ming shares 2026 with Mom.** In Mom's namespace:

```
/from-ming/2026/  → /ming/F1/2026/   files.list, files.read, files.watch
                    comes from: Ming's /photos/   made by: Ming
```

Mom sees `/from-ming/2026/cat.jpg`: the same object O2 as Ming's
`/photos/2026/cat.jpg`, by another path. She cannot reach `private/`.

**3. The photos app asks for photos.** Ming picks `/photos/2026/` and also
ticks `files.create`. In the app's namespace:

```
/              → appdata/photos/ in Ming's folder   all permissions
/granted/7Hq2/ → /ming/F1/2026/   files.list, files.read, files.watch, files.create
                 comes from: Ming's /photos/
```

The app reads `/granted/7Hq2/cat.jpg` (O2) and does not know that it is
`/ming/F1/2026/cat.jpg` or under Ming's `/photos/`. When it uploads `dog.jpg`,
the file `/ming/F1/2026/dog.jpg` is made, and Ming and Mom see it at once.

**4. The photos app shares `cat.jpg` with the chat app.** In the chat app's
namespace:

```
/granted/K9pa/cat.jpg → /ming/F1/2026/cat.jpg   files.read, files.watch
                        comes from: the photos app's /granted/7Hq2/
```

O2 now has four paths:

| Principal | Path                      | Permissions               |
| --------- | ------------------------- | ------------------------- |
| Ming      | `/photos/2026/cat.jpg`    | all                       |
| Mom       | `/from-ming/2026/cat.jpg` | list, read, watch         |
| Photos    | `/granted/7Hq2/cat.jpg`   | list, read, watch, create |
| Chat      | `/granted/K9pa/cat.jpg`   | read, watch               |

**5. Checking writes.** The photos app's upload is valid: the chain Ming (the
owner) → the photos app's link has `files.create`. Mom's upload is not: her
only link has no `files.create`. The chat app cannot change `cat.jpg`: its
link has no `files.change`.

**6. Taking back.** Ming takes back the photos app's link. The chat app's link,
which came from it, goes too. Mom's link, which came from Ming's root link,
stays. An upload that the photos app made at the same time on another device
is not valid (strong removal). Both apps get watch events under `/granted/`.

**7. A group.** The family group's folder `G` has Mom as its owner and Ming and
Dad as members. The family album `F2` is owned by `G`. Ming and Dad each hold a
read-and-write link to `F2` that comes from `G`'s root link. When Ming writes,
the chain is `G` (owner: Mom) → Ming's link. In a conflict, the owner at the
end of the chain, Mom, wins.

**8. Keys.** Keys follow root paths (see [Data](../data/#keys-that-follow-paths)):
Ming has the key of `F1`; Mom and the photos app have the key of `2026/`, from
which they cannot work out the key of `private/`; the chat app has only the
key of `2026/cat.jpg`. When Ming takes back the photos app's link, only the
keys of `2026/` change, and the new ones go to Ming and Mom.

## One logic for all of it

| What                              | How                                            |
| --------------------------------- | ---------------------------------------------- |
| Access, of anyone to anything     | A link with permissions                        |
| Mounts, contacts, inboxes, groups | Links                                          |
| A person picks a file for an app  | The kernel makes a link                        |
| An app shares with another        | A link, made from the sharing app's link       |
| The command line gives access     | A link                                         |
| An app learns of its links        | Its namespace, and watch events on `/granted/` |
| Rules of an app                   | Making and taking back links, and checks       |
| Taking back                       | Deleting a link, with what came from it        |
| Who wins                          | The owner at the end of the chain              |
| Who can read                      | Who has the keys of the path                   |
