---
title: "Identity and finding nodes"
description: How a person has one identity, and how anyone who knows it finds all of that person's nodes, with no center.
---

:::note[Status: design]
Nothing on this page is built yet. It is the plan for the next pull requests.
:::

## The goal

A person has **one identity**. Anyone who knows the identity can find
**all the nodes** of that person: a box at home, a small VPS, a laptop. No
company, server or global network is needed for this.

## Identity and node are different things

- An **identity** is a person. It is an ed25519 key, `I`.
- A **node** is a machine. It has its own ed25519 key, `N`, which is also its
  libp2p PeerId.

An identity may have many nodes, and its nodes may change. The identity does
not change.

The secret part of `I` is not on every node. It is on one device that the
owner picks. Only that device can add or take away nodes, so a node that is
broken into cannot add nodes to the identity.

### The identity folder

Every identity has one folder, made by `I`. Only `I` writes to it. It has:

| Path               | What it is                                                   |
| ------------------ | ------------------------------------------------------------ |
| `/nodes/<N>`       | A node certificate: `I` says that `N` is one of its nodes    |
| `/rendezvous`      | The rendezvous nodes of the identity, as multiaddrs          |

A **node certificate** is signed by `I`, so it is a proof by itself, outside
the log: anyone who has it can check it. To take a node away, `I` deletes its
certificate.

Which nodes belong to an identity is a fact, so the kernel checks it. The
engine is not asked. The formats are part of the contract between nodes. They
have a version number and do not change.

The nodes of an identity keep a copy of its folder, and so can people who
follow the identity. When the list of nodes or rendezvous nodes changes, they
get the change by sync.

### Identity links

A person gives their identity as a link:

```
fos:<I>?r=<multiaddr>&r=<multiaddr>
```

The `r` values are **hints**: rendezvous nodes or nodes of the identity. They
may be old. The link is still good when they are, because the other ways of
finding nodes (below) do not need them.

## Two kinds of record

Finding nodes is two questions, and they change at very different speeds:

|              | Which nodes are mine         | Where each node is now                  |
| ------------ | ---------------------------- | --------------------------------------- |
| How often    | Seldom: a new device         | Often: a new IP address, a new network  |
| Signed by    | The identity key `I`         | The node key `N`                        |
| Kept in      | The identity folder (synced) | Address records, not in any log         |

An **address record** says: node `N`, these addresses, number `seq`, good
until time `t`. It is signed by `N`. A record with a bigger `seq` takes the
place of one with a smaller `seq`. By default the addresses are relay
addresses (through Circuit Relay), not the IP address of the home.

Both kinds of record are signed. So any node can pass them on, but no node can
make false ones. The worst that a helper can do is to say nothing.

## Rendezvous nodes

A **rendezvous node** keeps address records for identities and gives them to
people who ask. It is like a Nostr relay, but much smaller.

### Why Nostr relays come together on a few servers

A Nostr relay does a lot: it keeps all content for a long time, answers
queries with any filter (so it needs a database and indexes), has feeds and
search, stops spam and keeps mailboxes. The content is on the relay, so
readers have to go where the content is. A relay costs a lot to run, and
everyone goes to the big ones.

Also, Nostr users do not run anything themselves. Their clients are thin, so
everything has to be on the servers of other people. In fileOS, everyone
already runs a node.

### What a rendezvous node does, and only that

These are rules. New features may not break them:

- **It keeps only pointers.** Small address records, about 1 KiB for each
  identity. No content, no history.
- **It looks up only by an exact key.** No query language.
- **Data is not on it.** Data is on the nodes that keep each folder. When a
  rendezvous node goes away, only one way to find people goes away. No data
  goes with it.
- **It works from memory only.** It needs no disk. After a restart, nodes
  send their records again in a few minutes.
- **It is part of `fos`.** Any node with a public address can be one, with no
  other program and no config. A node can also be only a rendezvous node.
- **It has limits.** A small number of records for each key, a size limit for
  each record, a limit on how often a node may write, and records that end.

Rendezvous is not the same job as keeping data (the `store` ability), keeping
mail for nodes that are offline, or relaying connections. A node may do any of
these jobs and not the others. Relaying is the one that costs money (bandwidth),
so it has its own limits.

