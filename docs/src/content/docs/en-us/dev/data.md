---
title: "Data: sync, conflicts, storage and keys"
description: How data is kept and synced with no ordering node, how conflicts are settled, how storage stays bounded, and how data is encrypted.
---

:::note[Status: built in part]
Most of this page is built, in `fileos-space`, and the node runs on it: the
data model, conflicts, strong removal, content in chunks, checkpoints,
retention and [encryption](#encryption). Not built yet: bookmarks, the
authority path, budgets and CRDT files. [Building it](#building-it-a-new-kernel) says
what is done.
:::

## Why there is no ordering node

The prototype gave each folder one ordering node, which put all writes in
one order. In a network of people, we cannot take it that any node is always
online and never moves. A laptop sleeps, a phone changes networks, a box at
home loses power. With one ordering node, nobody can write when it is away.

There is no way round this: with no node that is always there, and no
majority of nodes online, a system cannot both take writes at any time and
give every reader the newest data (the CAP theorem). We pick writes at any
time. Data is made correct by merging, not by an owner:

- **Every node can write at any time, also when offline.**
- **All nodes come to the same state** when they have the same operations,
  in any order.
- **An owner is only an aid** (for speed and less work), never what makes data
  correct.

## The data model

### Signed logs in a causal graph

- Every writer (a node or an app, by its key) has its own **append-only log**.
  Each entry is signed by the writer, names the entry before it in the same
  log by its hash, and names the entries of other logs that it depends on.
- Together the logs make a **causal graph** (a DAG): an entry comes after
  everything it names.
- An entry has a small **header** (writer, signature, place in the log,
  dependencies, the hash of the body) and a **body**. They are kept apart, so
  a body can be deleted and the chain of hashes stays good.

This is close to the format of [p2panda](https://p2panda.org): Ed25519,
BLAKE3, CBOR, one log for each author, and headers apart from bodies. We take
its format as our model.

### A space is a flat key space

Inside a space, data is a **flat set of keys**, as in S3, not a tree of
directory nodes:

- A key is a path such as `/photos/2026/a.jpg`. A "directory" is only a prefix
  that keys have in common. Listing a directory is a search by prefix.
- There is no move operation. A rename makes a new key and deletes the old
  one. This is cheap: the content is chunks named by their hash, so no data is
  copied.
- An empty directory is a marker key, if one is needed.
- Keys are UTF-8 in NFC form, with `/` between parts. They are case-sensitive.
  `.` and `..` are not allowed, and keys have a size limit.

Why not a tree: when directories can move, a concurrent move can make a cycle (A
into B while B goes into A), and every node must agree where a node was at
the time of each write and each grant. That is very hard to get right, and
it changes access without anyone asking. With a flat key space, access goes
with the path, and a path is never changed by another operation.

People still see a tree, made by views: a holder mounts the grants it has at
names it picks. A space is the large unit of sync, owners and keys, like an S3
bucket, and it changes seldom.

### Each key merges by itself

- Writes to different keys never conflict.
- Concurrent writes to the same file are kept as **versions side by side**.
  No winner is picked without the user knowing (see
  [Conflicts](#conflicts-what-people-and-developers-see)).
- A file whose content is a CRDT (see [What the kernel gives apps](#what-the-kernel-gives-apps))
  is merged by the kernel, not kept as versions.
- There are no streams and no key-value store: everything is a file. A stream
  of records (a chat, a feed) is a directory with one small file for each
  record, named so that names sort by time and never meet. Different files
  never conflict.

## Access: grants, with strong removal

### Grants of exact permissions on prefixes or files

Access is a **grant** of exact permissions (such as `files.read` or
`access.give`) on a prefix of names in a space (an "area", as in
[Willow's Meadowcap](https://willowprotocol.org/specs/meadowcap/index.html))
or on one file by its ID, to an app, a person, a device or a group. A grant can be given on to others, but only
made smaller. Every node can check a write with no network. There is no
authorization engine: rules are made by apps that manage files. The whole
model (content, files, spaces, principals, grants, views and events) is in
[The model: spaces, grants and views](../access/).

### When a removal and a write happen at the same time

This is the hardest question in access control with no ordering node. We use
the rule that [p2panda-auth](https://p2panda.org/2025/07/28/access-control.html)
calls **strong removal**:

- When someone's access is taken away, every operation they made
  **concurrently** with the removal is not valid. It stays in the graph, but it
  has no effect. Operations that depend on it are not valid either.
- It uses only causal order. No clocks, and every node gets the same answer.

Strong removal has one bad case: two managers who remove each other at the
same time are both removed, and the group may have no manager. So every
identity has an **owner root key**: the rotation key with the highest
priority in its [`did:fileos`](../identity/) DID. It can never be removed, and it
wins any conflict. Family and friends get access through groups, under it.

Strong removal is the one rule of the kernel, the same on every node.
Different rules for a set of files come from the app that manages them (see
[The model](../access/)).

**Times are never used for access.** A clock that is wrong, or a node that
lies about the time, must not be able to take or keep access.

## Consistency

### What is promised

- **All nodes come to the same state** (strong eventual consistency).
- **In one session, you always see your own writes, and you never see older
  data than you saw before** (read-your-writes, monotonic reads).
- **Between devices, a write is seen in seconds** when they are online, but
  not at once.

### Stickiness and bookmarks

We take the way of [Cloudflare D1's Sessions API](https://developers.cloudflare.com/d1/best-practices/read-replication/):

- **Stickiness:** a session stays on one node, usually the node on your
  device. That node puts its own writes into its view at once, so you see
  them.
- **Bookmarks:** each answer has a `FileOS-Bookmark` header: the heads of the
  graph that the answer was made from. The app sends it back with its next
  request. A node that has not yet seen those entries waits for them, or
  sends the request to a node that has.

### Rules that need agreement

Some rules cannot be kept by merging, because each node decides alone: a name
that must be unique, a booking that must not be made two times, a number that
must not go below zero. For these few cases there is an **authority path**:
the app says that a write must be agreed by one named node (by default, a
node of the owner). If that node cannot be reached, the write fails. It is
never kept in secret and merged later.

## Conflicts: what people and developers see

A CRDT promises that all devices end with the same data. It does not promise
that the data is what anyone wanted. A conflict is not taken away; it is
settled by a fixed rule. These still happen:

- **The same value written at the same time.** With "last writer wins", one
  write is lost. With a multi-value register, both are kept and the app
  chooses.
- **Text written at the same place at the same time.** Some algorithms mix the
  letters of the two writes. Newer ones such as
  [Fugue](https://arxiv.org/abs/2305.00583) keep this very small.
- **Moves.** Concurrent moves in a list can make two copies, and in a tree they
  can make a cycle. (A reason for flat keys.)
- **A delete at the same time as an edit.** Most often the delete wins, and the
  edit is lost.
- **Rules about meaning** (unique names, two bookings, below zero). A CRDT
  cannot see these. See [Rules that need agreement](#rules-that-need-agreement).

### Defaults

| Data                  | Default                                                                                                       |
| --------------------- | ------------------------------------------------------------------------------------------------------------- |
| Whole files           | Keep all versions. Never pick a winner in secret. The system shows "N versions" and lets you choose           |
| Data files of apps    | As for files; or a CRDT file (such as a map with last writer wins for each field) where the app wants merging |
| Documents (rich text) | A text CRDT (Fugue-based), with full history; drafts and review for written work                              |
| Rules about meaning   | The authority path, or a fix that the user can see                                                            |

### What people see: a simple page and an advanced page

The kernel's interface (`fileos.localhost`) shows conflicts and rollbacks in the
same way for every app and file:

- **The simple page** says what to do: for versions side by side, "keep mine",
  "keep theirs" or "keep both"; when data was rolled back, one sentence on why
  ("3 changes by Ann were undone because her access was taken back"); when
  changes made offline were put on top again, one sentence and a link.
- **The advanced page** is for people who want to know what happened: the full
  history and timeline of a file, by author and device; the difference between
  any two versions; going back to any version; the causal graph; and why a
  checkpoint or strong removal made an action not valid.

### What the kernel gives apps

- **Files whose content is a CRDT**, which the kernel merges: registers (last
  writer wins, and multi-value), maps, counters, text, movable lists and
  movable trees. A CRDT file is a kind of file, not a separate store. They are
  built on one library, [Loro](https://github.com/loro-dev/loro) (MIT), which
  has movable lists and trees and can cut old history. Its cost is to be
  measured before it is taken.
- **History that keeps everything** (inside the retention policy), so a write
  that "lost" can always be found.
- **One conflict interface and one system view** of versions, not one for each
  app.
- **Checks on writes** before a write is taken, and **watch events** after a
  merge, where an app can fix what its rules do not allow (see
  [The model](../access/#checks-on-writes)).
- **The authority path**, as a policy that can be changed.

## Storage that does not grow without end

Data that grows with time, and not with what people care about, is a main
reason why software is hard to run yourself. So this is a rule:
**no kind of data grows without a limit by default.** Every kind of data has
an owner, a time it is kept, and a plan for when space runs out.

| Data                   | How it is kept bounded                                                |
| ---------------------- | --------------------------------------------------------------------- |
| Content now            | Grows only with what people add                                       |
| Old versions           | Retention policy, thinner with age                                    |
| Logs, headers, deletes | Made smaller at checkpoints                                           |
| Caches and indexes     | A budget; the least used go first; never backed up; can be made again |
| Data kept for others   | A quota for each person; relay and mailbox data ends after a time     |

### Retention: thinner with age

As in Time Machine, restic's `forget --keep-…` and Syncthing's staggered
versions. The default:

- the last 24 hours: one version each hour;
- the last 30 days: one version each day;
- older: one version each week, with a limit.

It is a policy for each space. Deleted files go to a bin that empties after
a time.

### Chunks named by content

Files are cut into chunks at points that the content decides
(content-defined chunking, as in restic and borg), and each chunk is named by
its BLAKE3 hash. A new version keeps only the chunks that changed. The same
content in two files is kept once. Sync sends only the chunks that are
missing. So old versions cost about as much as the bytes that changed, not
the size of the file.

### Checkpoints: smaller logs and final history

The owner of a space (at the end of its chain of owners), or a principal
with `space.checkpoint`, signs a **checkpoint** from time to time (a policy:
for example once a day, when a device of the owner is online). When nobody who
may sign is online for a long time, checkpoints wait, and so does compaction:
data is right, it only takes more space for a time. A checkpoint says:
"the state up to these heads is final".

- **Compaction.** History before an owner-signed checkpoint becomes a
  snapshot. Other checkpoints keep the log that shows the right to sign.
  Only the entries that the retention policy keeps stay, deletes
  ("tombstones") go, and access control keeps only the members at that point,
  as Kafka does for each key. New nodes can start from the snapshot. Old bodies can be
  deleted first, and the small headers at the next checkpoint. Loro can cut
  history in the same way (shallow snapshots).
- **Finality.** An entry that comes after a checkpoint but says it is older is
  not taken. Without this, a stolen device could make writes that say they
  are old, and nobody could tell them from a late honest write.
- **A device that was offline for a long time.** Its changes that are older
  than the checkpoint are put on top again as new entries, as `git rebase`
  does, and the user is told. Where they meet other changes, the result is
  versions side by side. Nothing is lost, but nothing is merged in secret.

### A budget that people can see

- `fileos node status` says where the space goes: content, old versions,
  metadata, caches, and data kept for others.
- People can set a budget. When it is nearly full, the kernel makes room in
  this order: caches first, then thinner old versions, then no new data for
  others. Last, it tells the person. Content that the person has now is never
  deleted by the kernel.
- An app's data to keep is files in the home space; its derived data is cache
  files on each node. Cache files count against the budget and can be deleted
  at any time.

## Encryption

### Forward secrecy, and where it means something

Forward secrecy means: if a key is stolen today, older data stays safe. For
data that is **still kept**, this is not possible: whoever has today's key
can read what you can read. Systems with CRDTs, such as Keyhive, give it up
for this reason. Forward secrecy means something in three other places, and
we keep it there:

| Where                     | How                                                                                                          |
| ------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Deleted or compacted data | **The key is destroyed** with the data, so copies anywhere are of no use                                     |
| Moving over the network   | libp2p's Noise already gives it                                                                              |
| "Burn after reading" chat | A double ratchet for that conversation, if the person wants it (new devices then cannot read older messages) |

### Keys that follow prefixes

A grant can be on a prefix deep in a space, or on one file. A read grant
must give the keys of that part only. So keys follow the prefixes of names, in
the way of Cryptree (Grolimund and others) and Tahoe-LAFS:

1. **Each chunk is encrypted with a random data key.** Data keys are kept
   encrypted with the key of the prefix that the file is under (key
   wrapping).
2. **The key of a prefix is worked out from the key of the prefix above it**,
   by a one-way function. Whoever has the key of `2026/` in space `S1` can work
   out the keys of everything under it, but not the key of the whole space or
   of `private/`.
3. **A read grant gives the key of what it is on.** Mom, with a grant on
   `2026/`, gets the key of `2026/`; an app with a grant on one file gets the
   data key of that file only.
4. **When a device loses access** (a take-back, a device taken away from an
   identity, a new signing key, a member taken out of a group), the space
   gets a new **epoch**: a new
   random root key, from which all prefix keys come. The data keys of the
   files that are there now are wrapped again under it (the data is not
   encrypted again), and a new version of a file gets a new data key. The
   new keys go to everyone who still has a read grant: this is
   post-compromise security. Today the whole space changes epoch; changing
   only the part that the grant reached can come later.
5. **Delete means destroy the key** (crypto-shredding). A checkpoint keeps
   only the sealed keys that the files it keeps still need. When a file is
   deleted and the bin empties, or an old version leaves the retention
   policy, its data key and the keys of old epochs that nothing uses are
   dropped with it. Copies of the ciphertext on friends' nodes and in backups
   are then only bytes. We cannot make a friend's node delete your data;
   this makes it not matter.

Who may give access to a part may give its keys. A device that may read a
part may also give its keys to another device that may read it, such as
another node of the same person; this leaks nothing, and so a person's nodes
pass keys to each other when the owner is offline.

Which device has which key is in the open: every node can see it, and the
owner's node uses it to send the keys that readers do not have yet. The keys
are not. **Not encrypted:** the names of files, their sizes, and who wrote
what and when; a node that keeps a space checks every entry, so it must see
them. A node that only keeps a space (`data.keep`) gets no keys and sees only
ciphertext.

### Giving out keys: HPKE

The device that makes or changes a read grant sends the key of what it is on
with **HPKE** (RFC 9180) to each device of each principal that has a read grant
on it.

- With two changes at the same time, both keys are good for reading, and new
  writes use one of them by a fixed rule. No ordering node is needed.
- The cost is one HPKE message for each device. A family space of 10 people
  with 3 devices each is 30 messages, for a change that seldom happens.
- **Each device has a recipient key**, an X25519 key that it uses only to
  get keys, and shows it in each space it may read (`Act::Recipient`). Keys
  are sealed to the newest recipient key of a device, never to its ed25519
  key: that key cannot be changed, so whoever stole it later could open
  every key ever sealed to it.
- **A device changes its recipient key every 30 days** (a policy) and keeps
  only the one before it, so older secrets are destroyed. Other nodes seal
  the keys again to the new one.
- **Each epoch lasts at most 30 days** (a policy), the time deleted files
  stay in the bin. So keys of deleted data, and keys sealed to old
  recipient keys, stop being of use: a deleted file cannot be opened
  anywhere, even from copies, about 60 days at most after it was deleted.

We looked at three group key agreements:

| Protocol                                    | Why not now                                                                       |
| ------------------------------------------- | --------------------------------------------------------------------------------- |
| MLS (RFC 9420)                              | Needs exactly one commit in each epoch, so it needs an ordering node              |
| DCGKA (Weidner, Kleppmann and others, 2021) | The best fit, but its only Rust code (p2panda's) has open findings from its audit |
| BeeKEM (Keyhive)                            | A good fit, but it is not ready ("do not use in production"), with no audit       |

HPKE is simple, uses standard parts, and is enough for small groups. When
DCGKA or BeeKEM is ready and audited, we can move to it for stronger forward
secrecy. The crate is [`hpke`](https://github.com/rozbb/rust-hpke) 0.15
(pure Rust, builds for WebAssembly). There will be an outside security audit
before this is used for real data.

## Building it: a new kernel

The design on this page, [The model](../access/) and [Apps as workers](../workers/)
is too far from the prototype to change it a step at a time, and a state
between the two would mean nothing. So the kernel is written again:

- **Kept:** `fileos-core` (formats), `fileos-id` (`did:fileos`), the simulation in
  `fileos-sim` as the way to test, `.app` files, the gateway, and the `wasi:http`
  runtime.
- **New:** signed logs in a causal graph, spaces with flat names, files with
  fixed IDs, grants and views, strong removal, checks on writes, chunks,
  checkpoints, keys that follow prefixes, `wasi:filesystem` for apps, and watch.
- **Order:** first the data model in `fileos-sim`, starting from the first test,
  "a node reads its own write while it is offline"; then storage, network and
  the runtime; last, the old `fileos-fs`, `fileos-sync` and engine are taken away.
- **Done:** the data model is in the crate `fileos-space` (sans-IO, portable),
  with tests in `fileos-sim`: signed logs in a causal graph, spaces with flat
  names, files with fixed IDs and versions side by side, grants with strong
  removal, groups (owned by keys or by groups, and in groups), content in
  chunks cut where the content says, checkpoints with compaction, snapshots
  and retention, and sync between nodes. The store, the network, the node and
  the runtime use it, and the old `fileos-fs`, `fileos-sync` and engine are gone.
  Encryption is built too: keys that follow prefixes, data keys wrapped
  under them, HPKE, new epochs when access is taken back, and keys dropped
  at checkpoints. Next: checks on writes, views and `wasi:filesystem`.

## Still open

- How the interface tells people that data they already had was rolled back
  (by strong removal, or by a rebase after a checkpoint).
- How direct file writes (the command line) and app writes fit the same rules.
- A space shared by several identities: who signs its checkpoints, and whose
  nodes handle its events.
- The cost of Loro, measured with `budget`.

## Projects we learn from

| Project                                                                                       | What we take                                                    |
| --------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| [p2panda](https://p2panda.org)                                                                | The log format; strong removal (p2panda-auth)                   |
| [Keyhive](https://www.inkandswitch.com/keyhive/notebook/)                                     | The Relay access; why forward secrecy is given up for history   |
| [Willow and Meadowcap](https://willowprotocol.org)                                            | Access by path prefix; why time must not decide access          |
| [Matrix state resolution](https://spec.matrix.org/latest/rooms/v2/)                           | A rule for access in a graph, proven at scale, and its problems |
| [Cloudflare D1](https://developers.cloudflare.com/d1/best-practices/read-replication/)        | Bookmarks for consistency in a session                          |
| [Loro](https://github.com/loro-dev/loro), [Automerge](https://github.com/automerge/automerge) | CRDTs for app data                                              |
| [Upwelling](https://www.inkandswitch.com/upwelling/)                                          | Automatic merge is necessary but not enough; people review      |
| restic, borg, Time Machine, Syncthing                                                         | Chunks, and retention that gets thinner with age                |
