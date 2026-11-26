---
title: "Access: grants, permissions and sharing"
description: One model of access for apps, people and devices; grants made of exact permissions; rules that apps keep with checks; files that apps get with no knowledge of where they are.
---

:::note[Status: design]
Nothing on this page is built yet. The prototype has an authorization engine
(a kernel plugin), and gives apps the paths of their manifest when they are
installed. This page takes the place of both.
:::

## Everything is a file, and the kernel has one rule for access

- The data of fileOS is only files in folders. There is no separate key-value
  store and no separate kind of stream: an app's data is files, and a stream
  of records is a directory with one small file for each record.
- There is **no authorization engine**. The kernel has one fixed way to check
  access, the same on every node, so that all nodes come to the same answer.
- **Rules are made by apps.** A person gives an app the right to manage a set
  of files; the app then gives and takes access, and checks writes, in its own
  way. To have different rules, use a different app.

## Grants

A **grant** is the only form of access. It is given to a **principal**: an
app, a person (an identity) or a device (a node). It is the same thing for all
three. As with [Cloudflare API tokens](https://developers.cloudflare.com/fundamentals/api/get-started/create-token/):

- A grant is a list of **policies**.
- A policy is an **effect** (allow or deny) × a set of **permissions** × a set
  of **resources**.
- **Deny wins over allow.** "All of this folder, but not `private/`" is one
  allow and one deny. The answer does not depend on order, so every node gets
  the same one.

### Permissions

A permission is the smallest unit. There is no permission made of others.

| Group  | Permission      | What it lets you do                                         |
| ------ | --------------- | ----------------------------------------------------------- |
| Files  | `files.list`    | See the names                                               |
|        | `files.read`    | Read the content                                            |
|        | `files.create`  | Make new files                                              |
|        | `files.change`  | Change files that are there                                 |
|        | `files.delete`  | Delete files                                                |
|        | `files.history` | Read old versions                                           |
|        | `files.watch`   | Get change events                                           |
| Access | `access.view`   | See who has access                                          |
|        | `access.give`   | Give others the access you have, never more                 |
|        | `access.take`   | Take back access that you gave                              |
| Rules  | `rules.set`     | Put a check on the writes (see [Checks](#checks-on-writes)) |
| Share  | `share.send`    | Send a file to another app through the share sheet          |
| Keep   | `data.keep`     | Keep and pass on the ciphertext, with no keys               |

### Resources

- The private data of an app.
- A file or a folder that the person picked. A folder has everything under it.

### Templates are for giving access to others

When a person gives access to another person or to a device, they can start
from a **template**: a named set of permissions.

| Template       | Permissions                                                                |
| -------------- | -------------------------------------------------------------------------- |
| Read           | `files.list`, `files.read`, `files.watch`                                  |
| Read and write | Read, and `files.create`, `files.change`, `files.delete`                   |
| Manage         | Read and write, and `files.history`, `access.*`, `rules.set`, `share.send` |
| Relay          | `data.keep`                                                                |

A template has no meaning of its own. "Manage" is only a set of permissions.
A grant keeps the permissions, not the name of the template, so a change to a
template later does not change grants that were given.

**Apps do not use templates.** An app asks for exact permissions, so what it
gets is exactly what it needs, and a person can see and choose each one.

### Giving on, and taking back

- With `access.give`, a principal can give on what it has, but only less of
  it: fewer permissions, or fewer resources.
- When a grant is taken back, the grants that were given on from it go with it.
- When a removal and an action happen at the same time, the action is not
  valid (strong removal; see [Data](../data/#when-a-removal-and-a-write-happen-at-the-same-time)).
  The owner root key of the identity can never be removed.

## Apps: the manifest is a limit, grants come at run time

As on iOS and Android:

- **The manifest lists the most an app may ask for**, as exact permissions and
  kinds of resource (a file or a folder), with an optional type hint such as
  `image/*`. It gives nothing by itself.
- **An app asks at run time, when it needs something.** A photos app asks only
  for `files.list`, `files.read` and `files.watch` to show photos, and asks for
  `files.create` when the person wants to upload.
- **The person decides in the kernel's picker** (a "powerbox", as in
  [Sandstorm](https://sandstorm.io)). They pick the file or folder, can untick
  any permission, or say no. They can take back the whole grant or single
  permissions at any time.
- An app cannot ask for more than its manifest.
- Grants are kept in the home folder of the identity, so a grant on one device
  is a grant on all of them.

An app is a worker with no window of its own, so asking works by moving the
browser, as sign-in does:

1. The app's page sends the person to `fos.localhost/grant?…`.
2. The person picks and chooses in the kernel's page. The app cannot make a
   false one: it is on the kernel's origin.
3. The kernel sends the person back to the app.

## Apps do not see where files are

As with Android's scoped storage and Flatpak's document portal:

- An app sees **its own view**, not the person's tree of files:
  - `/` is its private data (really `/appdata/<app>/`); it needs no grant;
  - every grant is under `/granted/<handle>/`.
- A **handle** says nothing about where the file is. An app cannot see what is
  above or next to it.
- **The same file has a different handle in each app**, so two apps cannot
  find out that they have the same file.
- Handles stay good until the grant is taken back.
- An app learns about its grants from its view: they are in `/granted/`, and
  it gets a change event when one comes or goes.

## Sharing between apps

As with Android intents:

1. An app that can take things says so in its manifest:

   ```toml
   [[receives]]
   action = "send"
   types = ["image/*"]
   path = "/receive"
   ```

2. An app with `share.send` sends the person to the kernel's **share sheet**
   with the file. The kernel lists the apps that take that type, and the person
   picks one.
3. The kernel gives the receiving app a grant (with its own handle) and sends
   the person to its `path`, with the handle. For the receiving app this is a
   normal `fetch` event.

A share is a grant like any other:

- By default it is read only (`files.list` is not needed for one file:
  `files.read` and `files.watch`), for one file, and it stays until it is taken
  back.
- It follows the rules of giving on: an app can share only what it has, and
  only as much. When the sharer's grant is taken back, the share goes too.

## Checks on writes

An app with `rules.set` on some files can put a **check** on writes to them,
for rules that permissions cannot say: a size limit, a format, who may post in
a room.

A write is synced to every node that keeps the folder, and every node must come
to the same answer about it. So a check has these limits, as CouchDB's
`validate_doc_update` and Holochain's validation have:

1. **It only says yes or no**, with a reason. It writes nothing, sends nothing,
   and has no network. Work after a write is for change events.
2. **It always gives the same answer**: it gets no clock and no random numbers,
   and it judges a write by **its causal past** (the entries it depends on), not
   by the state of a node now. Every node has the same causal past.
3. **Its code is fixed.** The hash of the check's component is written in the
   folder, signed; a new version is an operation in the folder too. A write is
   checked with the version that was in force at its place in the graph.
4. **More than one check: all must say yes.** This does not depend on order, so
   there is no chain and no `next()`, and no check can change what another
   sees.
5. **No checks on reads.** A node with `files.read` and the keys has the data;
   a check cannot stop it. Who can read is decided by keys alone.

The limits on fuel are set far above what a check needs, because different
versions of the runtime may count fuel a little differently.

Every node that keeps the folder runs the checks, so a shared folder means that
friends' nodes run the code of your app, in the sandbox. A node that does not
run them takes writes that others refuse, and its copy goes its own way. That
is its choice.

## Nodes

A node's own choices (whom it relays for, whose backups it keeps, how much
space it gives) are not access to shared files. They are **settings files**
of the node, with defaults, under `/node/`. An app with a grant on those files
can manage them, as for any other files.

## The command line

The command line is a way in for the owner, as the picker is. It can see real
paths, because the owner can see everything.

```
fos app grant <app> <path> <permissions>      for example: fos app grant photos /photos files.read,files.create
fos app grants [<app>]                        grants, with every permission
fos app revoke <app> <handle> [<permissions>] all of a grant, or only some permissions
fos folder grant <path> <identity or node> <template or permissions>
```

## One logic for all of it

| What                             | How                                               |
| -------------------------------- | ------------------------------------------------- |
| A person picks a file for an app | A grant                                           |
| An app shares with another       | A grant, by the rules of giving on                |
| The command line gives access    | A grant                                           |
| Sharing a folder with a friend   | A grant to their identity, from a template        |
| A helper node keeps your data    | A grant of `data.keep`                            |
| An app learns of a grant         | A file under `/granted/`, and a change event      |
| An app takes a share             | A `fetch` event                                   |
| Rules of an app                  | `access.give`/`access.take`, and checks on writes |
| Taking back                      | Strong removal, with what was given on            |
| The same on every device         | The home folder syncs                             |