Whom a rendezvous node helps is a policy. The engine says. By default, a node
with a public address is a rendezvous node for the identity of its owner and
for the nodes that its owner trusts.

### The rendezvous service of the project

The project also runs a rendezvous node, on one small server with a relay
that has limits. One small server is enough for a very large number of
identities: a million identities is about 1 GB of memory, and the signatures
of their records take about a tenth of one CPU core.

It is not special:

- It runs the normal `fos`, with the same code and protocol as any other
  rendezvous node. Its address is not in the kernel.
- It is a default in the policy, as the default engine is. A new identity has
  it in its rendezvous list. `fos id` shows it, and one command takes it away.
- The nodes of the owner come first. When an identity has a node with a public
  address, that node comes first in the list and in the hints of its link.
- When it is down, nobody is lost. People who know you already have your
  identity folder. Only people who find you for the first time need it.

## Keys that cannot be followed back

A rendezvous node must not learn which identity a record is for, and nobody
may be able to make a list of all identities. So records are not kept under
`I`. For each day (a **period** `p`, of 24 hours), the nodes of `I` make:

```
name = BLAKE3 derive_key("fos rendezvous name v1", I || p)
key  = BLAKE3 derive_key("fos rendezvous key v1",  I || p)
```

- The record is kept under `name`. The name is new every day, and it cannot
  be followed back to `I`.
- The record is encrypted with `key`, with XChaCha20-Poly1305. Inside it are
  the node certificate (signed by `I`) and the address record (signed by `N`).
- Anyone who knows `I` can make `name` and `key` and find the record. Anyone
  who does not know `I` cannot.

On the rendezvous node, each node `N` has its own place under a name. Only
`N` can change its place, because the record is signed by `N`. Someone may
put records of no value under a name, but only someone who knows `I` knows
the name, the limits keep this small, and readers throw away records that do
not check.

In the last two hours of a period, nodes also send their records for the next
period. When people look, they try the period of now and then the one before
it, in case clocks are not the same.

### Why not the key blinding of Tor

Tor onion services make a blinded key from the identity key, and records are
signed with the blinded key. That needs the secret part of `I` (or
certificates that it made earlier) for every period. In fileOS the secret
part of `I` is on one device only, and that device may be offline. With the
design above, the nodes do the work alone, and we need no new kind of
cryptography: only BLAKE3, ed25519 and XChaCha20-Poly1305, which are already
in the dependency tree.

### What this does not hide

- The rendezvous node sees which node key sends a record, and the IP address
  it comes from.
- It sees which node asks for which name. It cannot see the identity, but it
  can see that node `X` looked for the records that node `N` sent. So it sees
  links between nodes.
- Anyone who knows `I` can see your addresses. By default they are relay
  addresses, not the address of your home.

Asking through the node of a friend would hide more. That is for later.

## How a node is found

To find the nodes of identity `I`:

1. **Hints.** Ask the rendezvous nodes and nodes in the link.
2. **The rendezvous list.** Ask the rendezvous nodes in the identity folder,
   if we have a copy of it.
3. **People we know.** Ask the nodes we are connected to. A node that has
   records for `name` (as a rendezvous node, or from an earlier look) answers.

When one node of `I` answers, we have all of them: it has the identity folder
with every node certificate, and the nodes of one identity send their newest
address records to each other.

If we know only `I`, with no hints, no copy of the identity folder and no
shared rendezvous node or friend, we cannot find the nodes. In practice
people get an identity from a link, a QR code or a friend, so there are
always hints.

## The protocol

A libp2p request-response protocol, `/fos/rendezvous/1`, with CBOR messages:

- `Publish(record)`: the answer is `Ok` or `Refused(reason)`.
- `Find(name)`: the answer is the records under `name`.

The logic (records, checks, limits, what a rendezvous node keeps, the steps of
a look) is sans-IO, in a portable crate, so it runs in `fos-sim`.

## Plan

1. **Identity:** the identity key apart from the node key, the identity
   folder, node certificates, identity links, and adding a node
   (`fos id`, `fos id join`, `fos id add-node`).
2. **Address records:** nodes sign their addresses, and the nodes of one
   identity send them to each other.
3. **Rendezvous:** names and encryption by period, the rendezvous store with
   its limits, the order of a look, and the project's rendezvous node as a
   default. Tests in `fos-sim`.
4. **The project's rendezvous node:** put it on a server.
