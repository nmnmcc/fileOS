---
title: "Identity and finding nodes"
description: How a person has one identity, and how anyone who knows it finds all of that person's nodes, with no center.
---

:::note[Status: design, partly built]
`did:fileos`, the identity space, node certificates and identity links are
built (step 1 of the [plan](#plan)). The rest is not built yet.
:::

## The goal

A person has **one identity**. Anyone who knows the identity can find
**all the nodes** of that person: a box at home, a small VPS, a laptop. No
company, server or global network is needed for this.

## Identity and node are different things

- An **identity** is a person. It is a [W3C DID](https://www.w3.org/TR/did-core/)
  of the method `did:fileos`, for example `did:fileos:4kq…`.
- A **node** is a machine. It has its own ed25519 key, `N`, which is also its
  libp2p PeerId.

An identity may have many nodes, and its nodes may change. The keys of the
identity may change too. The DID does not change.

## did:fileos: the did:plc method, with no directory

We do not make a new kind of identity. `did:fileos` is the
[`did:plc`](https://web.plc.directory/spec/v0.1/did-plc) method of AT
Protocol, which millions of Bluesky accounts use, and we keep its rules:

- **The DID is the hash of the first operation of a signed log.** No one can
  make a different first operation with the same hash, so the DID proves
  itself.
- **Each operation gives the whole state of the identity:**
  - `rotationKeys`: 1 to 5 keys that control the identity. The first one has
    the highest priority.
  - `verificationMethods`: keys for other work, by name. The key `fileos` is the
    **signing key** of the identity in fileOS.
  - `alsoKnownAs`: other names of the identity, such as a domain (see
    [Names](#names-a-domain-as-another-name)).
  - `services`: where to find things. `fileos_identity` names the identity space.
- **Every operation after the first is signed by a rotation key of the state
  before it**, and names the operation before it by its hash.
- **Keys are `did:key`**, and an operation has at most 7500 bytes and 10
  verification methods.
- **Rotation keys and the signing key are different keys**, as in AT Protocol.
  A rotation key can change everything, so it may be kept offline, for
  example on paper. The signing key does the daily work, on one node.

In fileOS, this log is kept in the identity space and synced like other
data. Every node checks it with the keys in the log itself, so no one else
has to be trusted. A person is a **principal**, not a space or a file: the
DID names the person, and the spaces they own have IDs of their own (see
[The model](../access/)).

We change three things, each for a reason:

| `did:plc`                                                                                                               | `did:fileos`                                                                                                                                                                                          | Why                                                                                                                                          |
| ----------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Rotation keys are secp256k1 or P-256                                                                                    | All keys are ed25519; the hash is BLAKE3                                                                                                                                                           | These are the key and hash of the rest of fileOS                                                                                             |
| The DID is the first 120 bits of the hash                                                                               | The DID is the full hash (52 characters of base32)                                                                                                                                                 | Names are for people; the DID is for machines, so it does not need to be short                                                               |
| A directory server (plc.directory) keeps the log, puts forks in order, and gives the time for a 72-hour recovery window | The log is in the identity space, on the nodes of the identity and of people who know it. At a fork, the branch whose first operation was signed by the rotation key with the higher priority wins | No center. A higher key can always take back what a lower key did. There is no time window, because no clock can be trusted without a server |

The DID log is a fact that every node can check, so the kernel checks it.
The formats are part of the contract between nodes.

### The identity space

Every identity has one space, owned by the signing key, so only the signing
key writes to it. It has:

| Path          | What it is                                                                  |
| ------------- | --------------------------------------------------------------------------- |
| `/did/<hash>` | The operations of the DID log, each by its hash                             |
| `/nodes/<N>`  | A node certificate: the signing key says that `N` is a node of the identity |

The operations check themselves. The signing key could delete one, but it
cannot make one, because only rotation keys sign operations.

A **node certificate** names the DID and the node, and is signed by the
signing key. It is good while that key is the `fileos` key of the identity. To
take a node away, the signing key deletes its certificate.

The nodes of an identity keep a copy of its space, and so can people who
follow the identity. On a node, the identity space is mounted at
`/identity`.

When a signing key is lost or stolen, a rotation key writes an operation with
a new signing key. Because the owner of a space does not change, the new
signing key makes a new identity space, and the operation names it.

### Making an identity and adding a node

1. `fileos identity new` makes a rotation key and a signing key on this node, the
   identity space, and the first operation. It prints the DID.
2. On the node that holds the signing key: `fileos identity add-node <node key>`. It
   puts the certificate of the node into the identity space, lets the node
   read and keep the space, and prints the ID of the space.
3. On the new node: `fileos identity join <space ID>`. It mounts the space at
   `/identity`, and gets it from the first node.

`fileos identity remove-node <node key>` deletes the certificate, takes back the
node's grant on the identity space, and puts a revocation signed by the
signing key at `/revoked/<node>`. Every node of the identity writes the
revocation into the spaces it has, so the removed node loses what it had as a
node of the person, everywhere. To add it again, the identity needs a new
signing key.

### A grant to a person

`fileos space grant /family did:fileos:… read-write` gives the permissions to the
person, so to every node of their identity, also to nodes added later. Each
node shows its certificate when it connects; a node that has the space writes
a vouch for it (see [The model](../access/#principals)). `fileos identity
remove-node` takes one device away from every space; a new signing key takes
all old certificates away at once.

### Identity links

A person gives their identity as a DID URL with hints:

```
did:fileos:<id>?r=<multiaddr>&r=<multiaddr>
```

The `r` values are **hints**: rendezvous nodes or nodes of the identity. They
may be old. The link is still good when they are, because the other ways of
finding nodes (below) do not need them.

### Names: a domain as another name

We take the handles of AT Protocol as they are. A person who has a domain,
such as `alice.example`, can use it as the name of their identity:

- The domain points to the DID, with a DNS TXT record at `_fileos.alice.example`
  with the text `did=did:fileos:…`, or with the file
  `https://alice.example/.well-known/fileos-did`.
- The DID points back to the domain: `alsoKnownAs` has `fileos://alice.example`.
- A name is good only when both point to each other.

The only change is the names of the record and the file (`_fileos` and
`fileos-did`, not `_atproto` and `atproto-did`), because one domain may also be
the handle of a Bluesky account, which has a different DID.

## Two kinds of record

Finding nodes is two questions, and they change at very different speeds:

|           | Which nodes are mine            | Where each node is now                 |
| --------- | ------------------------------- | -------------------------------------- |
| How often | Seldom: a new device            | Often: a new IP address, a new network |
| Signed by | The signing key of the identity | The node key `N`                       |
| Kept in   | The identity space (synced)     | Address records, not in any log        |

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
- **Data is not on it.** Data is on the nodes that keep each space. When a
  rendezvous node goes away, only one way to find people goes away. No data
  goes with it.
- **It works from memory only.** It needs no disk. After a restart, nodes
  send their records again in a few minutes.
- **It is part of `fileos`.** Any node with a public address can be one, with no
  other program and no config. A node can also be only a rendezvous node.
- **It has limits.** A small number of records for each key, a size limit for
  each record, a limit on how often a node may write, and records that end.

Rendezvous is not the same job as keeping data (the `store` ability), keeping
mail for nodes that are offline, or relaying connections. A node may do any of
these jobs and not the others. Relaying is the one that costs money (bandwidth),
so it has its own limits.

Whom a rendezvous node helps is a policy, with a default. By default, a node
with a public address is a rendezvous node for the identity of its owner and
for the nodes that its owner trusts.

### The rendezvous service of the project

The project also runs a rendezvous node, on one small server with a relay
that has limits. One small server is enough for a very large number of
identities: a million identities is about 1 GB of memory, and the signatures
of their records take about a tenth of one CPU core.

It is not special:

- It runs the normal `fileos`, with the same code and protocol as any other
  rendezvous node. Its address is not in the kernel.
- It is a default in the policy. A new identity has
  it in its rendezvous list. `fileos identity` shows it, and one command takes it away.
- The nodes of the owner come first. When an identity has a node with a public
  address, that node comes first in the list and in the hints of its link.
- When it is down, nobody is lost. People who know you already have your
  identity space. Only people who find you for the first time need it.

## Keys that cannot be followed back

A rendezvous node must not learn which identity a record is for, and nobody
may be able to make a list of all identities. So records are not kept under
the DID. For each day (a **period** `p`, of 24 hours), the nodes of the
identity make:

```
name = BLAKE3 derive_key("fileos rendezvous name v1", DID || p)
key  = BLAKE3 derive_key("fileos rendezvous key v1",  DID || p)
```

- The record is kept under `name`. The name is new every day, and it cannot
  be followed back to the DID.
- The record is encrypted with `key`, with XChaCha20-Poly1305. Inside it are
  the node certificate (signed by the signing key) and the address record
  (signed by `N`).
- Anyone who knows the DID can make `name` and `key` and find the record.
  Anyone who does not know the DID cannot.

On the rendezvous node, each node `N` has its own place under a name. Only
`N` can change its place, because the record is signed by `N`. Someone may
put records of no value under a name, but only someone who knows the DID knows
the name, the limits keep this small, and readers throw away records that do
not check.

In the last two hours of a period, nodes also send their records for the next
period. When people look, they try the period of now and then the one before
it, in case clocks are not the same.

### Why not the key blinding of Tor

Tor onion services make a blinded key from the identity key, and records are
signed with the blinded key. That needs a secret key of the identity (or
certificates that it made earlier) for every period. In fileOS the secret
keys of the identity are on one device only, and that device may be offline. With the
design above, the nodes do the work alone, and we need no new kind of
cryptography: only BLAKE3, ed25519 and XChaCha20-Poly1305, which are already
in the dependency tree.

### What this does not hide

- The rendezvous node sees which node key sends a record, and the IP address
  it comes from.
- It sees which node asks for which name. It cannot see the identity, but it
  can see that node `X` looked for the records that node `N` sent. So it sees
  links between nodes.
- Anyone who knows your DID can see your addresses. By default they are relay
  addresses, not the address of your home.

Asking through the node of a friend would hide more. That is for later.

## How a node is found

To find the nodes of an identity:

1. **Hints.** Ask the rendezvous nodes and nodes in the link.
2. **The rendezvous list.** Ask the rendezvous nodes in the identity space,
   if we have a copy of it.
3. **People we know.** Ask the nodes we are connected to. A node that has
   records for `name` (as a rendezvous node, or from an earlier look) answers.

When one node of the identity answers, we have all of them: it has the identity space
with every node certificate, and the nodes of one identity send their newest
address records to each other.

If we know only the DID, with no hints, no copy of the identity space and no
shared rendezvous node or friend, we cannot find the nodes. In practice
people get an identity from a link, a QR code or a friend, so there are
always hints.

## The protocol

A libp2p request-response protocol, `/fileos/rendezvous/1`, with CBOR messages:

- `Publish(record)`: the answer is `Ok` or `Refused(reason)`.
- `Find(name)`: the answer is the records under `name`.

The logic (records, checks, limits, what a rendezvous node keeps, the steps of
a look) is sans-IO, in a portable crate, so it runs in `fileos-sim`.

## Plan

1. **Identity:** `did:fileos` (the DID log, rotation keys, the signing key),
   the identity space, node certificates, identity links, and adding a node
   (`fileos identity`, `fileos identity new`, `fileos identity add-node`, `fileos identity join`). Built.
2. **More keys and names:** add and change rotation keys and the signing key,
   a recovery key kept on paper, and domain names (`_fileos` and
   `/.well-known/fileos-did`).
3. **Address records:** nodes sign their addresses, and the nodes of one
   identity send them to each other.
4. **Rendezvous:** names and encryption by period, the rendezvous store with
   its limits, the order of a look, and the project's rendezvous node as a
   default. Tests in `fileos-sim`.
5. **The project's rendezvous node:** put it on a server.
