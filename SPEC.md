# homeserver specification

**English** | [简体中文](SPEC.zh-CN.md)

This document says how homeserver works: what a node keeps, in what form, and
what it says to clients and to other nodes. It is complete in itself. The only
other documents it names are published standards (RFCs and the like). They are
listed in [Appendix D](#appendix-d-normative-references).

It describes **homeserver 0.1.0**. That release is experimental, so the formats
in it may still change.

## Contents

- [0. About this document](#0-about-this-document)
- **Part I. The whole picture**
  - [1. What homeserver is](#1-what-homeserver-is)
  - [2. The main ideas](#2-the-main-ideas)
  - [3. Words used in this document](#3-words-used-in-this-document)
  - [4. How the parts work together](#4-how-the-parts-work-together)
- **Part II. Encodings**
  - [5. Text forms of bytes](#5-text-forms-of-bytes)
  - [6. Hashes and derived keys](#6-hashes-and-derived-keys)
  - [7. Deterministic CBOR](#7-deterministic-cbor)
  - [8. Signatures and COSE_Sign1](#8-signatures-and-cose_sign1)
  - [9. JSON](#9-json)
- **Part III. Identity**
  - [10. Keys](#10-keys)
  - [11. Recovery code and recovery shares](#11-recovery-code-and-recovery-shares)
  - [12. The identity log](#12-the-identity-log)
  - [13. The DID document](#13-the-did-document)
- **Part IV. Spaces and their logs**
  - [14. The space model](#14-the-space-model)
  - [15. Proposals](#15-proposals)
  - [16. Entries and the hash chain](#16-entries-and-the-hash-chain)
  - [17. Payloads of each kind](#17-payloads-of-each-kind)
  - [18. Ordering and applying](#18-ordering-and-applying)
  - [19. Heads, segments and checkpoints](#19-heads-segments-and-checkpoints)
  - [20. Write conditions and idempotency](#20-write-conditions-and-idempotency)
- **Part V. The node and its data directory**
  - [21. The data directory](#21-the-data-directory)
  - [22. Configuration](#22-configuration)
  - [23. Making and opening a node](#23-making-and-opening-a-node)
  - [24. Blobs](#24-blobs)
  - [25. Space logs on disk](#25-space-logs-on-disk)
  - [26. The write path in a node](#26-the-write-path-in-a-node)
  - [27. The derived index](#27-the-derived-index)
- **Part VI. Access**
  - [28. Who is asking](#28-who-is-asking)
  - [29. Grants and tokens](#29-grants-and-tokens)
- **Part VII. The HTTP interface**
  - [30. The HTTP server](#30-the-http-server)
  - [31. The REST API](#31-the-rest-api)
  - [32. Contacts and calendars (CardDAV and CalDAV)](#32-contacts-and-calendars-carddav-and-caldav)
  - [33. Password vault (Bitwarden API)](#33-password-vault-bitwarden-api)
  - [34. AI assistants (MCP)](#34-ai-assistants-mcp)
  - [35. Sign-in for people and other apps](#35-sign-in-for-people-and-other-apps)
  - [36. Notifications (Web Push)](#36-notifications-web-push)
  - [37. Media](#37-media)
  - [38. The web app files](#38-the-web-app-files)
- **Part VIII. Between nodes**
  - [39. Finding and trusting another node](#39-finding-and-trusting-another-node)
  - [40. HTTP Message Signatures](#40-http-message-signatures)
  - [41. The federation endpoints](#41-the-federation-endpoints)
  - [42. The sync protocol](#42-the-sync-protocol)
  - [43. The outgoing HTTP client](#43-the-outgoing-http-client)
  - [44. Port mapping](#44-port-mapping)
- **Part IX. Backups and moving**
  - [45. Replicas](#45-replicas)
  - [46. Encryption of replica objects](#46-encryption-of-replica-objects)
  - [47. Replica sync](#47-replica-sync)
  - [48. Restore](#48-restore)
  - [49. S3 signing and the S3 endpoint of a node](#49-s3-signing-and-the-s3-endpoint-of-a-node)
  - [50. Export](#50-export)
- **Part X. The command line**
  - [51. Program, options and where things are](#51-program-options-and-where-things-are)
  - [52. Commands](#52-commands)
- **Appendices**
  - [A. Constants](#appendix-a-constants)
  - [B. Not in this version](#appendix-b-not-in-this-version)
  - [C. Known limits of this version](#appendix-c-known-limits-of-this-version)
  - [D. Normative references](#appendix-d-normative-references)

## 0. About this document

**How to read it.** Part I gives the whole picture in plain words. Each later
part goes one level down, and it uses only things that the parts before it
have defined. Part II defines the byte encodings. Part III builds identity on
them. Part IV builds spaces and their logs. Part V puts these on disk in one
node. Part VI says who may do what. Parts VII and VIII give the HTTP interfaces
to clients and to other nodes. Part IX covers backups. Part X covers the
command line. The appendices give every constant in one table, what is not in
this version, and the known limits.

**What it covers.** It covers behaviour that can be seen from outside a node.
That means files in the data directory, bytes on the network, HTTP requests and
responses, and the command line. It does not say how the program is divided
inside. Another program that does what this document says is compatible with
homeserver 0.1.0.

**Requirement words.** The words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY,
in capitals, have the meanings given in BCP 14 (RFC 2119 and RFC 8174).

**Notation.**

- `‖` joins byte strings end to end.
- `0x..` is a byte or a number in hexadecimal.
- **ms** is a time: whole milliseconds since 1970-01-01T00:00:00Z (Unix time),
  as an unsigned 64-bit number. **s** is the same in whole seconds.
- **Hex** is lowercase hexadecimal, two characters per byte, unless the text
  says something different.
- Text is UTF-8.
- `{name}` in a path or a string is a value put in its place.
- Sizes: 1 KiB = 1024 bytes, 1 MiB = 1024 KiB.

# Part I. The whole picture

## 1. What homeserver is

homeserver is a personal server. One program, `hs`, with one data directory,
is one **node**. A node belongs to one person, and it keeps that person's data:
notes, contacts, events, files, photos, passwords and anything else written as
a **resource**.

A node does three jobs:

1. **It keeps data.** All changes are kept in signed logs that are only added
   to. File contents are kept as blobs named by their hash. A search index is
   made from these, and it can be made again at any time.
2. **It serves clients.** Everything is reached over HTTP. There is a REST API,
   and there are adapters for standard clients: CardDAV and CalDAV, a
   Bitwarden-compatible vault, the Model Context Protocol (MCP) for AI
   assistants, an OpenID Connect provider, passkeys, forward auth and Web Push.
   The `hs` command line is one more client of the same API.
3. **It works with other nodes.** Data is put in **spaces**. A space can have
   members on other nodes. One node, the space's **primary**, puts all changes
   to the space in order. The other members get copies of the log and send
   their changes to the primary. A node can also keep encrypted backups
   (**replicas**) in a directory, in an S3 bucket, or on another node.

## 2. The main ideas

These ideas shape every later part.

1. **The log is the truth.** Every change to a space is an **entry** in the
   space's log. An entry is signed, it has a sequence number, and it has the
   hash of the entry before it. Entries are never changed. The current state of
   a space is what you get when you apply its entries in order.
2. **Contents are named by their hash.** A blob's name is the BLAKE3 hash of
   its bytes. An entry does not hold the resource's bytes. It holds their hash,
   and the bytes are kept as a blob.
3. **Everything else is derived.** The SQLite index (current resources, search,
   places, links) is made from the logs and blobs. A node can delete it and make
   it again.
4. **One node orders each space.** A space has one primary. Only the primary
   adds entries. Other nodes propose changes to it. Nothing needs agreement
   between many nodes.
5. **One API and one permission model.** Apps, people's devices, other nodes
   and AI assistants all use HTTP. A **grant** says what a token may do: a
   role, and maybe a list of spaces and a list of resource types. Grants are
   entries in the owner's private space, so they are signed and kept in order.
6. **Signed requests between nodes.** A node signs its requests to other nodes
   with its node key (HTTP Message Signatures). A node trusts a key because the
   key is a member of a space, not because of a certificate authority.
7. **Encrypt before it leaves.** Backups are encrypted with a key that comes
   from the person's recovery code. A replica store sees only random-looking
   names and ciphertext.
8. **Policy is not built in.** Where a choice is the person's, the node gives a
   setting with a default. Examples: which replicas to keep, whether to open
   registration of a vault.

## 3. Words used in this document

| Word          | Meaning                                                                                                           |
| ------------- | ----------------------------------------------------------------------------------------------------------------- |
| person        | The owner of a node.                                                                                              |
| personal key  | An Ed25519 key pair that is the person. It is not kept on the node. The recovery code is its seed.                |
| identity ID   | The text form of the first personal key. It never changes.                                                        |
| identity log  | A short signed log that says which personal key is current, which nodes serve the person, and at which addresses. |
| node          | One running `hs` with its data directory.                                                                         |
| node key      | The Ed25519 key pair of one node. It signs log entries, proposals and HTTP requests.                              |
| storage key   | A 32-byte key made from the personal key's seed. It encrypts replicas.                                            |
| space         | A unit of data and sharing, with members and one log.                                                             |
| private space | The space made by `init`. It keeps grants, the vault, passkeys and other secrets of the node.                     |
| member        | A node key with a role in a space.                                                                                |
| role          | `reader`, `writer` or `owner`, in that order of power.                                                            |
| primary       | The member node that orders a space's log.                                                                        |
| home          | The URL at which a space's primary can be reached.                                                                |
| proposal      | A change, signed by the node that wants it.                                                                       |
| entry         | A proposal put in order by the primary, signed by the primary.                                                    |
| seq           | The sequence number of an entry. The first entry (genesis) has seq 0.                                             |
| target        | The name of a resource inside a space: `type/id`, such as `note/shopping`.                                        |
| resource      | The data at a target. Its current bytes are its payload.                                                          |
| payload       | The bytes that a proposal names by hash.                                                                          |
| blob          | Bytes kept under their BLAKE3 hash.                                                                               |
| grant         | A record that gives a token (or a session) a role.                                                                |
| token         | A secret string that a client sends to use a grant.                                                               |
| replica       | An encrypted copy of a node's data directory, somewhere else.                                                     |
| index         | The SQLite database that is made from the logs.                                                                   |

## 4. How the parts work together

This section follows the data through the system. Later parts give the exact
rules.

### 4.1 Making and starting a node

`hs ctl init --name NAME` makes a data directory. It makes three secrets: a
personal key, a node key and the storage key. It writes an identity log that
says "this node serves this person". It makes the private space, with this node
as primary and only owner. It writes a first grant, with role owner, and it
shows that grant's token one time. It shows the personal key's seed one time,
as the **recovery code**, and does not keep it.

`hs ctl serve` opens the data directory, takes a lock on it, reads all space
logs, brings the index up to date, and listens for HTTP.

### 4.2 Writing a resource

A client sends `PUT /api/spaces/{space}/resources/note/shopping` with a JSON
body and a token.

1. The node finds the grant for the token. The grant must give at least
   `writer` in that space, and its type list (if it has one) must include
   `note`.
2. The node keeps the body as a blob and makes a **proposal**: "put the
   payload with this hash at `note/shopping` in this space". It signs the
   proposal with its node key.
3. If this node is the space's primary, it checks the proposal, gives it the
   next seq, makes an **entry** that holds the proposal and the hash of the
   entry before it, signs the entry, and adds it to the log on disk.
   If another node is the primary, this node sends the proposal to it, waits
   for the seq, and then gets the new entry from it.
4. The node applies the entry to the space's state, updates the index, and
   tells listeners (event streams, push notifications, other members).
5. The client gets the seq and the resource's new ETag.

### 4.3 Reading and finding

A read of one resource comes from the space's state and the blob store. A list,
a search, the timeline, places near a point, and "everything linked to this"
come from the index. A resource can be given as JSON, or as vCard or iCalendar
when it holds a contact or an event.

### 4.4 Sharing a space with another node

Alice's node A has a space. Bob's node B will be a member.

1. Alice adds B. A asks B's address for B's node key and identity, and writes a
   `member` entry for B's node key, with B's address as its home.
2. Bob tells B to join the space at A's address. B gets the whole log from A,
   checks every entry, gets the blobs it needs, and keeps its own copy.
3. When A adds an entry, A sends it to every member that has a home. Every 60
   seconds, each member also asks the primary for entries it does not have.
4. When Bob writes on B, B sends the signed proposal to A. A orders it, and
   the new entry comes back to B.
5. An owner can make another member the primary (**handover**). After that,
   the members send their proposals to the new primary.

### 4.5 Backups and moving

A node can keep any number of **replicas**: a directory, an S3 bucket, or the
S3 endpoint of another node. Every hour it encrypts each changed file of its
data directory and uploads it. Then it uploads an encrypted list of all the
files (the manifest), with a compare-and-swap write. With only the recovery
code and the replica's address, `hs ctl restore` makes the data directory
again.

### 4.6 One picture

```
 clients: hs CLI · DAV apps · Bitwarden apps · MCP · OIDC apps · browsers
        │ HTTP + token (one grant model)
        ▼
┌──────────────────────────── node ─────────────────────────────┐
│ HTTP interface  REST · DAV · Bitwarden · MCP · OIDC · push · S3 │
│ access          grants and tokens (in the private space)      │
│ index           SQLite: resources, search, places, links      │
│                 (made again from the logs at any time)        │
│ spaces          signed proposals → ordered, hash-chained log  │
│ identity        personal key · node key · identity log · DID  │
│ storage         logs/ · blobs/ · keys/ · config.json          │
└───────────────────────────────────────────────────────────────┘
        │ HTTP + HTTP Message Signatures (node key)
        ▼
 other nodes (members of shared spaces) · replica stores
```

# Part II. Encodings

Every later part uses these encodings. They are small subsets of standards, and
the node is strict about them: there is one right encoding for each value.

## 5. Text forms of bytes

### 5.1 Hex

Output is lowercase. Input accepts uppercase too. A 32-byte hash is 64
characters. Input that does not decode to exactly the expected number of bytes
is not valid.

### 5.2 Base32

This is the base32 of RFC 4648 section 6, with the alphabet in lowercase:

```
abcdefghijklmnopqrstuvwxyz234567
```

- Bits are taken most significant first.
- Output has no padding. Input with `=` is not valid.
- Input may be in uppercase.
- The unused low bits of the last character MUST be zero, so each value has
  one encoding. An input length that whole bytes cannot give (length mod 8 is
  1, 3 or 6) is not valid. No other characters are skipped.
- Examples: `""`→`""`, `"f"`→`my`, `"fo"`→`mzxq`, `"foo"`→`mzxw6`,
  `"foob"`→`mzxw6yq`, `"fooba"`→`mzxw6ytb`, `"foobar"`→`mzxw6ytboi`.

32 bytes are 52 characters.

### 5.3 Base64

- **base64url** is RFC 4648 section 5 without padding. Where the node reads
  base64url from a client (WebAuthn, Web Push keys and the like), it accepts
  the standard alphabet and padding too, unless a section says something
  different.
- **base64** is RFC 4648 section 4 with padding.

### 5.4 Key IDs

A **key ID** is the base32 (section 5.2) of a 32-byte Ed25519 public key: 52
characters. Because of the rule on unused bits, the last character is always
`a` or `q`. Node keys, personal keys and identity IDs are written as key IDs.

### 5.5 Multikey

The DID document (section 13) writes an Ed25519 public key as a Multikey:
`z` followed by base58btc of the bytes `0xed 0x01 ‖ key` (32 bytes). Base58btc
uses the alphabet
`123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz`. It is a
big-endian change of base, and each leading `0x00` byte becomes one leading
`1`. Examples: the bytes of `Hello World!` give `2NEpo7TZRRrLZSi2U`; the bytes
`00 00 01` give `112`.

## 6. Hashes and derived keys

- **Hash.** The hash of bytes `b` is plain BLAKE3 of `b`, with 32 bytes of
  output, no key and no context. A hash is written as hex (64 characters).
  "The hash of" in this document always means this, unless it names another
  function.
- **Derived key.** `derive_key(context, material)` is the BLAKE3 key
  derivation mode with the given context string. The node uses these contexts:

  | Context string                   | Material                  | Use                      |
  | -------------------------------- | ------------------------- | ------------------------ |
  | `homeserver 2025 storage key v1` | the 32-byte personal seed | storage key (section 10) |
  | `homeserver 2025 token v1`       | the bytes of a token      | token hash (section 29)  |

- **Keyed hash.** `keyed(key, msg)` is BLAKE3 in keyed mode, with a 32-byte
  key. Replica encryption uses it (section 46).
- **SHA-256** (FIPS 180-4) is used only where an outside standard needs it:
  HTTP `Content-Digest`, S3 Signature Version 4, WebAuthn, PKCE, JOSE key
  thumbprints, OAuth client secrets and the Bitwarden password hash.

## 7. Deterministic CBOR

Logs, proposals and identity entries use CBOR (RFC 8949). The node writes and
reads only the subset given here, in the deterministic encoding of RFC 8949
section 4.2.1.

### 7.1 Data model

| Major type | Values                                            |
| ---------- | ------------------------------------------------- |
| 0          | unsigned integer, 0 to 2^64−1                     |
| 1          | negative integer, −1 to −2^64                     |
| 2          | byte string                                       |
| 3          | text string (UTF-8)                               |
| 4          | array                                             |
| 5          | map (keys may be of any type in this list)        |
| 7          | only `false` (0xf4), `true` (0xf5), `null` (0xf6) |

There are no floating-point numbers, no tags (major type 6), no `undefined`,
no other simple values, and no indefinite lengths.

### 7.2 Writing

1. Every integer and every length uses the shortest form: below 24 in the
   first byte; up to 0xff with additional information 24 and 1 byte; up to
   0xffff with 25 and 2 bytes; up to 0xffffffff with 26 and 4 bytes; others
   with 27 and 8 bytes. Multi-byte numbers are big-endian.
2. A map's entries are sorted by the bytes of their encoded keys, in byte order
   (a shorter key that is a prefix comes first). If two keys encode the same,
   only the first one given is written.

### 7.3 Reading

A reader MUST refuse:

1. any bytes after the one top-level item (`trailing bytes`);
2. nesting deeper than 32 (the top item is at depth 0, and each array item, map
   key and map value is one deeper) (`nested too deeply`);
3. additional information 28, 29, 30 or 31, in any major type
   (`unsupported additional information`);
4. an integer or a length not in its shortest form (`non-shortest integer`);
5. input that ends too soon (`unexpected end`), and lengths that do not fit
   (`length overflow`);
6. an array or map whose count is greater than the length of the whole input
   (`unexpected end`), before it makes space for the items;
7. text that is not valid UTF-8 (`invalid UTF-8`);
8. a map whose encoded keys are not in strictly rising byte order; this also
   refuses keys that are the same (`map keys not in canonical order`);
9. major type 7 with any value but false, true or null
   (`unsupported simple value`);
10. major type 6 (`unsupported major type`).

The error text is `cbor: ` followed by the name in brackets.

Where a section below defines a map with integer keys, a reader finds each
field by its key, and it does not look at keys it does not know.

## 8. Signatures and COSE_Sign1

### 8.1 Ed25519

All keys of persons and nodes are Ed25519 (RFC 8032): pure Ed25519, with no
prehash and no context. A key pair comes from a 32-byte seed. A signature is
64 bytes.

### 8.2 COSE_Sign1

Signed objects (proposals, entries, heads, checkpoints, identity log entries)
are COSE_Sign1 messages (RFC 9052), **without** the COSE tag 18:

```
COSE_Sign1 = [
  protected:   bstr .cbor {1: -8},     ; alg = EdDSA
  unprotected: {4: bstr},              ; kid = 32-byte public key of the signer
  payload:     bstr,                   ; always present, never nil
  signature:   bstr                    ; 64 bytes
]
```

The protected header is always the three bytes `a1 01 27`. So every message
starts with these bytes:

```
84 43 a1 01 27 a1 04 58 20 <32-byte kid> <payload bstr> 58 40 <64-byte signature>
```

**What is signed** is the Sig_structure of RFC 9052 section 4.4, with an empty
external_aad:

```
["Signature1", h'a10127', h'', payload]
= 84 6a 5369676e617475726531 43 a10127 40 <payload bstr>
```

**Reading** a COSE_Sign1, in order:

1. Decode the CBOR with the rules of section 7.3.
2. It MUST be an array of exactly four items.
3. Item 0 MUST be a byte string whose bytes are exactly `a1 01 27`.
4. Item 1 MUST have key 4 with a byte string (the kid). Other keys in item 1
   are not used.
5. Items 2 and 3 MUST be byte strings.

**Checking** a COSE_Sign1: the kid MUST be 32 bytes, and the signature MUST be
a valid Ed25519 signature of the Sig_structure under the kid as public key.
The result is the kid: the key that signed.

Where this document says "the hash of a signed object", it means the hash of
the COSE_Sign1 bytes exactly as they were stored or sent. It does not mean the
hash of a new encoding of them.

## 9. JSON

JSON is RFC 8259.

- JSON that the node writes as a payload into a log (genesis information,
  members, grants, resources it makes itself) has no spaces, and the members
  of each object are sorted by name.
- JSON in HTTP responses has no spaces, unless a section says it is "pretty"
  (indented by two spaces). The order of members in a response has no meaning.
- A reader MUST NOT depend on the order of members.

# Part III. Identity

## 10. Keys

A node uses three secrets.

| Key          | Made from                                                     | Kept                                                          | Use                                                             |
| ------------ | ------------------------------------------------------------- | ------------------------------------------------------------- | --------------------------------------------------------------- |
| personal key | 32 random bytes (the personal seed)                           | **not on the node**; the person keeps it as the recovery code | signs the identity log; the identity ID is its first public key |
| node key     | 32 random bytes                                               | `keys/node` in the data directory                             | signs proposals, entries and HTTP requests                      |
| storage key  | `derive_key("homeserver 2025 storage key v1", personal seed)` | `keys/storage`                                                | encrypts replicas (section 46)                                  |

Random bytes MUST come from a secure random number generator of the operating
system.

The node keeps other keys for single protocols. They are given where they are
used: the OIDC signing key (section 35.2), the forward-auth secret
(section 35.5), the Web Push key (section 36.2) and the TLS certificate
(section 30.1).

## 11. Recovery code and recovery shares

### 11.1 Recovery code

The recovery code is the personal seed in a form that people can write down:

1. Take the base32 (section 5.2) of the 32-byte seed: 52 characters.
2. Cut it into groups of 4 characters, and join the groups with `-`. The result
   has 13 groups and 64 characters, such as `abcd-efgh-…-wxyq`.

To read a recovery code, keep only the ASCII letters and digits (drop dashes,
spaces and all other characters), then decode it as base32. The result MUST be
exactly 32 bytes.

### 11.2 Recovery shares

The recovery code can be split between friends with Shamir secret sharing, so
that any _k_ of _n_ shares give it back.

**The field.** GF(2^8) with the polynomial x^8 + x^4 + x^3 + x + 1 (0x11b, the
field of AES). Addition is XOR. The inverse of `a` is `a^254`.

**Splitting** a secret `s` (32 bytes) with threshold `k` and count `n`
(1 ≤ k ≤ n ≤ 255):

1. Get `(k−1) × 32` random bytes `r`.
2. Share number `x` runs from 1 to `n`.
3. For each byte position `i`, the polynomial is
   `f(x) = s[i] + r[i(k−1)] x + r[i(k−1)+1] x^2 + … + r[i(k−1)+k−2] x^(k−1)`.
   Share `x` has `y[i] = f(x)`.
4. With `k = 1`, every share is the secret.

**Combining** shares `(x_j, y_j)`: all `y` MUST have the same length, every `x`
MUST be non-zero, and no `x` may come twice. Each byte is the Lagrange value at
zero: `s[i] = Σ_j y_j[i] · Π_{m≠j} x_m / (x_m + x_j)`. With fewer than `k`
shares the result is wrong, and nothing shows it. The check value below finds
this.

**The text form of a share** is

```
hs-share-{x}-{check}-{y}
```

- `x` is the share number in decimal (1 to 255);
- `check` is the first 8 hex characters of the hash of the secret;
- `y` is the base32 of the 32 share bytes.

When shares are combined, every share MUST have the same `check`. The result
MUST be 32 bytes, and the first 8 hex characters of its hash MUST equal `check`.
If not, there were not enough shares, or one was damaged.

## 12. The identity log

The identity log says who the person is now and where their nodes are. It is a
list of entries, each a COSE_Sign1 (section 8.2) signed by the personal key
that is current at that point.

### 12.1 Entry

The payload is a CBOR map:

| Key | Field | Type                                                                                                    |
| --- | ----- | ------------------------------------------------------------------------------------------------------- |
| 1   | seq   | unsigned; the first entry has 0                                                                         |
| 2   | prev  | 32-byte byte string: the hash of the signed bytes of the entry before; 32 zero bytes in the first entry |
| 3   | time  | unsigned, ms                                                                                            |
| 4   | kind  | text, one of the kinds below                                                                            |
| 5   | value | depends on the kind                                                                                     |

| Kind          | Value                                           | Meaning                                              |
| ------------- | ----------------------------------------------- | ---------------------------------------------------- |
| `create`      | `null` (not checked when read)                  | the identity starts; only allowed as the first entry |
| `add-node`    | 32-byte node public key                         | this node serves the identity                        |
| `remove-node` | 32-byte node public key                         | this node no longer serves it                        |
| `address`     | text: a URL, such as `https://home.example.org` | an address where the nodes can be reached            |
| `alias`       | text: a domain name                             | another name for the identity                        |
| `rotate`      | 32-byte public key                              | from the next entry on, this key is the personal key |

An unknown kind makes the entry not valid.

### 12.2 The whole log

The log as one value (the file `identity` in the data directory) is a CBOR
array of byte strings. Each byte string is one entry's COSE_Sign1 bytes, in
order.

### 12.3 Checking the log and finding its state

Start with no state. For each entry, at position `i` from 0:

1. Decode and check the COSE_Sign1 (section 8.2). The kid is the signer.
2. Decode the payload as an entry.
3. `seq` MUST be `i`. `prev` MUST be 32 zero bytes when there is no state, and
   otherwise the hash of the entry before.
4. If there is no state, the kind MUST be `create`. The state becomes:
   identity ID = signer, personal key = signer, no nodes, no addresses, no
   aliases.
   If there is a state, the kind MUST NOT be `create`, and the signer MUST be
   the current personal key.
5. Apply the entry:
   - `add-node`: add the key to the end of the node list, if it is not in it;
   - `remove-node`: take the key out of the node list, if it is in it;
   - `address`: add the URL to the end of the address list, if it is not in it;
   - `alias`: add the domain to the end of the alias list, if it is not in it;
   - `rotate`: the personal key becomes the given key.
6. The state's seq becomes the entry's seq. The state's head becomes the hash
   of this entry's COSE_Sign1 bytes.

A log with no entries is not valid. Any failure makes the whole log not valid.
Nothing checks that times go up, and nothing checks the form of URLs or
domains. There is no kind that removes an address or an alias.

**Adding an entry.** Only the current personal key can add one. It has
`seq` = state seq + 1 and `prev` = state head.

## 13. The DID document

A node shows the identity as a W3C DID document (DID Core 1.0).

- The DID is `did:hs:` followed by the identity ID (section 5.4). The method
  name `hs` may change in a later version.
- The document is made from the state of the identity log:

```json
{
  "@context": [
    "https://www.w3.org/ns/did/v1",
    "https://w3id.org/security/multikey/v1"
  ],
  "id": "did:hs:{identity ID}",
  "alsoKnownAs": ["https://{alias}", "…"],
  "verificationMethod": [
    {
      "id": "did:hs:{id}#personal",
      "type": "Multikey",
      "controller": "did:hs:{id}",
      "publicKeyMultibase": "{multikey of the current personal key}"
    },
    {
      "id": "did:hs:{id}#node-{i}",
      "type": "Multikey",
      "controller": "did:hs:{id}",
      "publicKeyMultibase": "{multikey of node i}"
    }
  ],
  "authentication": ["did:hs:{id}#personal"],
  "assertionMethod": ["did:hs:{id}#personal"],
  "service": [
    {
      "id": "did:hs:{id}#hs-{i}",
      "type": "Homeserver",
      "serviceEndpoint": "{address i}"
    }
  ]
}
```

- `alsoKnownAs` has one item per alias, in log order.
- `verificationMethod` has the personal method first, then one method per
  current node key, with `i` counting from 0 in log order.
- `service` has one item per address, with `i` counting from 0.
- The DID uses the **first** personal key. `#personal` shows the **current**
  personal key.
- `{i}` is a position in the current list, so the IDs of later items change
  when a node is removed.
- Multikey is defined in section 5.5.

# Part IV. Spaces and their logs

## 14. The space model

A **space** is a set of resources with members and one log. Its state is
exactly what comes from applying its log entries in order (section 18). The
state has these parts:

| Part      | Content                                                               |
| --------- | --------------------------------------------------------------------- |
| id        | the space ID, a 32-byte hash (section 16.3)                           |
| info      | the JSON value of the genesis payload                                 |
| primary   | the node key that orders the log                                      |
| members   | a map from node key to `{identity, role, home}`                       |
| resources | a map from target to `{payload, media type, seq, time, history}`      |
| grants    | a map from grant ID to `{body, revoked}`                              |
| seq, head | the seq and the hash of the last entry                                |
| nonces    | a map from (author key, nonce) to the seq where that proposal went in |

The **name** of a space is the string `info.name`, or the empty string if
there is none.

A member is a **node key**, not a person. Each member record also has the
member's identity ID, and maybe a **home**: the URL where that node can be
reached.

A resource's `payload` is the hash of its current bytes, or nothing when it
was deleted or erased. `time` is the time of the entry that last changed it.
`history` is every payload hash it has had, so that erasing can find them.

### 14.1 Roles and what they allow

The roles are `reader` < `writer` < `owner`. Their text forms are exactly these
lowercase words.

| Kind of proposal                        | Lowest role needed                 |
| --------------------------------------- | ---------------------------------- |
| `mls`                                   | reader                             |
| `put`, `delete`, `erase`                | writer                             |
| `member`, `grant`, `revoke`, `handover` | owner                              |
| `genesis`                               | not allowed after the space starts |

A node that is not a member cannot propose anything. Only the node whose key
is the space's `primary` can add entries. Being primary does not depend on
the role.

A space MUST always have at least one owner (section 18.3).

## 15. Proposals

A **proposal** is a request to change a space. It is a COSE_Sign1
(section 8.2) whose payload is this CBOR map:

| Key | Field      | Type                           | Notes                                                                                                           |
| --- | ---------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| 1   | space      | 32-byte byte string            | the space ID; 32 zero bytes in a genesis                                                                        |
| 2   | kind       | text                           | one of the kinds in section 15.1                                                                                |
| 3   | target     | text                           | a valid target (section 15.2)                                                                                   |
| 4   | payload    | 32-byte byte string, or `null` | hash of the payload; the key MUST be there                                                                      |
| 5   | media_type | text                           | such as `application/json`; may be empty                                                                        |
| 6   | time       | unsigned                       | ms, by the author's clock                                                                                       |
| 7   | nonce      | byte string                    | any length; the writer makes it random, or derives it (section 20.2)                                            |
| 8   | grant      | text, or `null`                | ID of the grant that the change was made under; the key MUST be there                                           |
| 9   | request    | 32-byte byte string            | fingerprint of the HTTP request (section 20.2); the key is left out when there is none, and `null` is not valid |

The signer (the kid) is the **author**. In this version the author is always a
node key: the node that made the proposal for its client.

Keys 1 to 8 MUST be there. A reader checks the fields in this order, and gives
the error `proposal: bad field {name}`: target (text), kind, target (valid),
space, payload, media type, time, nonce, grant, request.

### 15.1 Kinds

| Kind       | Meaning                                                      | Payload                                          |
| ---------- | ------------------------------------------------------------ | ------------------------------------------------ |
| `genesis`  | starts a space                                               | JSON (section 17.1)                              |
| `put`      | sets a resource's bytes                                      | the resource's bytes                             |
| `delete`   | removes a resource                                           | none (if one is given, it is put in the history) |
| `erase`    | removes a resource and all of its old bytes                  | none (if one is given, it is not used)           |
| `member`   | adds, changes or removes a member                            | JSON (section 17.2)                              |
| `grant`    | records a grant                                              | JSON (section 17.3)                              |
| `revoke`   | revokes a grant                                              | none                                             |
| `mls`      | an MLS message; it does nothing to the state in this version | MLS bytes                                        |
| `handover` | names a new primary                                          | JSON (section 17.4)                              |

### 15.2 Targets

A target is `type/id`:

1. There are exactly two parts when it is split on `/`.
2. Each part is 1 to 128 bytes long.
3. Each byte is an ASCII letter or digit, or one of `-` `_` `.` `~`.
4. No part is `.` or `..`.

So `note/abc-1` is valid. `note`, `note/a/b` and `note/..` are not. The first
part is the resource's **type**.

## 16. Entries and the hash chain

### 16.1 Entry

An **entry** is a COSE_Sign1 signed by the primary. Its payload is this CBOR
map:

| Key | Field    | Type                                                                                           |
| --- | -------- | ---------------------------------------------------------------------------------------------- |
| 1   | seq      | unsigned; 0 for the genesis                                                                    |
| 2   | prev     | 32-byte byte string: the hash of the signed bytes of the entry before; 32 zero bytes for seq 0 |
| 3   | time     | unsigned, ms, by the primary's clock                                                           |
| 4   | proposal | byte string: the author's whole COSE_Sign1 bytes                                               |

All four keys MUST be there (error `entry: bad field {name}`).

### 16.2 The chain

Entry `n` has `seq = n` and `prev` = the hash of entry `n−1`. A log is valid
only if every entry follows this rule and is signed by the key that is primary
at that point. The **head** of a log is the hash of its last entry.

### 16.3 Space ID and genesis

The **space ID** is the hash of the signed genesis **proposal** (not of the
entry).

To start a space, a node:

1. Takes the info object (for example `{"name": "Photos"}`). It sets three
   members in it, and these replace any that are there:
   - `primary`: its own node key ID;
   - `members`: `[{"node": {its node key ID}, "identity": {its identity ID}, "role": "owner"}]`;
   - `created`: the time, in ms.
2. Writes the info as JSON (section 9). These bytes are the payload.
3. Makes a proposal: space = 32 zero bytes, kind `genesis`, target
   `space/info`, payload = the hash of the bytes, media type
   `application/json`, the time, a nonce of 16 random bytes, grant `null`, and
   no request. It signs the proposal with its node key.
4. Makes entry 0: seq 0, prev = 32 zero bytes, the time, the signed proposal.
   It signs the entry with its node key.

## 17. Payloads of each kind

All of these payloads are JSON with media type `application/json`. The node
reads the payload from the blob whose hash is in the proposal. The bytes MUST
have that hash. If the bytes are not there, the entry cannot be applied (and
a log that holds it cannot be replayed).

### 17.1 Genesis

The info object of section 16.3. These members are used:

- `primary` (text, a node key ID): the first primary. It is required.
- `members` (array of member objects, section 17.2): the first members. If it
  is missing or not an array, there are none.
- `name` (text): the space's name.
- `kind` (text): `private` for the private space. Other values have no meaning
  in this version.

Other members are kept and have no meaning.

### 17.2 Member

```json
{
  "node": "{node key ID}",
  "identity": "{identity ID}",
  "role": "writer",
  "home": "https://…",
  "remove": true
}
```

- `node` and `identity` are required, and MUST be valid key IDs.
- `role` is `reader`, `writer` or `owner`. If it is missing or is not a
  string, it is `writer`. Any other string makes the payload not valid.
- `home` is a URL, and is optional.
- `remove`: if it is the JSON value `true`, the member is removed. A removal
  payload still needs valid `node` and `identity`.

The node uses the target `member/{node key ID}`. The rules do not check the
target.

### 17.3 Grant

Any JSON object with a text member `id`. The whole object is the grant's body.
Section 29 gives the members that the node uses. The node uses the target
`grant/{id}`.

### 17.4 Handover

```json
{ "primary": "{node key ID}" }
```

The node uses the target `space/primary`.

### 17.5 Revoke

No payload. The target is `grant/{id}`.

## 18. Ordering and applying

### 18.1 Checking a proposal

The primary does this before it orders a proposal. Every node does it again
when it applies an entry (except entry 0). The first failure stops it, with
the reason given:

1. The COSE_Sign1 decodes and its signature is good (else `bad proposal signature`).
   The kid is the author.
2. The proposal decodes (section 15).
3. The kind is not `genesis` (else `genesis only starts a space`).
4. `space` is this space's ID (else `proposal is for another space`).
5. The author is a member (else `author is not a member`).
6. The author's role is at least the role that the kind needs (section 14.1)
   (else `a {role} cannot {kind}`, for example `a reader cannot put`).
7. For `put`, `member`, `grant`, `handover` and `mls`, `payload` is not `null`
   (else `missing payload`).

Nothing checks the proposal's time, its media type, or whether the target fits
the kind. The `grant` and `request` fields are recorded, but the space rules do
not use them.

### 18.2 Ordering (on the primary)

To order a signed proposal at time `t`:

1. This node's key MUST be the space's `primary` (else
   `not the primary of this space`).
2. Check the proposal (section 18.1).
3. **Duplicate.** If (author, nonce) is already in `nonces`, stop. The answer
   is "duplicate" with the seq where it went in. Nothing changes, and the
   write conditions are not checked.
4. **Conditions.** Give the current payload hash of the target (or nothing) to
   the write conditions of section 20.1. If they fail, stop.
5. Make the entry `{seq: seq+1, prev: head, time: t, proposal}` and sign it
   with the node key.
6. Apply the entry (section 18.3). All checks of 18.3 come before the
   entry is written to disk, so a refused entry is never written.

If any step fails, the state does not change.

### 18.3 Applying an entry

To apply a signed entry (on the primary after ordering, and on every other
member when it gets the entry):

1. The COSE_Sign1 decodes and its signature is good (else `bad entry signature`).
   The kid is the signer. The entry decodes.
2. If seq is 0, do the genesis steps (section 18.4). Else:
   - the signer MUST be the current `primary` (else `entry not signed by the primary`);
   - seq MUST be the current seq + 1, and prev MUST be the current head (else
     `entry {seq} does not follow {current seq}`);
   - check the proposal (section 18.1).
3. Work out the change for the kind:
   - `put`, `delete`: a resource change;
   - `erase`: an erase;
   - `member`: read the payload (else `member payload missing`) and parse it
     (else `bad member`). If the result is not a removal and its role is
     `owner`, it is allowed. Otherwise there MUST be some _other_ member that
     is an owner (else `a space needs an owner`);
   - `grant`: read the payload (else `grant payload missing`); it MUST have a
     text `id` (else `grant has no id`);
   - `revoke`: a revoke;
   - `handover`: read the payload (else `handover payload missing`); `primary`
     MUST be a valid key ID (else `handover names no primary`), and that key
     MUST be a member of any role (else `the new primary must be a member`);
   - `mls`: no change.
4. Write the entry to the log on disk (section 25). If that fails, stop.
   Nothing has changed.
5. Make the change:
   - **put**: make the resource if it is not there. Set payload = the
     proposal's payload, media type = the proposal's media type, seq = this
     seq, time = this entry's time, and add the payload hash to the history.
   - **delete**: the same, but payload = nothing. If the proposal has a
     payload hash, it is added to the history.
   - **erase**: if the resource is there, the list of things to erase is its
     whole history. The history becomes empty and payload = nothing. Its media
     type, seq and time do not change. If it is not there, nothing happens.
   - **member**: put the member record in the map, or take it out for a
     removal.
   - **grant**: put `{body, revoked: false}` under the grant ID. If a grant
     with that ID was revoked, this makes it good again.
   - **revoke**: the grant ID is the target with a leading `grant/` taken
     off. If that grant is there, it becomes revoked. If not, nothing happens.
     The entry is still valid.
   - **handover**: `primary` becomes the given key.
6. Set `nonces[(author, nonce)] = seq`, the state's seq = this seq, and head =
   the hash of this entry. A member does not refuse an entry whose (author,
   nonce) it has seen before. Only the primary looks for duplicates.
7. The result has the seq, the entry hash, the proposal, the author, the
   entry's time, and the list of hashes to erase.

### 18.4 Entry 0 (genesis)

1. The space MUST have no members yet (else `genesis on a started space`).
2. The hash of the entry's proposal MUST be the space ID, and prev MUST be 32
   zero bytes (else `genesis does not match the space ID`).
3. The proposal's COSE_Sign1 MUST decode and have a good signature (else
   `bad genesis signature`), and the kind MUST be `genesis` (else
   `entry 0 must be a genesis`).
4. Read the payload (else `genesis payload missing`).
5. `info.primary` MUST be a key ID (else `genesis has no primary`), and it
   MUST be both the entry's signer and the proposal's author (else
   `genesis must be signed by its primary`).
6. Each item of `info.members` MUST be a valid member (else
   `bad genesis member`).
7. The state's info, primary and members are set from the payload.

### 18.5 Replay

To build a space from its log, start with an empty state with the space ID,
and apply each entry in order. A log with no entries is not valid
(`empty log`). The first failure stops the replay.

## 19. Heads, segments and checkpoints

### 19.1 Signed head

A **head** is a COSE_Sign1 signed by the primary, with this CBOR map:

| Key | Field    | Type                                                                           |
| --- | -------- | ------------------------------------------------------------------------------ |
| 1   | space    | 32-byte byte string                                                            |
| 2   | seq      | unsigned: the seq of the last entry                                            |
| 3   | hash     | 32-byte byte string: the hash of the last entry                                |
| 4   | segments | array of 32-byte byte strings: the hashes of the sealed segments, oldest first |
| 5   | primary  | 32-byte byte string: the primary's key                                         |

### 19.2 Segment

A **segment** holds a run of entries. Its bytes are, for each entry in order,
the CBOR head of an **unsigned integer** (major type 0) whose value is the
entry's length in bytes, followed by the entry's bytes. There is no count and
no header. The **name** of a segment is its hash.

A reader reads one CBOR item; it MUST be an unsigned integer (else
`segment: bad length`); that many bytes MUST follow (else
`segment: truncated`). It goes on until the bytes end.

Each sealed segment holds exactly **256** entries. Entries 0 to 255 are in the
first segment.

### 19.3 Checkpoint

A **checkpoint** is the state of a space at one seq. This version defines its
format, but nodes do not make or read checkpoints yet. It is a COSE_Sign1
signed by the primary, with this CBOR map:

| Key | Content                                                                                                                              |
| --- | ------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | space ID (32 bytes)                                                                                                                  |
| 2   | seq                                                                                                                                  |
| 3   | head (32 bytes)                                                                                                                      |
| 4   | primary key (32 bytes)                                                                                                               |
| 5   | info, as JSON text                                                                                                                   |
| 6   | members: array of `[node (32 bytes), identity (32 bytes), role (text), home (text or null)]`, sorted by node key                     |
| 7   | resources: array of `[target (text), payload (32 bytes or null), media type (text), seq, time, [history hashes…]]`, sorted by target |
| 8   | grants: array of `[grant ID (text), body as JSON text, revoked (bool)]`, sorted by ID                                                |

A checkpoint does not hold `nonces`.

## 20. Write conditions and idempotency

### 20.1 Write conditions

A write can carry conditions. The primary checks them at the moment it orders
the proposal (step 4 of section 18.2), against the target's current payload
hash `c` (or nothing). They are:

| Condition     | From                     | Fails when                                                                             | Result                            |
| ------------- | ------------------------ | -------------------------------------------------------------------------------------- | --------------------------------- |
| expected      | the node itself (PATCH)  | it is set and is not `c`                                                               | 412 `the resource has changed`    |
| If-Match      | the HTTP header, as is   | there is no `c`, or no item of the list equals `*` or the ETag of `c` exactly          | 412 `the resource has changed`    |
| If-None-Match | the HTTP header, as is   | there is a `c`, and an item (with any `W/` taken off) is `*` or equals the ETag of `c` | 412 `the resource already exists` |
| must exist    | the node itself (DELETE) | it is set and there is no `c`                                                          | 404 `resource not found`          |

They are checked in the order of the table. A list is split on `,`, and each
item is trimmed. If-Match compares strongly, so a `W/` tag never matches.

The **ETag** of a payload is `"` + the first 32 hex characters of its hash +
`"`.

**Wire form.** Between nodes the conditions are a CBOR array of exactly four
items: `[if_match (text or null), if_none_match (text or null), expected (32
bytes or null), must_exist (bool)]`.

### 20.2 Idempotency

A proposal's (author, nonce) can go in only once (section 18.2, step 3).
Clients use this through the HTTP header `Idempotency-Key`. For a write with
the key `K`, made under the grant with ID `G` (or `""` when the caller has no
grant):

- **nonce** = the first 16 bytes of the hash of the text `{G}\n{K}`;
- **request** (the fingerprint) = the hash of six fields, in this order: the
  method, the target (`type/id`), the `Content-Type` header, the `If-Match`
  header, the `If-None-Match` header, and the body. Each field is written as
  its length (8 bytes, little-endian) and then its bytes. A missing header is
  an empty field.

Without the header, the nonce is 16 random bytes and there is no fingerprint.

When an ordered proposal is a duplicate, the node compares the new proposal
with the one in the log at that seq. They are **the same request** when the
author, nonce, kind, target and grant are the same, and:

- if both have a fingerprint, the fingerprints are the same;
- if not, the payload hashes and the media types are the same.

The same request gets the first answer again, with "created" = false. A
different request gets 409 `idempotency key already used for a different
request`. Keys do not expire.

# Part V. The node and its data directory

## 21. The data directory

All of a node's state is in one directory. A copy of it (without the index)
is a full backup.

| Path                                           | Content                                                                   |
| ---------------------------------------------- | ------------------------------------------------------------------------- |
| `config.json`                                  | configuration (section 22)                                                |
| `identity`                                     | the identity log (section 12.2)                                           |
| `keys/node`                                    | the node key's seed: 64 hex characters, no newline                        |
| `keys/storage`                                 | the storage key: 64 hex characters, no newline                            |
| `keys/oidc`                                    | the OIDC signing key, PKCS#8 (made when first needed)                     |
| `keys/forward`                                 | the forward-auth secret, 32 bytes (made when first needed)                |
| `keys/vapid`                                   | the Web Push key, PKCS#8 (made when first needed)                         |
| `keys/self-signed.pem`, `keys/self-signed.key` | the TLS certificate and key (made when first needed)                      |
| `node.lock`                                    | an empty file; the running process holds an exclusive lock on it          |
| `logs/{space ID}/`                             | the log of one space (section 25)                                         |
| `blobs/{h0h1}/{h2…h63}`                        | a blob, named by the hex of its hash (section 24)                         |
| `index.db`, `index.db-wal`, `index.db-shm`     | the derived index (section 27)                                            |
| `access.log`                                   | the HTTP access log (section 30.6)                                        |
| `buckets/{bucket}/…`                           | objects that other nodes keep here through the S3 endpoint (section 49.2) |

Files under `keys/` are made with mode 0600 on Unix, but not the TLS files
`keys/self-signed.pem` and `keys/self-signed.key`, which get the default mode
of the process. Temporary files start
with `.tmp-`; directories being built start with `.space-` or `.restore-`.
Readers MUST skip them.

**Writing a file safely.** Where this document says a file is "written
atomically", the node writes a temporary file `.tmp-…` in the same directory,
flushes it to disk (`fsync`), and renames it over the final name. Where it says
"durably", the node then also flushes each directory from the file's parent up
to the root of that store.

## 22. Configuration

`config.json` is a JSON object, written pretty. The node keeps every member,
and it uses these:

| Member               | Type             | Meaning                                                                           |
| -------------------- | ---------------- | --------------------------------------------------------------------------------- |
| `identity`           | text, a key ID   | the identity ID; required                                                         |
| `private`            | text, 64 hex     | the ID of the private space; required                                             |
| `name`               | text             | the node's name, shown to people; default `""`                                    |
| `url`                | text or null     | the public base URL of the node, such as `https://home.example.org`; default none |
| `replicas`           | array of text    | replica addresses (section 45); items that are not text are skipped               |
| `s3_keys`            | array of objects | keys for the S3 endpoint (section 49.2)                                           |
| `bitwarden_register` | bool             | `true` lets one vault account be made (section 33)                                |
| `forward_domain`     | text             | the domain whose apps forward auth covers (section 35.5)                          |
| `push_subject`       | text             | the VAPID subject (section 36.2)                                                  |

When a member is set, the node checks the whole object again: `identity` MUST
be a key ID and `private` MUST be 64 hex characters. If the check or the write
fails, nothing changes. The file is written atomically.

## 23. Making and opening a node

### 23.1 Making a node (`init`)

Given a name and maybe a URL:

1. If `config.json` is there, stop: `{dir} already holds a node`.
2. Make `keys/`.
3. Make the personal seed and the node key seed (32 random bytes each). Take
   one time `t`.
4. Make the identity log: `create` signed by the personal key, then `add-node`
   with the node key, then `address` with the URL if there is one. All have
   time `t`.
5. Derive the storage key from the personal seed.
6. Write `keys/node` and `keys/storage` (each MUST be a new file, mode 0600,
   flushed to disk).
7. Write `identity`.
8. Make the genesis of the private space (section 16.3), with info
   `{"name": "private", "kind": "private"}`.
9. Write `config.json`: `{"identity", "private", "name", "url"}`.
10. Open the node (section 23.2), and start the private space with its
    genesis. This node is its primary.
11. Make a grant with role `owner`, all spaces, label
    `owner (created by hs ctl init)` (section 29).
12. Give back the recovery code (section 11.1) and the owner token. The
    personal seed is then forgotten.

### 23.2 Opening a node

1. Make `node.lock` and take an exclusive, non-blocking lock on it. If another
   process holds it, stop: `another hs process is using {dir}`. The lock is
   held while the node is open.
2. Read `config.json` and check it (section 22).
3. Read `keys/node` and `keys/storage`. Each MUST be 64 hex characters, with
   white space at the ends taken off (else `keys/{name} is damaged`).
4. Read and check `identity` (section 12.3).
5. Open the blob store.
6. Open the index (section 27.1). It may need a full rebuild.
7. For each directory in `logs/` whose name is 64 hex characters, in sorted
   order: load the log (section 25.3), and replay it (section 18.5). The
   index's record of this space gives the last seq it has. Each entry after
   that seq is given to the index (all of them after a rebuild).
8. Erase the blobs named by every erase in every log (section 26.3).

The node does not tell listeners about entries during opening.

## 24. Blobs

A blob is kept at `blobs/{first two hex characters}/{other 62 hex
characters}` of its hash.

- **Writing.** Hash the bytes. If a file is already at that name, do nothing.
  Else write it atomically.
- **Reading.** Read the file. If the hash of its bytes is not the name, treat
  it as missing.
- A blob has no size limit of its own. The HTTP body limit (section 30.1)
  limits uploads.

## 25. Space logs on disk

### 25.1 Files

`logs/{space ID}/` holds:

| File              | Content                                                                                                     |
| ----------------- | ----------------------------------------------------------------------------------------------------------- |
| `tail`            | the entries that are not sealed yet, each **framed** (below), end to end                                    |
| `segments/{hash}` | a sealed segment (section 19.2), named by the hex of its hash                                               |
| `segments.txt`    | the list of sealed segments: one 64-hex hash per line, each line ending in `\n`, oldest first               |
| `home`            | only on a node that is not the primary: the URL of the primary, UTF-8 (white space at the ends is not used) |

A **frame** is the CBOR head of an unsigned integer (the entry's length), then
the entry's bytes. It is the same framing as in a segment.

### 25.2 Adding an entry

1. Open `tail` for adding, and note its length.
2. Write the frame, and flush the data to disk.
3. If that fails, cut `tail` back to the noted length and flush again. If that
   also fails, the log refuses all later writes until it is opened again
   (`space log must be opened again: a failed write could not be undone`).

**Sealing.** When the number of entries divided by 256 is more than the number
of segments, the node seals:

1. For each full group of 256 entries that is not sealed, write the segment
   atomically as `segments/{hash}`.
2. Flush `segments/`.
3. Write `segments.txt` atomically, with all segment hashes.
4. Flush the log directory.
5. Write `tail` again atomically, with only the entries after the sealed ones.
   If this step fails, it is not an error; loading (below) handles it.

### 25.3 Loading and recovery after a crash

1. If `segments.txt` is there: for each line that is not empty, the line MUST
   be a hash (else `{dir}: bad segment list`), and the file
   `segments/{line}` MUST have that hash (else `segment {line} is damaged`).
   Its entries are taken in order.
2. Read `tail` frame by frame. Stop at the first frame that is not whole, or
   whose length is not an unsigned integer. If bytes are left after the last
   whole frame, cut `tail` to the end of that frame. (A write that was cut by a
   crash goes away.)
3. Drop any `tail` entry whose seq is lower than the number of entries from
   the segments. (A crash between steps 4 and 5 of sealing leaves them.)
4. The result is the whole list of entries. Read `home` if it is there.

### 25.4 A new space log

To make the log of a new or newly joined space, the node builds it in
`logs/.space-XXXXXX`: it makes `segments/`, writes `home` if there is one,
adds every entry, and seals. Then it flushes the directory, renames it to
`logs/{space ID}`, and flushes `logs/`.

## 26. The write path in a node

### 26.1 Commit

To **commit** a signed proposal with its payload and conditions to a space:

1. Keep the payload as a blob (even if the proposal is refused later).
2. Take the space's write lock. Only one commit or apply runs at a time in a
   space. The lock is held until the disk work ends, even if the HTTP client
   goes away.
3. Finish any pending work (section 26.2).
4. If this node is not the primary, stop with "not the primary" and the
   space's `home` (section 42.7 sends the proposal there).
5. Order the proposal (section 18.2).
   - A **duplicate** gives the first result again, if it is the same request
     (section 20.2), with "created" = false. Else it is a conflict (409).
   - A **new** entry gives the seq, the entry hash, and "created" = true if
     the target had no current payload before.
6. Finish pending work again.

To **apply** an entry that came from the primary, the node takes the same lock,
finishes pending work, and applies the entry only if its seq is higher than the
space's seq (section 18.3). Lower seqs are ignored. A gap is refused.

### 26.2 Pending work

After an entry is on disk, three more things happen, in order:

1. The index is updated (section 27.3).
2. If the entry is an erase, the blobs are erased (section 26.3).
3. Listeners are told (an **event**, below). This is done for entries that
   this node ordered and for entries it applied one at a time from the primary.
   It is not done for the history that a node gets when it joins a space.

If one of these fails, it stays pending, and the next commit or apply in that
space does it first. A crash before it ends is fixed on open (section 23.2).

If the space has 256 or more unsealed entries, the node seals it (section
25.2) before this work.

### 26.3 Erasing

To erase a list of hashes, the node makes the set of all hashes that are still
used: every current payload and every history hash of every resource in every
space. It deletes each blob in the list that is not in that set.

### 26.4 Events

An event is `{space, seq, kind, target, entry}`, with a flag that says whether
this node ordered the entry. Events go to an in-memory channel that holds up to
1024 events. A listener that falls behind loses events. Event streams
(section 31.9), push notifications (section 36.3) and the sync pusher
(section 42.4) listen.

## 27. The derived index

The index is a SQLite database, `index.db`, in WAL mode, with `synchronous =
NORMAL` and a busy wait of 5 seconds. One connection writes; up to four
read-only connections read. The index is never copied to replicas, never sent
to other nodes, and never migrated. It is private to the node: another program
may keep it in any form, as long as the API gives the same answers.

### 27.1 Schema and rebuild

```sql
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE spaces(space TEXT PRIMARY KEY, name TEXT NOT NULL, seq INTEGER NOT NULL);
CREATE TABLE resources(
  id INTEGER PRIMARY KEY,
  space TEXT NOT NULL, target TEXT NOT NULL, type TEXT NOT NULL,
  payload TEXT, media_type TEXT NOT NULL, seq INTEGER NOT NULL,
  updated INTEGER NOT NULL, time INTEGER NOT NULL,
  title TEXT NOT NULL, body TEXT,
  UNIQUE(space, target));
CREATE INDEX resources_type ON resources(space, type, time);
CREATE INDEX resources_time ON resources(time);
CREATE TABLE links(src INTEGER NOT NULL, rel TEXT NOT NULL, dst_space TEXT NOT NULL, dst TEXT NOT NULL);
CREATE INDEX links_src ON links(src);
CREATE INDEX links_dst ON links(dst_space, dst);
CREATE VIRTUAL TABLE fts USING fts5(text, content='', contentless_delete=1, tokenize='unicode61');
CREATE VIRTUAL TABLE places USING rtree(id, min_lat, max_lat, min_lon, max_lon);
```

`meta` holds `('schema', '1')`. The schema version is 1. When the node opens
the index and the file is missing, or cannot be read, or has another schema
version, the node deletes `index.db`, `index.db-wal` and `index.db-shm`, makes
the schema, and gives every entry of every log to it again.

The columns: `space` is the space ID in hex; `target` is `type/id`; `type` is
the part before the first `/`; `payload` is the payload hash in hex (or NULL);
`seq` is the seq of the last change; `updated` is that entry's time; `time` is
the time the resource is about (below); `title` is a short title; `body` is the
JSON of the resource, when it is JSON. `fts.rowid`, `places.id` and
`links.src` are `resources.id`.

### 27.2 Spaces

For a genesis entry, the node sets the row `(space, name, seq)` in `spaces`.
For every entry, it sets that space's `seq` to the entry's seq.

### 27.3 Giving an entry to the index

In one transaction:

1. If the kind is `put`, `delete` or `erase`: if a row for (space, target) is
   there, delete its rows in `fts`, `places`, `links` and `resources`.
2. If the kind is `put`, add the new row (keeping the old `id` if there was
   one), as below.
3. Set `spaces.seq`.

So after a delete or an erase, the resource has no row at all. The other
kinds change only `spaces.seq`.

**Making a row.** Let `bytes` be the payload (it may be missing):

1. `json` = the bytes as JSON, if the media type starts with
   `application/json` or ends with `+json`, and they parse.
2. `text` = the bytes as UTF-8, if there is no `json` and the media type
   starts with `text/`.
3. **title**: from `json`, the first of the members `title`, `name`, `fn`,
   `summary`, `subject` that is a string (first 200 characters); else the
   member `text` (first 80 characters); else `""`. From `text`: the first line
   that starts with `FN:` or `SUMMARY:` (the rest, 200 characters); else the
   first line that is not blank, with leading `#` and white space taken off
   (80 characters).
4. **time**: `json.time` if it is an unsigned number (ms), or a string of the
   form below; else the entry's time.
5. Write the row with `body` = `json` written again (section 9), or NULL.
6. **Search text**, unless the type is `cipher`, `grant`, `push` or
   `credential`: from `json`, every string value, depth first, each followed
   by `\n`, but not values whose member name is `href`, `hash`, `id`, `type`,
   `rel`, `media_type` or `time`, and nothing inside a member named `links`.
   Strings in arrays take the array's member name. It stops adding when the
   text is longer than 64 KiB. From `text`: the whole text. If the text is not
   blank, add `bigrams(text)` (section 27.4) to `fts` with the row's id.
7. **Place**: if `json.place.lat` and `json.place.lon` are numbers with
   −90 ≤ lat ≤ 90 and −180 ≤ lon ≤ 180, add the point to `places`.
8. **Links**: for each item of the array `json.links`: `rel` is its text
   member `rel` (default `related`); `href` MUST be text. If `href` is
   `space:{space}/{target}`, the link goes to that space and target. Else it
   goes to `href` in the same space. The link is kept only if its target is
   valid (section 15.2).

**Time strings.** `YYYY-MM-DD`, then maybe `T` or a space and `hh:mm`, then
maybe `:ss`. Month 1–12 and day 1–31 are checked. Anything after (parts of a
second, `Z`, an offset) is not used, and the time is read as UTC. Examples:
`1970-01-01` → 0; `2000-03-01T00:00:00Z` → 951868800000.

### 27.4 Search for Chinese, Japanese and Korean

The `unicode61` tokenizer does not cut CJK text into words. So the node puts
the text through `bigrams` both when it indexes and when it searches:

- These code points count as CJK: U+3040–U+30FF, U+3400–U+4DBF,
  U+4E00–U+9FFF, U+AC00–U+D7AF, U+F900–U+FAFF, U+20000–U+3134F.
- Each longest run of CJK characters is changed. A run of one character `c`
  becomes `c`. A longer run becomes each pair of side-by-side characters,
  each pair with a space before it, and then one space. Other characters stay
  as they are.
- Example: `人民公园` → `人民 民公 公园`; `去a` → ` 去 a`.

**Query.** The node puts the query through `bigrams`, then splits it at every
character that is not a letter or digit, and drops empty words. Each word
becomes `"word"` (with any `"` taken out). A word with no CJK character also
gets `*` (a prefix search). The words are joined with spaces, so all must
match, in any order. If there are no words, the search gives no results.
Example: `公园 walk` → `"公园" "walk"*`.

### 27.5 Queries

| Query                   | Rule                                                                                                                                                                                                                             |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| one row                 | `space` and `target` equal                                                                                                                                                                                                       |
| list                    | rows of one space, maybe of one type, maybe only of given types; `ORDER BY time DESC, target`; limit and offset                                                                                                                  |
| search                  | rows in the given spaces that match the FTS query; best first by `bm25`; a limit, no offset                                                                                                                                      |
| near (lat, lon, km)     | rows in the given spaces whose point is in the box lat ± km/111, lon ± km/(111 · max(\|cos lat\|, 0.01)); nearest first by (Δlat² + Δlon²) in degrees; a limit                                                                   |
| timeline                | rows in the given spaces with `time` < before (no before: all), not of the types `grant`, `push`, `cipher`, `credential`, `oauth-client`, `vault-folder`, `vault-account`; `ORDER BY time DESC, space, target`; limit and offset |
| related (space, target) | the rows that link to this target, together with the rows (in any space) that this resource links to; newest `time` first; a limit                                                                                               |
| counts                  | the number of rows of each type                                                                                                                                                                                                  |
| healthy                 | `PRAGMA quick_check` gives `ok`                                                                                                                                                                                                  |

# Part VI. Access

## 28. Who is asking

Every HTTP request is from one of three kinds of caller:

| Kind   | How it is known                                                                                       | What it may do                                                                 |
| ------ | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| public | no good token and no good signature                                                                   | public endpoints only                                                          |
| grant  | a token that matches a grant                                                                          | what the grant gives (section 29.3)                                            |
| node   | a good HTTP Message Signature (section 40) by a node key that is a member of at least one local space | in each such space, its member role; only the federation endpoints and the API |

The node decides the kind like this:

1. **Token.** Take the `Authorization` header:
   - `Bearer {token}` (this exact word, one space): the token is the rest,
     with white space at the ends taken off.
   - `Basic {base64}`: decode it (base64url or base64, with or without `=`);
     it MUST be UTF-8 with a `:`. The token is the part after the first `:`
     (the **password**). The user name is not used. If the decoding fails,
     there is no token.
   - Without a usable `Authorization` header, the cookie `hs_session` (its
     value as is) is the token.
2. If there is a token, find its grant (section 29.4). If there is none, the
   caller is public. A signature is then not looked at.
3. Else, if there is a `Signature-Input` header, check the signature
   (section 40.3) with a window of 300 seconds. If it is good, the caller is the
   node with that key. Its roles are its member roles in the local spaces. If
   it has none, the caller is public.
4. Else the caller is public.

## 29. Grants and tokens

### 29.1 The grant body

A grant is a `grant` entry (section 17.3) in the **private space**, at target
`grant/{id}`. Its JSON body has these members:

| Member    | Type                   | Meaning                                                                                |
| --------- | ---------------------- | -------------------------------------------------------------------------------------- |
| `id`      | text                   | the grant ID: base32 of 10 random bytes (16 characters)                                |
| `role`    | text                   | `reader`, `writer` or `owner`                                                          |
| `label`   | text                   | words for people                                                                       |
| `token`   | text                   | the token hash (section 29.2)                                                          |
| `created` | ms                     | when it was made                                                                       |
| `spaces`  | array of hex space IDs | if there, the grant covers only these spaces; items that are not valid IDs are skipped |
| `types`   | array of text          | if there, the grant covers only resources of these types                               |
| `expires` | ms                     | if there, the grant does not work after this time                                      |
| `session` | bool                   | `true` for a sign-in session (it is revoked when the session signs out)                |

Other members are kept and are not used. A grant is revoked by a `revoke`
entry at `grant/{id}` in the private space. A revoked grant stops working at
once.

### 29.2 Tokens

- A token is `hs_` followed by the base32 of 20 random bytes (35 characters).
- The node keeps only the **token hash**: the hex of
  `derive_key("homeserver 2025 token v1", token bytes)`.
- The token is shown one time, when the grant is made.

### 29.3 What a grant allows

For a space `S` and a target of type `T`:

- The grant's **role in S** is its `role` if it has no `spaces`, or if `S` is
  in `spaces`. Else it has no role in S.
- It may **read** S if it has a role in S.
- To **write**, the role MUST be at least the role that the kind needs
  (section 14.1). If the grant has `types`, `T` MUST be in it. The errors are
  403 `no access to this space`, `needs {role}`, and `grant does not cover {T}`.
- On reads, resources whose type is not in `types` are hidden.
- A grant is the **owner** of the node when its role is `owner` and it has no
  `spaces`. Some endpoints need the owner.

A node caller has its member role in each space where it is a member, and no
type limits. A node caller is never the owner.

The node itself uses an inner grant with ID `local`, role owner and no limits,
for writes it makes for itself and for the command line on a stopped node.

### 29.4 Finding the grant of a token

1. Compute the token hash.
2. Look at each grant that is not revoked, in order of grant ID. Skip a grant
   with no text `token`. Compare the hashes in constant time.
3. If the grant has `expires` and `expires` < now (ms), skip it.
4. The grant MUST have a text `id` and a valid `role`. The first grant that
   passes is the answer.

### 29.5 Sessions

A browser keeps a token in the cookie `hs_session`. The token can be of any
grant (`POST /api/session`, section 31.1), or of a grant that a passkey sign-in
made for the browser (section 35.4). A grant made for a sign-in has
`session: true`, and signing out revokes it. The cookie is:

```
hs_session={token}; Path=/; HttpOnly; SameSite=Lax[; Secure][; Max-Age={s}]
```

`Secure` is added only when the request itself came over TLS. To clear the
cookie, the node sends the same cookie with an empty value and `Max-Age=0`.

# Part VII. The HTTP interface

## 30. The HTTP server

### 30.1 Connections

- The node listens on one TCP address (default `127.0.0.1:8080`).
- It speaks HTTP/1.1 and HTTP/2. On plain TCP, a client may start HTTP/2 with
  prior knowledge. Connection upgrades are allowed.
- **TLS** is off, or on with a self-signed certificate. With TLS, the node uses
  TLS 1.3 or 1.2, offers ALPN `h2` and `http/1.1`, and does not ask for client
  certificates. A TLS handshake that takes more than 10 seconds is dropped. The
  certificate is made one time, for the names given on the command line (or
  the host of `url`) and also `localhost` and `127.0.0.1`. It is kept in
  `keys/self-signed.pem` and `keys/self-signed.key`, and used again on later
  starts. For a public domain with a certificate from an authority, a reverse
  proxy goes in front of the node. The node does not run ACME.
- A **request body** is read whole before the request is handled. It may be at
  most 256 MiB. A larger body (or a failed read) gets 413 with the body
  `request body too large\n`.
- If accepting a connection fails, the node waits 50 ms and goes on.
- On stop (SIGINT), the node stops accepting. It does not wait for open
  connections.

### 30.2 Routing

The path is split on `/`. Empty parts are dropped, so `//api//spaces/` is
`/api/spaces`. Each part is percent-decoded (`+` stays `+`), so `%2F` stays
inside one part. The query is not used for routing. The first part chooses:

| Path                                                       | Goes to                                         |
| ---------------------------------------------------------- | ----------------------------------------------- |
| `/api/…`                                                   | the REST API (section 31)                       |
| `/fed/…`                                                   | the federation endpoints (section 41)           |
| `/dav/…`                                                   | CardDAV and CalDAV (section 32)                 |
| `/.well-known/carddav`, `/.well-known/caldav`              | any method: 301 to `/dav/`                      |
| `/mcp`                                                     | MCP (section 34)                                |
| `/s3/…`                                                    | the S3 endpoint (section 49.2)                  |
| `/bitwarden/…`                                             | the vault (section 33)                          |
| `/.well-known/homeserver`                                  | any method: the node information (section 39.1) |
| `/auth/…`, `/oauth/…`, `/.well-known/openid-configuration` | sign-in (section 35)                            |
| anything else, GET or HEAD                                 | the web app files (section 38)                  |
| anything else                                              | 405 `method not allowed`                        |

### 30.3 Common response headers

Every response gets `X-Content-Type-Options: nosniff` and
`Referrer-Policy: same-origin` if it does not have them. There is no CORS
support and no rate limit.

### 30.4 Errors

Unless a section says something different, an error is JSON
`{"error": "{message}"}` with `Content-Type: application/json`. A 401 on a
path under `/dav` also gets `WWW-Authenticate: Basic realm="homeserver"`.

| Status | When                                                                                                                                       |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| 400    | a bad request: `bad request: …` (JSON body), `bad JSON: …`, `bad query: …`, or a message of the endpoint                                   |
| 401    | `sign in first`: the endpoint needs a caller that is not public                                                                            |
| 403    | the caller does not have the right: `no access to this space`, `needs {role}`, `grant does not cover {type}`, `only the owner can do this` |
| 404    | `not found`, `space {name}`, `resource not found`                                                                                          |
| 409    | `idempotency key already used for a different request`                                                                                     |
| 410    | `deleted`: the resource was deleted, or its bytes are not on this node                                                                     |
| 412    | `the resource has changed`, `the resource already exists`                                                                                  |
| 415    | the body or resource is of a kind the endpoint cannot use                                                                                  |
| 421    | `this node is not the primary of {space}`: the write needs the primary, and this node does not know where it is                            |
| 422    | the space rules refused the change (a message of section 18) or `{name} matches more than one space`                                       |
| 500    | a disk or index failure (`index: …`)                                                                                                       |
| 502    | another node could not be reached, or gave a bad answer                                                                                    |
| 503    | a needed key or worker is not there                                                                                                        |

### 30.5 Naming a space in a path

Where a path has `{space}`, it can be:

1. 64 hex characters: that space ID;
2. else the exact name of one space, or the start of one space ID in hex.

No match gives 404 `space {name}`. More than one match gives 422. A space that
the caller cannot read gives 404.

### 30.6 The access log

The node adds one line of JSON to `access.log` for each request (also errors
and web files), after the handler ends:

```json
{
  "event.name": "http.request.completed",
  "message.template": "{{http.request.method}} {{http.route}} returned {{http.response.status_code}} in {{ms}} ms",
  "time": 1764079380000,
  "http.request.method": "GET",
  "http.route": "/api",
  "http.response.status_code": 200,
  "ms": 3
}
```

- `time` is when the request started (ms). `ms` is how long the handler took.
- The method is one of `GET`, `HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`,
  `OPTIONS`, `PROPFIND`, `REPORT`, or `OTHER`.
- The route is only the first part of the path, as it was sent (it is not
  percent-decoded): `/api`, `/auth`, `/bitwarden`,
  `/dav`, `/fed`, `/mcp`, `/oauth`, `/s3`, `/.well-known`, or `/` for anything
  else.
- The log has no addresses, no query strings, no tokens and no hashes. It is
  never rotated.
- A queue of 1024 lines feeds a writer that writes up to 64 lines at a time.
  When the queue is full, requests wait. If a write fails, those lines are lost,
  and the file is opened again for the next lines.

## 31. The REST API

Request and response bodies are JSON, unless a row says something different.
"Signed in" means a caller that is not public. "Owner" means the owner grant
(section 29.3). The API is also described as an OpenAPI 3.1 document at
`GET /api/openapi.json`.

Any method that is not in the tables on a path that starts with `session`,
`spaces`, `blobs`, `grants` or `replicas` gets 405. Other unknown paths get 404. HEAD works only where the tables list it.

### 31.1 Session, node and identity

| Method and path         | Who       | Request     | Response                                                                                 |
| ----------------------- | --------- | ----------- | ---------------------------------------------------------------------------------------- |
| `GET /api/session`      | signed in | –           | 200 Session                                                                              |
| `POST /api/session`     | anyone    | `{"token"}` | 200 Session, and the cookie `hs_session` with no Max-Age; 401 `unknown token`            |
| `DELETE /api/session`   | anyone    | –           | 204, and the cookie is cleared; if the caller's grant has `session: true`, it is revoked |
| `GET /api/node`         | anyone    | –           | 200 NodeInfo (section 39.1)                                                              |
| `GET /api/identity`     | anyone    | –           | 200 the DID document (section 13), `Content-Type: application/did+json`                  |
| `GET /api/health`       | signed in | –           | 200 Health                                                                               |
| `GET /api/openapi.json` | anyone    | –           | 200 the OpenAPI document                                                                 |

- **Session**: `{"role": "reader"|"writer"|"owner"|null, "grant"?, "owner"?,
"spaces"?, "types"?, "node"?, "roles"?}`. For a grant: `grant` (its ID),
  `role`, `owner` (bool), `spaces` (hex IDs, if limited), `types` (if limited).
  For a node: `role` is null, `node` is its key ID, and `roles` is
  `[{"space", "role"}]`. Members marked `?` are left out when they have no
  value.
- **Health**: `{"ok", "index", "counts", "spaces", "time"}`. `index` is the
  result of the index check (section 27.5), and `ok` is the same. `counts` maps
  each type to its number of resources. `spaces` is the number of spaces.
  `time` is now (ms).

### 31.2 Spaces and members

| Method and path                             | Who                     | Request             | Response                                                                    |
| ------------------------------------------- | ----------------------- | ------------------- | --------------------------------------------------------------------------- |
| `GET /api/spaces`                           | signed in               | –                   | 200 `[Space]`: the spaces the caller can read                               |
| `POST /api/spaces`                          | owner                   | `{"name", "info"?}` | 201 Space                                                                   |
| `POST /api/spaces/join`                     | owner                   | `{"home", "space"}` | 201 Space (section 42.3)                                                    |
| `GET /api/spaces/{space}`                   | reader                  | –                   | 200 Space                                                                   |
| `POST /api/spaces/{space}/members`          | owner role in the space | NewMember           | 201 `{"seq", "member"}`                                                     |
| `DELETE /api/spaces/{space}/members/{node}` | owner role              | –                   | 204                                                                         |
| `POST /api/spaces/{space}/handover`         | owner role              | `{"node"}`          | 200 `{"seq"}`                                                               |
| `POST /api/spaces/{space}/sync`             | reader                  | –                   | 200 `{"pulled"}`: the number of entries got from the primary (section 42.6) |

- **Space**: `{"id", "name", "info", "seq", "primary", "is_primary", "home",
"members"}`. `id` is hex. `primary` is a key ID. `is_primary` says whether
  this node is the primary. `home` is the URL of the primary as this node knows
  it, or null. `members` is `[{"node", "identity", "role", "home"}]`, and
  `home` may be null.
- **Making a space.** An empty `name` gives 400 `name missing`. The info is
  `info` (an object; anything else becomes `{}`) with `name` set. The node
  makes the genesis (section 16.3) and is the primary. Then, if `url` is set in
  the configuration, the node writes a member entry for itself at
  `member/{its key ID}` with `"home": url`, so that other members can find it.
- **Adding a member.** NewMember is `{"node"?, "identity"?, "role"?, "home"?,
"url"?}`. If `url` is given and `node` is not, the node asks
  `GET {url}/.well-known/homeserver` (section 39.1) and takes `node` and
  `identity` from the answer. A failure gives 502 (`not a homeserver` when the
  answer is not node information). Without a node it is 400
  `node or url needed`; without an identity it is 400 `identity needed`. The
  role is `writer` if not given. The home is `home`, or else `url`. The node
  writes a `member` entry (section 17.2) at `member/{node}` (section 42.7 if it
  is not the primary).
- **Removing a member.** `{node}` MUST be a key ID (else 400 `bad node id`) and
  a member (else 404). The node writes a member entry with `"remove": true`.
- **Handover.** The node writes a `handover` entry at `space/primary` with
  `{"primary": node}` (section 42.8).

### 31.3 Resources

Paths: `/api/spaces/{space}/resources/{type}/{id}`. The target `{type}/{id}`
MUST be valid (section 15.2), else 400 `bad type or id`.

| Method                                       | Who                                       | Response                                       |
| -------------------------------------------- | ----------------------------------------- | ---------------------------------------------- |
| `GET`, `HEAD`                                | reader of the space, and the type allowed | 200 representation; 304; 404; 410              |
| `PUT`                                        | writer                                    | 201 (made) or 200 (changed), `{"seq", "etag"}` |
| `PATCH`                                      | writer                                    | 200 `{"seq", "etag"}`                          |
| `DELETE`                                     | writer                                    | 204                                            |
| other                                        | –                                         | 405                                            |
| `POST /api/spaces/{space}/erase/{type}/{id}` | writer                                    | 204                                            |

**GET and HEAD.**

1. If the grant does not cover the type, 404.
2. No such resource: 404. A resource whose payload is gone (deleted, erased,
   or the blob is not on this node): 410 `deleted`.
3. `If-None-Match`: if any item (with `W/` taken off) is `*` or equals the ETag,
   304 with only `ETag`.
4. Choose the representation. The query `format` wins if it is there.
   Else, if `Accept` contains `text/vcard`, it is `vcard`; else if it contains
   `text/calendar`, it is `ical`. If the choice is `vcard` or `ical` and the
   stored media type is exactly `application/json`, the body is the JSON in
   that form (section 31.4), with `Content-Type: text/vcard; charset=utf-8` or
   `text/calendar; charset=utf-8`. Else the body is the stored bytes with the
   stored media type (`application/octet-stream` if it is empty).
5. Headers: `ETag` (section 20.1; the same for every representation),
   `Last-Modified` (the resource's `time`, as an HTTP date),
   `Cache-Control: private, no-cache`. HEAD has no body.

**PUT.**

1. The media type is the `Content-Type` (default `application/json`), with
   parameters taken off, trimmed and in lowercase.
   - `text/vcard` or `text/x-vcard`: the body becomes contact JSON
     (section 31.4).
   - `text/calendar`: the body becomes event JSON (section 31.4).
   - Else the bytes are kept as they are, with that media type.
2. If the media type is `application/json` or ends with `+json`, the body MUST
   be JSON (else 400). If the type is `file`, the JSON is filled in from the
   blob (section 37.3). The JSON is written again (section 9), and the media
   type becomes `application/json`.
3. The conditions are `If-Match` and `If-None-Match` from the headers. The
   idempotency values come from `Idempotency-Key` (section 20.2).
4. The node writes a `put` (section 42.7).
5. 201 if the target had no current payload before, else 200. The body is
   `{"seq", "etag"}`, and the `ETag` header is set.

**PATCH** (JSON Merge Patch, RFC 7396).

1. With an `Idempotency-Key` that this node has already committed: the same
   request gets the first answer (200) again, and a different request gets 409.
2. The body MUST be JSON (else 400).
3. No resource, or no payload: 404. A stored media type that is not
   `application/json`: 415 `only JSON resources can be patched`.
4. Apply the merge patch: an object patch makes the target an object; a `null`
   member removes that member; objects are merged member by member; any other
   patch takes the place of the target.
5. Write it like PUT, with the condition "expected" = the current payload hash
   (so a change between the read and the write gives 412), and also
   `If-Match` and `If-None-Match`.

**DELETE.** The node writes a `delete` with no payload and an empty media type,
with `If-Match`, `If-None-Match` and "must exist". A resource that is not there
or is already deleted gives 404.

**Erase.** The node writes an `erase` with no payload. There are no conditions
and no idempotency.

### 31.4 Contacts and events as vCard and iCalendar

Contacts and events are kept as JSON. These rules change them to and from
vCard (RFC 6350) and iCalendar (RFC 5545).

**Reading text.** Split into lines at `\n`, and take off a `\r` at the end. A
line that starts with a space or a tab is added to the line before it, without
that character. Empty lines are dropped. The value starts after the first `:`
that is not inside double quotes; a line with no such `:` is dropped. The name
is the part before the first `;`, after the last `.` (a group prefix such as
`item1.` goes away), in uppercase. In values, `\n` and `\N` become a newline,
and `\x` becomes `x`.

**Writing text.** In values, `\` becomes `\\`, a newline becomes `\n`, `,`
becomes `\,`, and `;` becomes `\;`. A line longer than 75 bytes is folded with
`\r\n` and a space. Every line ends with `\r\n`.

**Contact JSON**: `{"name", "given", "family", "emails": [], "phones": [],
"org", "title", "note", "birthday", "address", "url", "uid", "extra": []}`.

vCard out, lines in this order:

```
BEGIN:VCARD
VERSION:4.0
UID:{uid, or the id part of the target}
FN:{name, or the target}
N:{family};{given};;;          (only if family or given)
EMAIL:{each of emails}
TEL:{each of phones}
ORG:{org}  TITLE:{title}  NOTE:{note}  BDAY:{birthday}  URL:{url}   (each only if there)
ADR:;;{address};;;;            (only if there)
{each line of extra, as it is}
END:VCARD
```

vCard in: `BEGIN`, `END`, `VERSION`, `PRODID` and `REV` are not used. `FN` →
`name`. `N` → `family` (part 1) and `given` (part 2). `EMAIL` → `emails`
(parameters are lost). `TEL` → `phones` (a leading `tel:` is taken off). `ORG`
(with `;` changed to `, `) → `org`. `TITLE`, `NOTE`, `BDAY`, `URL`, `UID` → their
members. `ADR` → the parts that are not empty, joined with `, ` → `address`.
Every other line is kept as it is in `extra`. With no `FN`, `name` is `given`
and `family` joined by a space. Lists that are empty are left out. If the text
has more than one card, their lines all go into one object.

**Event JSON**: `{"title", "start", "end", "all_day", "location",
"description", "rrule", "uid", "time", "extra": []}`.

iCalendar out:

```
BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//homeserver//homeserver//EN
BEGIN:VEVENT
UID:{uid, or the id part of the target}
DTSTAMP:19700101T000000Z
DTSTART:{start}   DTEND:{end}          (each only if there)
SUMMARY:{title, may be empty}
LOCATION:{location}  DESCRIPTION:{description}  RRULE:{rrule, not escaped}
{each line of extra}
END:VEVENT
END:VCALENDAR
```

When `all_day` is true, `DTSTART;VALUE=DATE:YYYYMMDD` (and the same for
`DTEND`). Else: a time with an offset (RFC 3339) is changed to UTC
`YYYYMMDDTHHMMSSZ`; a local time `YYYY-MM-DDTHH:MM[:SS[.f]]` becomes
`YYYYMMDDTHHMMSS`; a date `YYYY-MM-DD` becomes `YYYYMMDD`; other text stays as
it is.

iCalendar in: only the lines of the first `VEVENT`. `DTSTART` and `DTEND`:
`YYYYMMDD` → `YYYY-MM-DD` (a date), `YYYYMMDDTHHMMSS[Z]` →
`YYYY-MM-DDTHH:MM:SS[Z]`, other values stay as they are. Parameters such as
`TZID` are lost. `DTSTART` also sets `all_day` (true for a date) and `time`
(the same text). `SUMMARY` → `title`, `LOCATION`, `DESCRIPTION`, `RRULE` (as it
is), `UID`. `DTSTAMP`, `CREATED`, `LAST-MODIFIED` and `SEQUENCE` are not used.
Other lines in the `VEVENT` go into `extra`. Other components (also
`VTIMEZONE`) are not used.

### 31.5 Lists and pages

`GET /api/spaces/{space}/resources` (reader) and `GET /api/timeline` (signed
in) take:

| Query    | Meaning                                                                       |
| -------- | ----------------------------------------------------------------------------- |
| `type`   | only this type (resources only)                                               |
| `before` | only `time` < this (ms) (timeline only)                                       |
| `limit`  | at most this many; default 100 (resources) or 50 (timeline); at most 1000     |
| `offset` | skip this many; default 0                                                     |
| `page`   | `1` asks for a page object; any other value (0 to 255) asks for a plain array |

With `page=1` the answer is `{"items": [Row], "next_offset"}`. The node asks
the index for one more row than the limit. If there is one, `next_offset` is
offset + limit, else it is null. The grant's type limit is part of the query.

Without `page=1` the answer is `[Row]`. Rows whose type the grant does not
cover are taken out after the limit, so fewer rows than the limit may come
back.

The timeline covers every space that the caller can read. Section 27.5 gives
the order of both lists.

**Row**: `{"space", "target", "type", "payload", "media_type", "seq",
"updated", "time", "title", "body"}`. `payload` is the hex hash or null.
`body` is the JSON or null. The other members are as in section 27.1.

### 31.6 Finding

| Method and path                        | Who       | Query                                                                         | Response                                    |
| -------------------------------------- | --------- | ----------------------------------------------------------------------------- | ------------------------------------------- |
| `GET /api/search`                      | signed in | `q` (required), `limit` (default 50, at most 1000)                            | `[Row]`, best first, in all readable spaces |
| `GET /api/near`                        | signed in | `lat`, `lon` (required), `km` (default 1), `limit` (default 50, at most 1000) | `[Row]`, nearest first                      |
| `GET /api/related/{space}/{type}/{id}` | reader    | `limit` (default 100, at most 1000)                                           | `[{"rel", "row"}]`, newest first            |

Rows that the caller may not see (type not allowed; for related, also a space
it cannot read) are taken out after the limit.

### 31.7 Blobs

| Method and path                 | Who                   | Response                                              |
| ------------------------------- | --------------------- | ----------------------------------------------------- |
| `POST /api/blobs`               | a writer in any space | 201 `{"hash", "size"}`, `Location: /api/blobs/{hash}` |
| `GET`, `HEAD /api/blobs/{hash}` | signed in             | 200, 206, 304, 404, 416                               |

**Upload.** The body is the bytes; `Content-Type` is not used. A grant with role
writer or owner may upload, whatever its space list. A node caller must be a
writer in some space.

**Download.** Any signed-in caller can get any blob by its hash. Access is not
checked per space.

1. The hash MUST be 64 hex characters, and the blob MUST be there (else 404).
2. `ETag` is the ETag of the hash (section 20.1). `If-None-Match` with `*` or a
   matching tag (with `W/` taken off) gives 304.
3. The query `thumb={n}` asks for a thumbnail (section 37.4).
4. Else the media type is the query `type`, or `application/octet-stream`. If
   it contains `html`, `svg`, `xml` or `javascript`, it becomes
   `application/octet-stream`.
5. **Range.** Only one range, `bytes=a-b`, `bytes=a-` or `bytes=-n`, is used. A
   `Range` with a `,`, or not starting with `bytes=`, is not used. `a-b` is
   bytes a up to b (b is cut to the end); `a-` is from a to the end; `-n` is
   the last n bytes. The range must have start < end and start < length, else
   416 with `Content-Range: bytes */{length}` and no body.
6. Headers: `ETag`, `Content-Length`, `Accept-Ranges: bytes`,
   `Cache-Control: private, max-age=31536000, immutable`,
   `Content-Security-Policy: sandbox`, and for a range 206 with
   `Content-Range: bytes {start}-{end−1}/{length}`.
7. The body is sent in pieces of 64 KiB. For a whole blob, the node hashes the
   bytes as it sends them. If the hash is wrong at the end, it stops the
   response with an error before the last piece, so the client sees a broken
   transfer. A range is not checked.

### 31.8 Grants

| Method and path           | Who   | Request  | Response                                   |
| ------------------------- | ----- | -------- | ------------------------------------------ |
| `GET /api/grants`         | owner | –        | 200 `[Grant]`: grants that are not revoked |
| `POST /api/grants`        | owner | NewGrant | 201 `{"token", "id"}`                      |
| `DELETE /api/grants/{id}` | owner | –        | 204                                        |

- **Grant** (output): `{"id", "role", "label", "created", "spaces"?, "types"?,
"expires"?, "session"?}`. The token hash is not given.
- **NewGrant**: `{"role", "label"?, "spaces"?, "types"?, "expires"?,
"session"?}`. Each item of `spaces` names a space (section 30.5). The node
  makes a grant (section 29) and gives the token one time.

### 31.9 Events

`GET /api/events` (signed in) is a stream of Server-Sent Events (HTML Living
Standard), `Content-Type: text/event-stream`, `Cache-Control: no-cache`.

1. The first bytes are `retry: 3000\n\n`.
2. For each event (section 26.4) in a space that the caller can read, and of a
   type that it may see:

   ```
   id: {seq}
   data: {"space":"{hex}","seq":{seq},"kind":"{kind}","target":"{type/id}"}

   ```

   There is no `event:` line.

3. If no event comes for 25 seconds, the node sends `: keep-alive\n\n`.

The caller's access is fixed when the stream starts. `Last-Event-ID` is not
used, and missed events are not sent again; a client lists again after it
connects.

### 31.10 Backups

| Method and path           | Who   | Response                                                                |
| ------------------------- | ----- | ----------------------------------------------------------------------- |
| `GET /api/export`         | owner | 200 a tar file (section 50)                                             |
| `GET /api/replicas`       | owner | 200 `[text]`: the replica addresses, with secrets hidden (section 45.1) |
| `POST /api/replicas/sync` | owner | 200 `[ReplicaSync]` (section 47.3)                                      |

### 31.11 Push

| Method and path            | Who   | Request                    | Response                                                                               |
| -------------------------- | ----- | -------------------------- | -------------------------------------------------------------------------------------- |
| `GET /api/push/key`        | owner | –                          | 200 `{"key"}`: the VAPID public key, base64url of the 65-byte point; 503 `no push key` |
| `POST /api/push/subscribe` | owner | a browser PushSubscription | 201 `{"id": "push/{32 hex}"}`                                                          |
| `POST /api/push/test`      | owner | –                          | 200 `{"ok": true}`                                                                     |

Any other caller gets 401 on these paths, and other paths under `/api/push/`
get 404 for the owner. Section 36 gives the details.

## 32. Contacts and calendars (CardDAV and CalDAV)

Contacts and calendar clients reach the node at `/dav/`. They sign in with HTTP
Basic: any user name, and a token as the password.

### 32.1 Layout

```
/dav/                               the principal
/dav/contacts/                      the address book home
/dav/contacts/{space}/              one address book for each space
/dav/contacts/{space}/{id}.vcf      the resource contact/{id}
/dav/calendars/                     the calendar home
/dav/calendars/{space}/             one calendar for each space
/dav/calendars/{space}/{id}.ics     the resource event/{id}
```

- Every space that the caller can read is both an address book and a calendar.
- The node writes collection paths with the space ID in hex, and item names
  percent-encoded (all but `A-Z a-z 0-9 - . _ ~`). It accepts any form of
  section 30.5 for `{space}`.
- Items are kept as JSON. Section 31.4 changes them to and from vCard
  (`text/vcard; charset=utf-8`) and iCalendar (`text/calendar; charset=utf-8`).

### 32.2 Requests

1. `OPTIONS` (any path, no sign-in): 200 with
   `DAV: 1, 3, addressbook, calendar-access` and
   `Allow: OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, REPORT`.
2. A public caller gets 401 (with `WWW-Authenticate`).
3. `PROPFIND` and `REPORT` are below.
4. Other methods need the path `/{book}/{space}/{file}` (else 405 `not here`).
   An unknown book, a type the grant does not cover, or a file without the
   right extension gives 404.
   - `GET`, `HEAD`: the item as text. `If-None-Match` (a list, or `*`) that
     matches gives 304. Else 200 with `ETag`.
   - `PUT`: the body MUST contain `BEGIN:VCARD` (contacts) or
     `BEGIN:VCALENDAR` (calendars), else 415 `expected vCard or iCalendar`. It
     is changed to JSON and written as in section 31.3, with the same
     conditions and idempotency. 201 or 200, with the new `ETag`, and no body.
   - `DELETE`: as in section 31.3. 204.
   - Other methods: 405.

### 32.3 PROPFIND

- `Depth: 0` gives the resource only. Any other value or none counts as 1: the
  resource and its children.
- `/dav/`: the principal. Its children are the two homes.
- `/dav/{book}/`: the home. Its children are one collection for each readable
  space, if the grant covers the book's type (`contact` or `event`).
- `/dav/{book}/{space}/`: the collection. Its children are all items of the
  type, without their data.
- `/dav/{book}/{space}/{file}`: one item.

An empty body, or `allprop`, or a `propfind` without properties, asks for
`resourcetype`, `displayname`, `getetag`, `getcontenttype` and `getctag`.

The answer is 207 with `Content-Type: application/xml; charset=utf-8`, a
`multistatus` (RFC 4918) with prefixes `d` (`DAV:`), `c`
(`urn:ietf:params:xml:ns:caldav`), `a` (`urn:ietf:params:xml:ns:carddav`) and
`cs` (`http://calendarserver.org/ns/`). Each response has one `propstat` with
`HTTP/1.1 200 OK` for found properties. When properties were named, it has one
more with `HTTP/1.1 404 Not Found` for the others.

| Property                                                                                | Value                                                                                                                                                    |
| --------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `d:resourcetype`                                                                        | principal: `collection`, `principal`; home: `collection`; address book: `collection`, `a:addressbook`; calendar: `collection`, `c:calendar`; item: empty |
| `d:displayname`                                                                         | principal: the node's name; homes: `Contacts`, `Calendars`; collection: the space's name; item: not found                                                |
| `d:current-user-principal`, `d:principal-URL`, `d:owner`, `c:calendar-user-address-set` | `<d:href>/dav/</d:href>`                                                                                                                                 |
| `a:addressbook-home-set`                                                                | `/dav/contacts/`                                                                                                                                         |
| `c:calendar-home-set`                                                                   | `/dav/calendars/`                                                                                                                                        |
| `d:current-user-privilege-set`                                                          | read, write, write-content, bind, unbind (always)                                                                                                        |
| `d:supported-report-set`                                                                | address books: `addressbook-multiget`, `addressbook-query`; calendars: `calendar-multiget`, `calendar-query`; principal and homes: empty                 |
| `c:supported-calendar-component-set`                                                    | calendars: `<c:comp name="VEVENT"/>`                                                                                                                     |
| `a:supported-address-data`                                                              | address books: `<a:address-data-type content-type="text/vcard" version="3.0"/>`                                                                          |
| `cs:getctag`                                                                            | collections: `"{seq}"` of the space                                                                                                                      |
| `d:sync-token`                                                                          | collections: `urn:homeserver:{space hex}:{seq}`                                                                                                          |
| `d:getetag`                                                                             | items: the ETag                                                                                                                                          |
| `d:getcontenttype`                                                                      | items: the book's media type; others: `httpd/unix-directory`                                                                                             |
| `a:address-data`, `c:calendar-data`                                                     | items, when asked: the vCard or iCalendar text                                                                                                           |

Other properties are not found.

### 32.4 REPORT

The path MUST be at least `/dav/{book}/{space}`.

- `addressbook-multiget`, `calendar-multiget`: for each `href` (sorted, each
  once), the last part of the path names an item in **this** collection. A
  found item gets a normal response; a missing one gets a response with
  `HTTP/1.1 404 Not Found`.
- `addressbook-query`, `calendar-query`: every item of the type. Filters are
  not used.
- Other reports (also `sync-collection`): 403 `unsupported report`.

Data is given only when `address-data` or `calendar-data` is asked for.

## 33. Password vault (Bitwarden API)

The node has a small vault that Bitwarden apps can use, at the base URL
`{node}/bitwarden`. The apps encrypt everything. The node keeps their
encrypted data and does not see inside it.

### 33.1 Storage

All vault data is in the private space, as JSON:

- the account: `vault-account/main` (one account for each node);
- items: `cipher/{uuid}`;
- folders: `vault-folder/{uuid}`.

A **uuid** here is 16 random bytes as 32 hex characters `h`, written
`h[0..8]-h[8..12]-4h[13..16]-ah[17..20]-h[20..32]`. Times are written
`YYYY-MM-DDTHH:MM:SS.mmmZ`. In request bodies, a member name can be in
camelCase or with its first letter in uppercase.

The account object has `title: "Password vault"`, `id`, `email` (lowercase),
`name`, `hint`, `salt`, `password`, `key`, `publicKey`, `privateKey`, `kdf`
(default 0), `kdfIterations` (default 600000), `securityStamp`, `created`. The
node does not keep the master password hash that the app sends. It keeps
`password` = base64url of PBKDF2-HMAC-SHA256(that hash's text, `salt`,
100000 rounds, 32 bytes), with `salt` = 16 random bytes.

### 33.2 Endpoints without a session

| Method and path under `/bitwarden`                                     | Answer                                                                                                                                                               |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST identity/accounts/prelogin`, `POST api/accounts/prelogin`        | `{"kdf", "kdfIterations", "kdfMemory": null, "kdfParallelism": null}` from the account (or 0 and 600000)                                                             |
| `POST identity/accounts/register/send-verification-email`              | a JSON string of random base64url; nothing is sent                                                                                                                   |
| `POST api/accounts/register`, `POST identity/accounts/register/finish` | makes the account                                                                                                                                                    |
| `POST identity/connect/token`                                          | signs in (below)                                                                                                                                                     |
| `GET api/config`                                                       | `{"version": "2025.1.0", "gitHash": "", "server": {"name": "homeserver", "url": "https://github.com/"}, "environment": {}, "featureStates": {}, "object": "config"}` |
| `GET api/accounts/revision-date`                                       | the time now, in ms                                                                                                                                                  |
| `GET alive`, `GET api/alive`                                           | the time now as a JSON string, `"YYYY-MM-DDTHH:MM:SS.mmmZ"`                                                                                                          |

**Register.** It needs `bitwarden_register: true` in the configuration (else
403 `registration is closed; set bitwarden_register to true`), and no account
yet (else 403 `this node already has a vault account`). The body needs
`email`, `masterPasswordHash`, and `key` (or `userSymmetricKey`). It may have
`keys` (or `userAsymmetricKeys`) with `publicKey` and `encryptedPrivateKey`,
`kdf`, `kdfIterations`, `name` and `masterPasswordHint`. The answer is
`{"object": "register", "captchaBypassToken": null}`.

**Token** (a form body):

- `grant_type=password`: `username` (lowercased) MUST be the account's e-mail,
  and `password` MUST match. Else 400
  `Username or password is incorrect. Try again.`
- `grant_type=refresh_token`: `refresh_token` MUST be the token of a vault
  session that still works. The old grant is revoked.
- Errors are 400 `{"error": "invalid_grant", "error_description": m,
"ErrorModel": {"Message": m, "Object": "error"}}`.
- On success the node makes a grant: role writer, only the private space,
  types `cipher`, `vault-folder`, `vault-session`, `session: true`, label
  `Bitwarden app`, and an end 30 days later. The answer (with
  `Cache-Control: no-store`) is `access_token` and `refresh_token` (both that
  grant's token), `expires_in: 3600`, `token_type: "Bearer"`,
  `scope: "api offline_access"`, `Key`, `PrivateKey`, `Kdf`, `KdfIterations`,
  `KdfMemory: null`, `KdfParallelism: null`, `ResetMasterPassword: false`,
  `ForcePasswordReset: false`,
  `MasterPasswordPolicy: {"object": "masterPasswordPolicy"}` and
  `UserDecryptionOptions: {"HasMasterPassword": true, "Object": "userDecryptionOptions"}`.

### 33.3 Endpoints with a session

The caller MUST have a grant whose types include `vault-session`, or be the
owner (else 401). With no account, 404.

| Method and path                                           | Answer                                                                                                                 |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `GET api/sync`                                            | `{"object": "sync", "profile", "folders", "collections": [], "ciphers", "domains": null, "policies": [], "sends": []}` |
| `GET api/accounts/profile`                                | the profile                                                                                                            |
| `POST api/ciphers`, `POST api/ciphers/create`             | makes an item with a new uuid                                                                                          |
| `PUT` or `POST api/ciphers/{id}`                          | makes or replaces item `{id}`                                                                                          |
| `DELETE api/ciphers/{id}`, `POST api/ciphers/{id}/delete` | deletes it; answer `{}`                                                                                                |
| `PUT api/ciphers/{id}/delete`                             | sets `deletedDate` (soft delete)                                                                                       |
| `PUT api/ciphers/{id}/restore`                            | sets `deletedDate` to null; 404 if missing                                                                             |
| `GET api/folders`                                         | `{"data": [folders], "object": "list", "continuationToken": null}`                                                     |
| `POST api/folders`                                        | makes a folder                                                                                                         |
| `PUT` or `POST api/folders/{id}`                          | makes or replaces folder `{id}`                                                                                        |
| `DELETE api/folders/{id}`, `POST api/folders/{id}/delete` | deletes it; answer `{}`                                                                                                |

- **Profile**: `object: "profile"`, `id`, `name`, `email` from the account,
  `emailVerified: true`, `premium: true`, `premiumFromOrganization: false`,
  `masterPasswordHint: null`, `culture: "en-US"`, `twoFactorEnabled: false`,
  `key`, `privateKey`, `securityStamp`, `forcePasswordReset: false`,
  `usesKeyConnector: false`, `avatarColor: null`, `creationDate`,
  `organizations: []`, `providers: []`, `providerOrganizations: []`,
  `_status: 0`.
- **Storing an item.** Take the member `cipher` of the body if there is one,
  else the body. Take out `id`, `Id`, `revisionDate`, `RevisionDate`,
  `lastKnownRevisionDate` and `collectionIds`. Set `created` (kept from the old
  item, else now) and `title: "Vault item"`.
- **Giving an item.** The stored object without `title`, plus `id`,
  `object: "cipherDetails"`, `revisionDate`, `creationDate`, `deletedDate`
  (default null), `organizationId: null`, `collectionIds: []`, `edit: true`,
  `viewPassword: true`, `organizationUseTotp: true`, and the defaults
  `attachments: null`, `favorite: false`, `reprompt: 0`.
- **A folder** is stored as `{"name", "title": "Vault folder"}` (no `name`:
  400 `name missing`) and given as `{"id", "name", "revisionDate", "object":
"folder"}`.

Other paths give 404. Organizations, collections, sends, attachments,
two-factor sign-in and changes of the master password are not supported.

## 34. AI assistants (MCP)

The node is a Model Context Protocol server at `POST /mcp`. It uses the
Streamable HTTP transport, but it always answers with plain JSON, never with an
event stream.

1. A public caller gets 401. A method other than POST gets 405 `only POST`.
2. A body that is not JSON gets 200 with
   `{"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}`.
3. The body is one JSON-RPC 2.0 message, or an array of them. Messages without
   an `id` are notifications and get no answer. If there is nothing to answer,
   the status is 202 with no body.
4. Methods:
   - `initialize`: answers with `protocolVersion` (the client's, or
     `2025-06-18`), `capabilities: {"tools": {}}`,
     `serverInfo: {"name": "homeserver", "version": "0.1.0"}`, and
     `instructions: "Personal data on the owner's homeserver. Resources are JSON, addressed as space + type/id."`
   - `ping`: `{}`.
   - `tools/list`: `{"tools": [...]}`.
   - `tools/call`: runs a tool.
   - Others: error −32601 `method not found`.
5. A tool gives `{"content": [{"type": "text", "text": "{pretty JSON}"}],
"structuredContent": {"result": …}}`. A tool error gives
   `{"content": [{"type": "text", "text": "{message}"}], "isError": true}`.

The caller's grant limits every tool. `limit` is an integer from 1 to 200
(default 20).

| Tool          | Description                                               | Input                                   | Result                  |
| ------------- | --------------------------------------------------------- | --------------------------------------- | ----------------------- |
| `list_spaces` | The spaces this token can read, with their names and ids. | –                                       | `[Space]`               |
| `search`      | Full-text search across readable spaces, best first.      | `query`, `limit`                        | `[Row]`                 |
| `timeline`    | Resources by the time they are about, newest first.       | `before` (ms), `limit`                  | `[Row]`                 |
| `near`        | Resources with a place near a point, nearest first.       | `lat`, `lon`, `km` (default 1), `limit` | `[Row]`                 |
| `get`         | Read one resource.                                        | `space`, `target`                       | Row, or `not found`     |
| `related`     | Everything linked to or from a resource.                  | `space`, `target`, `limit`              | `[{"rel", "resource"}]` |
| `list`        | Resources of one type in a space, newest first.           | `space`, `type`, `limit`                | `[Row]`                 |
| `put`         | Create or replace a JSON resource. Needs a writer token.  | `space`, `target`, `body` (an object)   | `{"seq"}`               |

A missing text input gives the error `{name} is required`. An unknown tool
gives `unknown tool {name}`.

## 35. Sign-in for people and other apps

### 35.1 The origin

The node's **origin** is `url` from the configuration (with a trailing `/`
taken off). If there is no `url`, it is
`{X-Forwarded-Proto, or https with TLS, or http}://{Host, or the URI's authority, or localhost}`.
The origin is the OIDC issuer and the WebAuthn origin. The **RP ID** is the
origin's host without the port.

### 35.2 OpenID Connect provider

The node is an OpenID Connect provider (OpenID Connect Core 1.0, with OAuth 2.0
of RFC 6749 and PKCE of RFC 7636) for apps of its owner.

**Key.** `keys/oidc` holds a PKCS#8 key. If it is missing, the node makes an
ECDSA P-256 key (ES256). It can also read an Ed25519 key (EdDSA). Without a
key, every `/oauth/` path gives 503 `no signing key`. The `kid` is the JWK
thumbprint (RFC 7638), SHA-256 and base64url.

**Discovery.** `/.well-known/openid-configuration`:

```json
{
  "issuer": "{origin}",
  "authorization_endpoint": "{origin}/oauth/authorize",
  "token_endpoint": "{origin}/oauth/token",
  "jwks_uri": "{origin}/oauth/jwks",
  "userinfo_endpoint": "{origin}/oauth/userinfo",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code"],
  "code_challenge_methods_supported": ["S256"],
  "id_token_signing_alg_values_supported": ["ES256", "EdDSA"],
  "scopes_supported": ["openid", "profile", "email"],
  "subject_types_supported": ["public"],
  "token_endpoint_auth_methods_supported": [
    "none",
    "client_secret_post",
    "client_secret_basic"
  ],
  "claims_supported": ["iss", "sub", "aud", "exp", "iat", "nonce", "name"]
}
```

**JWKS.** `GET /oauth/jwks` gives `{"keys": [jwk]}`, where the JWK has its key
members and `kid`, `alg` and `use: "sig"`.

**Clients.** Only the owner may use `GET /oauth/clients` (a list) and
`POST /oauth/clients` (others get 401).

- Request: `{"name"? (default "app"), "redirect_uris": [text], "confidential"?: bool}`.
  An empty list gives 400 `redirect_uris missing`.
- The client ID is base32 of 10 random bytes. A confidential client also gets
  a secret: base64url of 24 random bytes. The node keeps the hex of its
  SHA-256.
- The client is kept in the private space at `oauth-client/{id}` as
  `{"id", "name", "redirect_uris", "secret_hash", "title"}`.
- Answer: 201 `{"client_id", "client_secret" (or null), "issuer"}`.

**Authorization** (`GET /oauth/authorize`):

1. Read the query. A parameter given twice is an error
   (`invalid_request: repeated parameter {k}`). An empty value counts as
   missing.
2. `response_type` MUST be `code`. `code_challenge_method` MUST be `S256`
   (PKCE is required for every client). `scope` MUST contain `openid`.
   `client_id`, `redirect_uri` and `code_challenge` are required. `state` and
   `nonce` are optional.
3. The client MUST be known, and `redirect_uri` MUST equal one of its URIs
   exactly.
4. Any error so far gives a 200 HTML page with the title `Cannot sign in`. The
   node does not send errors back to the client.
5. If the caller is not the owner, a 200 HTML page with the title
   `Sign in first` says that the app asks for sign-in, and has a link back to
   the same request.
6. If the caller is the owner, the node makes a code and answers 302 to
   `{redirect_uri}?code={code}&state={state}` (with `&` in place of `?` if the
   URI has a query already, and without `state` if there was none). There is no
   consent page.

A **code** is base64url of 32 random bytes. It is kept in memory for 60 seconds
and can be used only once (it is taken out even when the use fails).

**Token** (`POST /oauth/token`, a form body):

1. `grant_type` MUST be `authorization_code` (else 400
   `{"error": "unsupported_grant_type", "error_description": "only authorization_code"}`).
2. The client is from `client_id` and `client_secret` in the form, or from an
   `Authorization: Basic` header, which wins. An unknown client gives 401
   `invalid_client` / `unknown client`. A confidential client MUST give the
   right secret (else 401 `invalid_client` / `bad secret`).
3. The code MUST be known and not out of date, and MUST be for this client and
   `redirect_uri`. `code_verifier` MUST be 43 to 128 characters of
   `A-Z a-z 0-9 - . _ ~`, and base64url(SHA-256(verifier)) MUST equal the
   challenge. Errors are 400 `{"error": "invalid_grant", "error_description"}`.
4. The answer (with `Cache-Control: no-store`) is
   `{"access_token", "id_token", "token_type": "Bearer", "expires_in": 3600, "scope"}`.

The **ID token** is a JWT (RFC 7519) signed with the key, with header
`{"alg", "kid", "typ": "JWT"}` and claims `iss` (the origin), `sub` (the
identity ID), `aud` (the client ID), `iat`, `exp` (iat + 3600), `nonce` (if
given) and `name` (the node's name).

The **access token** is a JWT (RFC 9068) with header `typ: "at+jwt"` and claims
`iss`, `sub`, `aud` (the origin), `client_id`, `scope`, `iat`, `exp`
(iat + 3600) and `jti` (base64url of 16 random bytes). It works only at the
user information endpoint. The REST API does not take it.

**User information** (`GET` or `POST /oauth/userinfo`): it needs
`Authorization: Bearer {access token}`. The token MUST have a good signature,
`typ: "at+jwt"`, `iss` and `aud` equal to the origin, and `exp` after now. Else
401 `{"error": …}`. The answer is `{"sub", "name"}`.

**JWT checks.** A JWT MUST have three parts. Its `alg` MUST be the key's. Its
`kid`, if there, MUST be the key's. A `crit` header is refused. The payload MUST
be a JSON object.

### 35.3 HTML pages

Sign-in pages are 200 with `Content-Type: text/html; charset=utf-8`:

```html
<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width"><title>{t}</title><link rel=stylesheet href=/app.css><main class=narrow><h1>{t}</h1>{body}</main>
```

### 35.4 Passkeys (WebAuthn)

The owner can sign in to a browser with a passkey (W3C Web Authentication
Level 2). All four endpoints are `POST`. Byte values in bodies are base64url
(standard base64 is accepted).

A **challenge** is base64url of 32 random bytes. It is kept in memory for
5 minutes, and can be used only once.

1. `POST /auth/passkey/register/start` (owner only, else 401) →
   `{"challenge", "rp": {"id": "{RP ID}", "name": "homeserver"}, "user": {"id": "{base64url of the identity key}", "name": "{node name}"}}`.
2. `POST /auth/passkey/register/finish` (owner only) with `clientDataJSON`,
   `authenticatorData`, `publicKey` (the SubjectPublicKeyInfo from
   `getPublicKey()`), `publicKeyAlgorithm` (−7 or −8) and `id` (the credential
   ID). The node checks it (below) and keeps it in the private space at
   `credential/{first 32 hex characters of the hash of the credential ID}` as
   `{"id", "alg", "public_key", "sign_count", "title": "Passkey"}`. 201
   `{"id": "credential/…"}`.
3. `POST /auth/passkey/login/start` (anyone) →
   `{"challenge", "rpId", "allowCredentials": [every credential ID]}`.
4. `POST /auth/passkey/login/finish` with `clientDataJSON`, `id`,
   `authenticatorData` and `signature`. An unknown credential gives 401
   `unknown passkey`. If the check passes, the node keeps the new sign count,
   makes a grant (role owner, all spaces, label `passkey session`,
   `session: true`, ending 30 days later), and answers 200
   `{"role": "owner", "owner": true, "grant": "{id}"}` with the cookie
   `hs_session` and `Max-Age=2592000`.

A missing member gives 400 `{name} missing`. A challenge that was not made, is
out of date, or was used gives 400 `unknown or expired challenge`. A failed
check gives 400 (register) or 401 (login) with its reason.

**Checks.**

- Algorithms: −7 (ES256, P-256 with SHA-256) and −8 (EdDSA, Ed25519). No
  attestation is checked.
- Client data: `type` MUST be `webauthn.create` or `webauthn.get`, the
  challenge MUST be the one made, `origin` MUST equal the origin, and
  `crossOrigin` MUST NOT be true.
- Authenticator data: at least 37 bytes; the first 32 MUST be the SHA-256 of
  the RP ID; the flag UP (0x01) MUST be set. User verification is not required.
  The sign count is bytes 33 to 36, big-endian.
- Registration: the flag AT (0x40) MUST be set, and the credential ID in the
  authenticator data (length at bytes 53–54) MUST equal `id`. The public key
  comes from the SubjectPublicKeyInfo: P-256 (OIDs 1.2.840.10045.2.1 and
  1.2.840.10045.3.1.7, a 65-byte point starting with 0x04) or Ed25519 (OID
  1.3.101.112, 32 bytes).
- Sign-in: the signature is over `authenticatorData ‖ SHA-256(clientDataJSON)`;
  an ES256 signature is in ASN.1 DER form. If the new count or the kept count
  is not zero, the new count MUST be greater than the kept count (else
  `sign count did not go up; the authenticator may be a copy`).

### 35.5 Forward auth

A reverse proxy can ask the node whether a browser is signed in, for other apps
on the domain `forward_domain`.

**Secret.** `keys/forward` holds 32 random bytes. If the file has another
length, the node uses a random secret until it stops.

**Cookie.** `hs_forward = base64url(sub) "." exp "." base64url(HMAC-SHA256(secret, base64url(sub) "." exp))`,
where `sub` is the identity ID and `exp` is the end time in seconds. It is good
when the HMAC is right and `exp` is after now.

**`/auth/verify`** (any method):

1. The original URL is `{X-Forwarded-Proto, or https}://{X-Forwarded-Host}{X-Forwarded-Uri, or /}`.
2. A good `hs_forward` cookie gives 200 with `Remote-User: {sub}` and
   `Remote-Name: {node name}`.
3. Else, with no `X-Forwarded-Host`, 401. With one, 302 to
   `{origin}/auth/login?rd={percent-encoded original URL}`.

**`/auth/login?rd={url}`** (any method):

1. Not the owner: a page `Sign in first`, with a link to sign in and come back.
2. No `forward_domain`: a page `Not set up`.
3. `rd` MUST start with `https://` or `http://`, and its host (without a port)
   MUST be `forward_domain` or end with `.{forward_domain}`. Else a page
   `Cannot continue`.
4. 302 to `rd`, with
   `Set-Cookie: hs_forward={value}; Domain={forward_domain}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=604800`
   (7 days).

## 36. Notifications (Web Push)

The node sends Web Push messages (RFC 8030) to browsers of the owner when shared
spaces change.

### 36.1 Subscriptions

`POST /api/push/subscribe` takes a browser PushSubscription. `endpoint` MUST be
there (else 400 `endpoint missing`), MUST start with `https://`, and
`keys.p256dh` and `keys.auth` MUST be there (else 400
`not a push subscription`). The node keeps the whole object, with
`"title": "Push subscription"`, in the private space at
`push/{first 32 hex characters of the hash of the endpoint}`. The same
endpoint takes the place of the old one.

### 36.2 VAPID

- `keys/vapid` holds an ECDSA P-256 key, PKCS#8. The node makes it when it is
  first needed.
- The subject is `push_subject` from the configuration, else `url`, else
  `mailto:owner@homeserver.invalid`.
- For each message the node makes a JWT (RFC 8292): header
  `{"typ":"JWT","alg":"ES256"}`; claims `{"aud": "{scheme}://{host[:port]} of the endpoint", "exp": now + 43200, "sub": subject}`;
  signature ES256 (64 bytes, r ‖ s).
- Header: `Authorization: vapid t={jwt}, k={base64url public key}`.

### 36.3 Sending

1. The node listens to events (section 26.4). A **worth telling** event is a
   `put` in a space that is not the private space, whose type is not `push`,
   `credential`, `oauth-client`, `grant` or `cipher`.
2. The first such event starts a 5-second wait. All such events in the wait are
   put together, by space.
3. The message is JSON: `title` = the names of the changed spaces, joined with
   `, `; `body` = `{target} changed` for one change, else `{n} changes`;
   `space` = the hex ID of the first space (in order of ID); `target` = the
   first target in it.
4. The node sends it to every subscription (up to 1000), one at a time. If the
   push service answers 404 or 410, the node deletes the subscription. Other
   answers and failures are not used. Nothing is sent again.

`POST /api/push/test` sends `{"title": "{node name}", "body": "Notifications work."}`
to every subscription.

**Encryption** (RFC 8291, with `aes128gcm` of RFC 8188):

1. `p256dh` (65 bytes, starting with 0x04) and `auth` (16 bytes) come from the
   subscription, in base64url. The message MUST be at most 3993 bytes, else it
   is not sent.
2. Make a 16-byte salt and a new P-256 key pair (`as_public`, 65 bytes).
   `ecdh` = ECDH(new private key, `p256dh`).
3. `ikm` = HKDF-SHA256(salt = `auth`, ikm = `ecdh`,
   info = `"WebPush: info" 0x00 ‖ p256dh ‖ as_public`, 32 bytes).
4. `cek` = HKDF-SHA256(salt, `ikm`, `"Content-Encoding: aes128gcm" 0x00`, 16).
   `nonce` = HKDF-SHA256(salt, `ikm`, `"Content-Encoding: nonce" 0x00`, 12).
5. Body = salt (16) ‖ record size 4096 (4 bytes, big-endian) ‖ 65 (1 byte) ‖
   `as_public` ‖ AES-128-GCM(`cek`, `nonce`, message ‖ 0x02). One record, no
   padding.
6. `POST {endpoint}` with `TTL: 86400`, `Content-Encoding: aes128gcm`,
   `Content-Type: application/octet-stream` and the VAPID header.

## 37. Media

### 37.1 Kind and size

The node knows a picture by its first bytes: `image/jpeg`, `image/png`,
`image/gif`, `image/webp`, `image/avif`, `image/heic` (HEIF with HEVC), and
`image/heif` (other HEIF). It reads the width and height as stored. For HEIF it
uses the largest size given.

### 37.2 EXIF

For every kind but GIF, the node reads EXIF (the main image only):

- **time**: `DateTimeOriginal`, else `DateTime`, read as `YYYY:MM:DD HH:MM:SS`.
  With `OffsetTimeOriginal` (`±HH:MM`), it is changed to UTC
  `YYYY-MM-DDTHH:MM:SSZ`. Without it, it is local time `YYYY-MM-DDTHH:MM:SS`.
  A date that is not real (such as `0000:00:00`) is not used.
- **place**: `GPSLatitude` and `GPSLongitude` (three rationals each:
  degrees + minutes/60 + seconds/3600), rounded to 7 decimal places, negative
  for `S` or `W`, and only within ±90 and ±180.
- **camera**: `Model` if it starts with `Make` (any case); else `{Make} {Model}`;
  or whichever one there is.
- **orientation**: 1 to 8.

The result is
`{"media_type", "width", "height", "time", "place": {"lat", "lon"}, "camera", "orientation"}`,
with only the members that are known (width and height only when both are
more than 0).

### 37.3 Files

When a JSON resource of type `file` has a member `hash`, and that blob is on the
node, the node adds to the JSON before it keeps it:

- `size`: the blob's length, if not given;
- `media`: the whole result of section 37.2, if the blob is a known picture;
- `media_type`, `time` and `place` from that result, each only if not given.

The command line uploads a file as a blob and then puts
`{"hash", "media_type", "name", "size"}` at `file/{first 16 characters of the hash}`.

### 37.4 Thumbnails

`GET /api/blobs/{hash}?thumb={n}` gives a picture whose longer side is at most
`n` (cut to 16–1024).

1. The node runs a separate process of its own program
   (`hs internal thumbnail-worker {n}`) with the blob on its standard input.
   At most 2 run at a time (more gives 503 `thumbnail worker unavailable`).
   It has 15 seconds and up to 8 MiB of output.
2. The worker reads only JPEG and PNG, of at most 64,000,000 pixels and at most
   65535 on a side. It makes the picture smaller (never larger): each new pixel
   is the rounded mean of its box of old pixels. Then it turns the picture by
   its EXIF orientation. A picture without alpha becomes JPEG at quality 80;
   with alpha it becomes PNG.
3. The worker writes one byte, `j` (JPEG) or `p` (PNG), then the picture. It
   ends with code 0 for success, 2 for "cannot make a thumbnail of this", 1 for
   other failures.
4. Code 2 gives 415 `no thumbnail for this`. Other failures give 502
   `thumbnail worker unavailable`. A node without a worker gives 503
   `thumbnail worker is not configured`.

## 38. The web app files

GET and HEAD requests that no other route takes are for the web app. The files
are part of the program when it is built. A build with no web app has only
`index.html`, with this content:

```html
<!doctype html>
<meta charset="utf-8" />
<title>homeserver</title>
<p>
  This homeserver has no web app yet. Use the <code>hs</code> command line:
  <code>hs login</code>, then <code>hs --help</code>.
</p>
```

1. The path (parts joined with `/`, or `index.html` if empty) names a file.
   If there is none and the path is one part without a `.`, it is `index.html`.
   Else 404.
2. If the file has a Brotli copy and `Accept-Encoding` lists `br` (not with
   `q=0`), the copy is sent with `Content-Encoding: br`.
3. `ETag` is `"` + the first 16 hex characters of the hash of the file (not
   compressed) + `"`, with `-br` before the last `"` for the Brotli copy.
   `If-None-Match` with an equal item gives 304.
4. Headers: `Vary: accept-encoding` (if there is a Brotli copy);
   `Cache-Control: public, max-age=31536000, immutable` if a part of the path
   (split on `/` and `.`) is 8 or more hex characters, else `no-cache`;
   `Content-Security-Policy: default-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`;
   `X-Frame-Options: DENY`.

# Part VIII. Between nodes

## 39. Finding and trusting another node

### 39.1 Node information

`/.well-known/homeserver` (any method, no sign-in) and `GET /api/node` give:

```json
{"name": "…", "identity": "{identity ID}", "did": "did:hs:{identity ID}", "node": "{node key ID}", "url": "https://…" | null, "version": "0.1.0"}
```

### 39.2 Addresses

A node knows another node's address only from:

1. the `home` given when it joins a space (section 42.3);
2. the `home` of a member record in a space's state;
3. the `url` given when an owner adds a member.

The identity log and the DID document list addresses too (sections 12 and 13),
but in this version the sync protocol does not read them. A URL is joined to a
path by taking off any trailing `/` and adding the path.

### 39.3 Trust

A node trusts another node's key because the key is a member of a space. When
an owner adds a member by URL, the node takes the key and identity from that
URL's node information without more proof. Between nodes, TLS certificates are
not checked (section 43). The HTTP signature (section 40) is what proves which
node is asking.

## 40. HTTP Message Signatures

Nodes sign their requests to each other with HTTP Message Signatures
(RFC 9421), using this profile.

### 40.1 Signing

1. If the body is not empty, add the header
   `Content-Digest: sha-256=:{base64 of SHA-256(body)}:` (RFC 9530).
2. The covered components are, in this order:
   `"@method" "@authority" "@path" "@query"`, and then `"content-digest"` if
   the body is not empty.
3. The parameters are, in this order: `created` (now, in seconds), `keyid`
   (the node key ID, as a string) and `alg` (`"ed25519"`).
4. Headers:

   ```
   Signature-Input: sig1=("@method" "@authority" "@path" "@query" "content-digest");created=1764079380;keyid="{node key ID}";alg="ed25519"
   Signature: sig1=:{base64 of the Ed25519 signature}:
   ```

5. The signature base is as in RFC 9421 section 2.5: one line
   `"{name}": {value}` for each component, then the line
   `"@signature-params": {the inner list with its parameters}`, with no newline
   at the end. Component values:
   - `@method`: the method as sent;
   - `@authority`: the request URI's authority, or else the `Host` header,
     trimmed, in lowercase;
   - `@path`: the path as sent (`/` if empty), not decoded;
   - `@query`: `?` and the query (`?` alone if there is none);
   - a header: all its values, each trimmed, joined with `, `.

### 40.2 What is sent with it

Requests between nodes have `Content-Type: application/cbor`, also a GET with
no body. This header is not covered. The client sends `Host` as `host`, or
`host:port` when the URL has a port. A proxy that changes `Host` makes the
signature fail.

### 40.3 Checking

1. `Signature-Input` and `Signature` MUST be there. Both are read as
   dictionaries (RFC 9651). Only the label `sig1` is used.
2. Each component MUST be a string with no parameters. The components
   `@method`, `@authority`, `@path` and `@query` MUST all be covered. Other
   components may be covered too: `@target-uri`, `@scheme`, `@request-target`
   and lowercase header names. Any other derived component is refused.
3. `created` MUST be there, and within 300 seconds of now in either direction.
   If `expires` is there, it MUST NOT be before now. If `alg` is there, it MUST
   be `ed25519`. `keyid` MUST be a key ID. Other parameters (such as `nonce`)
   are not used.
4. If there is a body, or a `Content-Digest` header, `content-digest` MUST be
   covered, the header MUST have a `sha-256` member, and it MUST equal the
   SHA-256 of the body.
5. The signature MUST be a good Ed25519 signature of the base under the key in
   `keyid`. The key is taken from `keyid` itself.
6. The caller is that key. What it may do comes from its member roles
   (section 28).

There is no memory of nonces. A copy of a request can be sent again within the
300-second window.

## 41. The federation endpoints

All paths start with `/fed/`. Only a node caller (section 28) may use them.
Others get 401 `sign in first`. A `{space}` here MUST be 64 hex characters (else
404), and the caller MUST be a member of it with any role (else 403
`not a member of this space`). Errors use section 30.4.

| Method and path                              | Request body                                   | Answer                                                                                                                               |
| -------------------------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `GET /fed/spaces/{space}/log?after={n}`      | –                                              | 200 `application/cbor`: an array of byte strings, the entries with seq > n (all if `after` is missing or not a number), at most 1000 |
| `GET /fed/spaces/{space}/head`               | –                                              | 200 `application/cose`: the signed head (section 19.1); 421 `not the primary` if this node is not the primary                        |
| `GET /fed/blobs/{hash}`                      | –                                              | 200 `application/octet-stream`: the blob, if an entry in a space where the caller is a member names it; else 404                     |
| `POST /fed/spaces/{space}/inbox`             | CBOR `[proposal, payload or null]`             | 200 `{"seq", "created"}`                                                                                                             |
| `POST /fed/spaces/{space}/inbox/conditional` | CBOR `[proposal, payload or null, conditions]` | 200 `{"seq", "created"}`; a failed "must exist" gives 404 with `hs-conditional-write: 1`                                             |
| `POST /fed/spaces/{space}/push`              | CBOR `[entry, payload or null]`                | 200 `{"seq"}`: this node's seq after the push                                                                                        |

- In the bodies, `proposal` and `entry` are byte strings. For `…/inbox` and
  `…/push`, a payload that is not a byte string counts as none, and items
  after the second are not used. For `…/inbox/conditional`, the array MUST
  have exactly three items, and the payload MUST be a byte string or `null`. A body of the wrong form gives 400
  (`expected [proposal, payload]`, `expected [proposal, payload, conditions]`,
  `expected [entry, payload]`).
- `conditions` is the four-item array of section 20.1.
- The inbox commits the proposal (section 26.1). The author of the proposal
  MUST be a member with the needed role. The sending node may be any member.
- `created` is true when the target had no current payload, false for a
  duplicate, and may be null.
- A node that is not the primary answers the inbox with 421.

## 42. The sync protocol

### 42.1 Roles of the nodes

- The **primary** of a space orders all its proposals and keeps the true log.
- Every other member node keeps a copy, and keeps the URL of the primary as
  `home` in its log directory.
- Each member record in the space may have a `home`: the URL of that member
  node. The primary sends new entries only to members that have one.

### 42.2 Making a space

The node that makes a space is its primary (section 31.2). If it has a `url`,
it adds a member entry for itself with that URL as home. Without it, other
members cannot find the primary again after a handover.

### 42.3 Adding a member and joining

1. An owner on node A adds node B (section 31.2). A writes a `member` entry
   for B, with B's URL as its home.
2. A sends the new entry to all members with a home, also to B (section 42.4).
   B is not in the space yet, so it refuses. That is not an error.
3. The owner of B calls `POST /api/spaces/join` with `{"home": A's URL,
"space": ID}`. B then:
   1. gets the whole log: `GET {home}/fed/spaces/{id}/log`, then again with
      `after` = the last seq, while a page has exactly 1000 entries (502 on any
      failure; 502 `the home returned no entries` if there are none);
   2. gets each payload named in the log that it does not have, from
      `GET {home}/fed/blobs/{hash}`, and keeps it only if the hash is right
      (failures are skipped);
   3. replays the whole log (section 18.5). Every entry is checked from the
      genesis on. If the space is already on B, B applies the new entries
      instead;
   4. keeps the log with `home` = A's URL (section 25.4);
   5. updates its home (section 42.5).

A can answer B's requests only after A has the member entry for B, because B
signs them with its node key.

### 42.4 Sending new entries (the pusher)

The node listens to its events (section 26.4). For each entry that **this node
ordered**:

1. The body is CBOR `[entry, payload or null]`.
2. The receivers are all members, after this entry is applied, except this
   node, that have a home.
3. For each receiver, separately: `POST {home}/fed/spaces/{id}/push`, signed.
   The answer is not used. Nothing is sent again, and there is no order
   between receivers.
4. If the entry is a handover, the node also updates its home (section 42.5).

When the pusher falls behind and loses events, the pulls (section 42.6) fill
the gap.

### 42.5 Getting a push, and the home

When a node gets a push of an entry with seq `s`, and its own seq is `h`:

1. If `s ≤ h + 1`: keep the payload as a blob (if there is one), and apply the
   entry if `s = h + 1` (section 26.1). Then update the home.
2. If `s > h + 1` (a gap), or the seq cannot be read: pull (section 42.6).
3. The answer is the node's seq.

**Updating the home.** If this node is not the primary, take the `home` of the
primary's member record. If there is one and it is not the `home` kept in the
log directory, keep the new one.

### 42.6 Pulling

To pull a space:

1. If this node is the primary, or has no home, there is nothing to do.
2. Get the entries after its own seq from the home, page by page (as in
   42.3).
3. For each entry in order, get its payload (from its blobs, or from the home)
   and apply it. The first failure stops the pull.
4. If any entry was applied, update the home.

A node pulls every space every **60 seconds** (the first time 60 seconds after
it starts), after a gap in a push, after it sends a write to the primary, and
when `POST /api/spaces/{space}/sync` asks.

### 42.7 Writing to a space

Every write through the API goes like this:

1. Make and sign the proposal (section 26.1) for the caller's access.
2. Try to commit it here. If this node is the primary, that is the end.
3. If this node is not the primary and knows the home, send the proposal:
   to `…/inbox` if there are no conditions, else to `…/inbox/conditional`.
4. Read the answer:
   - 412 → 412 `the resource has changed`;
   - 409 → 409 `idempotency key already used for a different request`;
   - 404 with `hs-conditional-write` → 404 `not found`;
   - another 2xx → go on;
   - anything else, or no answer → 502 (`{url}: {status} {the first 300
characters of the body}`).
5. Read `{"seq", "created"}` from the answer (else 502
   `commit receipt is not valid: …`).
6. Pull, so that this node has the new entry.
7. The entry at that seq here MUST be the same request (section 20.2) (else
   409). If the pull did not get it, 500 `committed sequence {seq} missing`.

If this node is not the primary and does not know the home, the answer is 421.

An older primary that does not have `…/inbox/conditional` answers 404 without
the `hs-conditional-write` header. The writer then gives 502. So conditions are
never dropped without a word.

### 42.8 Handover

1. An owner calls `POST /api/spaces/{space}/handover` with `{"node"}`. The
   current primary orders a `handover` entry. The new primary MUST already be a
   member (else 422 `the new primary must be a member`).
2. After it is applied, the new key is the primary. The old primary sends the
   entry to all members (also to the new primary), and updates its own home to
   the new primary's member home.
3. The other members get the entry by push or pull, and update their homes.
4. From then on, the new primary commits writes, and the old primary answers
   its inbox with 421.

If the new primary's member record has no home, the other members cannot find
it. There is no automatic choice of a new primary when a primary goes away.

### 42.9 What is guaranteed

- Every member applies the same entries in the same order: the order in which
  the primary got them. A member never applies an entry out of order.
- Conditions are checked by the primary at the moment it orders. Of two writes
  with the same `If-Match`, one gets 200 and one gets 412.
- A proposal with the same (author, nonce) goes in only once.

## 43. The outgoing HTTP client

The node uses one HTTP client for other nodes, replicas and push services:

- HTTP/1.1 only (ALPN `http/1.1`), one new TCP connection for each request.
- `http` and `https` URLs; default ports 80 and 443.
- TLS 1.3 or 1.2. For push services and S3 stores the certificate MUST chain to
  the Mozilla root list. For other nodes and for `hs://` replicas **any
  certificate is accepted** (the TLS handshake signatures are still checked).
- 30 seconds for the whole request. A response body may be at most 256 MiB.
- Redirects are not followed. There are no retries, no proxies and no
  compression.

## 44. Port mapping

With `--map-port`, `hs ctl serve` asks the home router to forward its TCP port.

1. **Gateway.** On Linux, from `/proc/net/route`: the first row with
   destination `00000000` and the flags UP and GATEWAY. On macOS and the BSDs,
   from `route -n get default` (the line `gateway:`). Else there is no mapping.
2. **Ask.** First PCP (RFC 6887), then, if PCP fails, NAT-PMP (RFC 6886),
   at the gateway's UDP port 5351. Each request is sent up to three times,
   waiting 250, 500 and 1000 ms. The node asks for the same external port as
   the internal one, and a lifetime of 7200 seconds.
3. **PCP MAP request** (60 bytes): version 2, opcode 1 (MAP), lifetime
   (4 bytes), the node's address as 16 bytes (an IPv4 address in the form
   `::ffff:a.b.c.d`), a 12-byte random nonce, protocol 6 (TCP), 3 zero bytes,
   internal port, suggested external port, suggested external address (zero).
   An answer is used only if its nonce, protocol and internal port match.
4. **NAT-PMP request** (12 bytes): `00 02 00 00`, internal port, suggested
   external port, lifetime (all big-endian). After a mapping, the node also
   asks for the external address (`00 00`).
5. **Keep it.** After a mapping, wait max(lifetime / 2, 60) seconds and ask
   again with the same nonce. A failure keeps the old mapping and tries again
   after the same wait. The mapping is not removed when the node stops.

# Part IX. Backups and moving

## 45. Replicas

A **replica** is an encrypted copy of the data directory in a **store**. The
configuration member `replicas` lists them, as addresses.

### 45.1 Addresses

| Address                                                             | Store                                          |
| ------------------------------------------------------------------- | ---------------------------------------------- |
| a path starting with `/` or `.`, or `dir:{path}`                    | a local directory                              |
| `s3://{key}:{secret}@{host[:port]}/{bucket}[/{prefix}][?{options}]` | an S3-compatible bucket                        |
| `hs://{key}:{secret}@{host[:port]}/{bucket}[/{prefix}][?{options}]` | the S3 endpoint of another node (section 49.2) |

- The key and the secret are percent-decoded. The user part ends at the last
  `@`.
- The prefix, if there is one, gets a `/` at its end.
- Options (form-encoded): `region={region}` (default `us-east-1` for `s3`,
  `homeserver` for `hs`), and `http=1` to use `http` in place of `https`.
- For `hs`, request paths start with `/s3`, and any TLS certificate is
  accepted.
- The path of an object is
  `{base}/{bucket}/{prefix}{name}`, with the bucket fully percent-encoded and
  the rest percent-encoded except `/` (section 49.1).
- When an address is shown, its secret is changed to `***`:
  `{scheme}://{key}:***@{rest}`.

### 45.2 Operations on a store

| Operation | Directory                         | S3                                         |
| --------- | --------------------------------- | ------------------------------------------ |
| get       | read the file                     | `GET`: 200 bytes, 404 none, other an error |
| put       | write durably                     | `PUT`: 2xx is success                      |
| delete    | remove the file (missing is fine) | `DELETE`: 2xx or 404 is success            |

Every S3 request is signed (section 49.1), with the header
`x-amz-content-sha256` = the hex SHA-256 of the body.

## 46. Encryption of replica objects

The **storage key** (section 10) is the key. For a logical name `n` (text):

- `derive(context, n)` = `keyed(storage key, context ‖ 0x00 ‖ n)`
  (section 6), 32 bytes.
- **Stored name**: `h` = hex of `derive("hs name v1", n)`; the name in the
  store is `h[0..2]/h[2..64]`.
- **Object key**: `derive("hs key v1", n)`, used for AES-256-GCM.
- **Sealed bytes**: `"HSE1"` (the 4 bytes `48 53 45 31`) ‖ a 12-byte random
  nonce ‖ the ciphertext ‖ the 16-byte tag. The additional data is the bytes of
  `n`. The total is the length of the plaintext + 32.
- **Opening** fails if the bytes are fewer than 32, do not start with `HSE1`,
  or do not pass AES-GCM (wrong key, wrong name, or a change).

So the store sees neither file names nor contents, and it cannot tell which
files are the same.

## 47. Replica sync

### 47.1 What is copied

Every regular file in the data directory, but not:

- `index.db` and anything whose path starts with it;
- `keys/storage`;
- `access.log`;
- `buckets` and everything in it;
- any path with a part that starts with `.tmp-` or `.space-`;
- any path that ends with `.tmp` or `.lock`.

Symbolic links are not followed. A file name that is not UTF-8, or that has
`\`, is an error.

### 47.2 The manifest

The logical name `manifest` holds JSON:

```json
{
  "version": 2,
  "time": 1764079380000,
  "files": {
    "{path}": { "hash": "{hex}", "size": 123, "generation": "{32 hex}" }
  }
}
```

- `files` is sorted by path. `hash` is the hash of the file. `generation` names
  the sync run that uploaded this version of the file.
- The logical name of a file's object is `snapshots/{generation}/{path}`. In a
  version 1 manifest there is no `generation`, and the logical name is the path.
- A reader accepts versions 1 and 2. A manifest that does not open gives
  `the manifest does not open with this key`.

### 47.3 One sync run

1. Make a **generation**: 16 random bytes in hex.
2. Read the manifest and its **revision**:
   - directory: the hash of the manifest file (or none);
   - S3: the `ETag` of the manifest (or none for 404), when the store does
     conditional writes. A plain S3 store always does. An `hs` store does when
     its answer has the header `hs-conditional-put`. Else the revision is
     "legacy".
3. List the files to copy (section 47.1), with each file's length, change
   time and (on Unix) inode number.
4. For each file, in order of path: read it (its length MUST still be the same,
   else stop with `source files changed during replica sync; run the sync again`)
   and hash it.
   - If the old manifest has the same hash, keep its old entry.
   - Else seal the file under `snapshots/{generation}/{path}` and put it.
     Note the old object (if any) as no longer needed.
5. List the files again. If anything changed, stop with the same error.
6. Seal the new manifest and **publish** it with a compare-and-swap against
   the revision of step 2:
   - directory: under a lock file `.tmp-cas.lock`, write only if the hash of
     the current file is still the revision;
   - S3 with an ETag: `PUT` with `If-Match: {etag}`; with none:
     `If-None-Match: *`; 412 or 409 means another sync won;
   - legacy: a plain `PUT`.
     If another sync won, stop with
     `another sync published a replica snapshot first; run the sync again`.
7. If the revision was not legacy, delete the objects no longer needed: old
   versions of changed files, and files that are gone. A failed delete is only
   a warning.

The result of each replica is
`{"replica": "{address, secret hidden}", "ok": bool, "sent"?, "bytes"?, "unchanged"?, "deleted"?, "error"?}`.
`sent` is the number of files uploaded, `bytes` their plain size, `unchanged`
the number kept, and `deleted` the number of objects deleted.

A failed run leaves the old manifest in place, and the old snapshot can still be
restored. Objects of a failed run stay in the store.

### 47.4 When it runs

A running node syncs every replica in turn **every hour** (the first time one
hour after it starts), and when the owner asks (`POST /api/replicas/sync`,
`hs ctl replica sync`).

## 48. Restore

`hs ctl restore --from {address}` makes a node again from a replica, with only
the recovery code.

1. The data directory MUST NOT hold a node, and MUST be missing or empty.
2. Read the recovery code (from `HS_RECOVERY_CODE`, or else from standard
   input), and derive the storage key.
3. Read the manifest (none: `no replica here (no manifest)`).
4. In a new directory `.restore-XXXXXX` next to the data directory, for each
   file of the manifest: the path MUST be plain (no `\`, no empty part, no `.`
   or `..`); get its object (missing: `{path} is missing from the replica`),
   open it (`{path} does not open`), and check its length and hash
   (`{path} does not match the manifest`). Write it.
5. If this fails, read the manifest again. If it changed (a sync ran at the
   same time), try one more time with the new manifest.
6. Write `keys/storage` (the hex of the storage key), and set the mode of the
   files in `keys/` to 0600.
7. Rename the new directory to the data directory.
8. Build the index (section 27.1).

## 49. S3 signing and the S3 endpoint of a node

### 49.1 Signature Version 4

S3 requests are signed with AWS Signature Version 4, service `s3`:

1. **Canonical request**:
   `{method}\n{path}\n{query}\n{headers}\n{signed headers}\n{payload hash}`.
   - The path is already percent-encoded: all but `A-Z a-z 0-9 - _ . ~` (and
     `/` in the object part), with `%XX` in uppercase.
   - The query has each name and value encoded the same way (with `/` too),
     sorted, as `name=value` joined with `&`.
   - The headers are always `host`, `x-amz-content-sha256` and `x-amz-date`,
     and any others to be signed, with lowercase names and trimmed values,
     sorted, each `name:value\n`. The signed headers are the names joined
     with `;`.
2. **String to sign**:
   `AWS4-HMAC-SHA256\n{YYYYMMDDTHHMMSSZ}\n{YYYYMMDD}/{region}/s3/aws4_request\n{hex SHA-256 of the canonical request}`.
3. **Key**: HMAC-SHA256 in a chain: `"AWS4" + secret` with the day, then the
   region, then `s3`, then `aws4_request`.
4. **Header**:
   `Authorization: AWS4-HMAC-SHA256 Credential={key}/{day}/{region}/s3/aws4_request, SignedHeaders={names}, Signature={hex}`.

### 49.2 The S3 endpoint of a node

A node keeps replicas of other nodes (or of any S3 tool) at `/s3/`. This is a
small part of the S3 API.

**Keys.** The configuration member `s3_keys` is
`[{"access_key", "secret_key", "bucket"}]`. A missing `secret_key` is `""`. A
missing `bucket` is the access key. Each key may use only its bucket. The
objects are kept in `buckets/{bucket}/`.

**Checking a request.** Errors are XML,
`<?xml version="1.0" encoding="UTF-8"?><Error><Code>{code}</Code><Message>{code}</Message></Error>`,
with `Content-Type: application/xml`.

1. `Authorization` and `x-amz-date` MUST be there, and the date MUST be read
   (else 403 `AccessDenied`). Only header signing is supported.
2. The date MUST be within 900 seconds of now (else 403
   `RequestTimeTooSkewed`).
3. The access key from `Credential` MUST be known (else 403
   `InvalidAccessKeyId`). The region is the one in `Credential`.
4. `x-amz-content-sha256` MUST be `UNSIGNED-PAYLOAD` or the SHA-256 of the body
   (else 400 `XAmzContentSHA256Mismatch`).
5. The signature MUST match (section 49.1) (else 403 `SignatureDoesNotMatch`).
6. The path is `/s3/{bucket}/{key}`. The bucket MUST be the key's bucket (else
   403 `AccessDenied`) and its name MUST be ASCII letters, digits, `-` and `.`,
   not starting with `.` (else 400 `InvalidBucketName`). An object key that is
   not a valid name (section 49.3) gives 400 `InvalidArgument`.

**Operations.**

| Request                            | Answer                                                                                             |
| ---------------------------------- | -------------------------------------------------------------------------------------------------- |
| `GET /s3/{bucket}` (any query)     | ListObjectsV2 (below)                                                                              |
| `HEAD` or `PUT /s3/{bucket}`       | 200                                                                                                |
| `PUT /s3/{bucket}/{key}`           | write it; 200 with `ETag: "{hex hash of the body}"`                                                |
| `GET` or `HEAD /s3/{bucket}/{key}` | 200 (or 206 for a range), `ETag: "{hex hash}"`; 404 `NoSuchKey`; both with `hs-conditional-put: 1` |
| `DELETE /s3/{bucket}/{key}`        | 204, also when it is missing                                                                       |
| other                              | 405 `MethodNotAllowed`                                                                             |

- **Conditional put.** Only `If-Match: "{64 hex}"` (with quotes) writes only if
  the current object has that hash. Only `If-None-Match: *` writes only if there
  is no object. A failed condition, or an `If-Match` of another form, gives 412
  `PreconditionFailed`. Both headers, or `If-None-Match` with another value,
  give 400 `InvalidArgument`. With neither, the write has no condition.
- **Range** works as in section 31.7, with `Content-Type:
application/octet-stream`. A range that is not good gives the whole object
  with 200.
- **Listing**: the query `prefix`, and `continuation-token` or `start-after`.
  The answer is the names that start with the prefix and come after the token,
  in order, at most 1000:

  ```xml
  <?xml version="1.0" encoding="UTF-8"?>
  <ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>{bucket}</Name><Prefix>{prefix}</Prefix><KeyCount>{n}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>{true|false}</IsTruncated><NextContinuationToken>{last key}</NextContinuationToken><Contents><Key>{key}</Key><Size>{size}</Size></Contents>…</ListBucketResult>
  ```

  `NextContinuationToken` is there only when the list is cut. `max-keys` and
  `delimiter` are not used.

Not supported: multipart upload, copy, presigned URLs, chunked payload
signing, versions and object metadata.

### 49.3 Object names in a store

A name is not empty, does not start with `/`, has no empty part, `.` or `..`
when split on `/`, and has only ASCII letters, digits, `-`, `_`, `.` and `/`.
A listing leaves out any name that contains `.tmp`.

## 50. Export

`GET /api/export` (owner) and `hs ctl export` give the data directory as a tar
file, for people to keep and read.

- `Content-Type: application/x-tar`,
  `Content-Disposition: attachment; filename="homeserver-YYYY-MM-DD.tar"`.
- It has every regular file except: `keys/` and everything in it; paths that
  start with `index.db`; paths with a part that starts with `.tmp-` or
  `.space-`; paths that end with `.tmp` or `.lock`.
- The files are in sorted order. Each one is at `homeserver/{path}`, with a GNU
  header, mode 0644, and its change time. Long names use GNU long-name entries.
- The files are read while they are sent. A file that gets shorter during the
  export stops it with an error.

An export has no keys, so it cannot be used to restore a node in this version.
It is for reading data with other tools.

# Part X. The command line

## 51. Program, options and where things are

### 51.1 Names

The program is `hs`. If it is started under a name whose file stem is
`hsctl`, it acts as `hs ctl`: the commands of `hs ctl` are its top-level
commands.

### 51.2 Global options

These can come before or after the command.

| Option         | Variable    | Meaning                         |
| -------------- | ----------- | ------------------------------- |
| `--dir DIR`    | `HS_DIR`    | the data directory              |
| `--server URL` | `HS_SERVER` | the node to talk to             |
| `--json`       | –           | print the API's answers as JSON |

Other environment variables: `HS_TOKEN` (a token that wins over the saved one),
`HS_RECOVERY_CODE` (for `restore`), `VISUAL` and `EDITOR` (for `edit`),
`COMPLETE` (for shell completion), `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and
`HOME`.

### 51.3 The data directory

`--dir`, or `$XDG_DATA_HOME/homeserver`, or `$HOME/.local/share/homeserver`, or
`./.local/share/homeserver` when `HOME` is not set.

### 51.4 Saved sign-ins

`hs login` saves sign-ins in `$XDG_CONFIG_HOME/homeserver/cli.json` (or
`$HOME/.config/homeserver/cli.json`). The file is pretty JSON, mode 0600,
written to `cli.json.tmp` and renamed:

```json
{
  "default": "http://127.0.0.1:8080",
  "servers": {
    "http://127.0.0.1:8080": { "insecure": false, "token": "hs_…" }
  }
}
```

**URLs** are made regular before use: white space and trailing `/` are taken
off; with no `://`, `http://` is put before addresses that start with
`localhost`, `127.` or `[::1]`, and `https://` before others; the scheme MUST
be `http` or `https`, and there MUST be no path.

`insecure: true` (from `hs login --insecure`) accepts any TLS certificate.
Else the certificate MUST chain to the Mozilla root list.

**Choosing the token.** The server is `--server`, or the saved default. The
token is `HS_TOKEN` if it is set and not empty, else the saved token for that
server, else an error.

### 51.5 Local or remote

Some commands can work on the data directory of a stopped node or through the
API of a running one:

1. With `--server`, use the API.
2. Else with `--dir`, open the directory.
3. Else, with the default directory: if it holds a node, or there is no saved
   sign-in, try to open it. If that fails and there is a saved sign-in, use the
   API of the saved default server.

A running node holds `node.lock`, so opening its directory fails, and the
command uses the API. Some commands need the API (they need a running node).
Some need the directory (they need the node to be stopped).

### 51.6 How the client talks

The command line uses the REST API with `Authorization: Bearer {token}` and
`User-Agent: homeserver/0.1.0`. It reads bodies of at most 64 MiB whole
(streams have no limit). It does not try again. Path parts are percent-encoded
(all but `A-Z a-z 0-9 - _ . ~`). Error messages are:

- 401: `{url} did not accept the token ({message}); sign in again with hs login {url}`;
- 412: `it changed on the server after you read it ({message})`;
- others: `{message} ({status})`, where the message is the `error` of the
  body, or the status's reason.
- A failed connection: `cannot reach {url}: {error}` and a hint to start the
  node or use `--dir`.

## 52. Commands

### 52.1 `hs`

| Command                                                                    | Where        | What it does                                                                                                                                                                            |
| -------------------------------------------------------------------------- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `login [URL] [--insecure]`                                                 | API          | reads a token (`HS_TOKEN`, or asks for it without echo), checks it with `GET /api/session` and `GET /api/node`, saves it as the default; prints `Signed in to {name} ({url}) as {role}` |
| `logout [URL]`                                                             | file         | takes the sign-in out of `cli.json` (the token is not revoked)                                                                                                                          |
| `status`                                                                   | API          | node, identity, version, role, index health, counts                                                                                                                                     |
| `ls SPACE [TYPE] [--limit N] [--offset N] [--all]`                         | API          | lists resources (pages of `page=1`; `--all` gets every page)                                                                                                                            |
| `get SPACE TYPE/ID [-o FILE]`                                              | API          | prints or saves a resource                                                                                                                                                              |
| `put SPACE TYPE/ID [FILE] [--media-type T] [--new] [--if-match ETAG]`      | API          | PUT from a file or standard input; `--new` sends `If-None-Match: *`                                                                                                                     |
| `edit SPACE TYPE/ID`                                                       | API          | gets it, opens an editor, PUTs it back with `If-Match` (or `If-None-Match: *` if it is new)                                                                                             |
| `patch SPACE TYPE/ID PATCH`                                                | API          | JSON Merge Patch (`-` reads standard input)                                                                                                                                             |
| `rm SPACE TYPE/ID [--erase]`                                               | API          | DELETE, or erase                                                                                                                                                                        |
| `upload SPACE FILE…`                                                       | API          | each file: POST it as a blob, then PUT `file/{first 16 of the hash}` (section 37.3)                                                                                                     |
| `search WORDS… [--limit N]`                                                | API          | `GET /api/search`                                                                                                                                                                       |
| `timeline [--before YYYY-MM-DD] [--limit N]`                               | API          | `GET /api/timeline`                                                                                                                                                                     |
| `near LAT LON [--km K] [--limit N]`                                        | API          | `GET /api/near`                                                                                                                                                                         |
| `related SPACE TYPE/ID`                                                    | API          | `GET /api/related/…`                                                                                                                                                                    |
| `blob put FILE [--media-type T]` / `blob get HASH [-o FILE]`               | API          | upload or download a blob                                                                                                                                                               |
| `watch`                                                                    | API          | prints events from `GET /api/events` until stopped                                                                                                                                      |
| `space list` / `space create NAME`                                         | local or API | list or make spaces                                                                                                                                                                     |
| `space show SPACE`                                                         | API          | the space and its members                                                                                                                                                               |
| `space join --home URL SPACE`                                              | API          | `POST /api/spaces/join`                                                                                                                                                                 |
| `space member add SPACE (--url URL \| --node ID --identity ID) [--role R]` | API          | add a member (role default `writer`)                                                                                                                                                    |
| `space member remove SPACE NODE`                                           | API          | remove a member                                                                                                                                                                         |
| `space handover SPACE NODE`                                                | API          | make another member the primary                                                                                                                                                         |
| `space sync SPACE`                                                         | API          | pull now                                                                                                                                                                                |
| `identity`                                                                 | local or API | prints the DID document                                                                                                                                                                 |
| `openapi`                                                                  | –            | prints the OpenAPI document                                                                                                                                                             |
| `completion SHELL`                                                         | –            | prints a completion script for `hs` and `hsctl` (bash, zsh, fish, elvish, powershell)                                                                                                   |
| `ctl …`                                                                    |              | section 52.2                                                                                                                                                                            |

Defaults: `--limit` 50, `--km` 1.

Media types for `put` and `upload` come from the file's extension: `json`,
`vcf`/`vcard` (`text/vcard`), `ics` (`text/calendar`), `txt`, `md`,
`html`/`htm` (each `; charset=utf-8`), `jpg`/`jpeg`, `png`, `gif`, `webp`,
`heic`, `svg` (`image/svg+xml`), `pdf`, `mp4`, `mov` (`video/quicktime`), `mp3`
(`audio/mpeg`), `m4a` (`audio/mp4`). For `put`, bytes that are JSON are
`application/json`. For `upload`, other files are `application/octet-stream`.

### 52.2 `hs ctl` (and `hsctl`)

| Command                                                                         | Where        | What it does                                                                                                                                                              |
| ------------------------------------------------------------------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `init --name NAME [--url URL]`                                                  | directory    | makes a node (section 23.1); prints the identity, the recovery code and the owner token; if there are no saved sign-ins, saves one for `URL` (or `http://127.0.0.1:8080`) |
| `serve [--listen ADDR] [--tls off\|self-signed] [--domain D]… [--map-port]`     | directory    | runs the node (section 30.1); `--listen` default `127.0.0.1:8080`                                                                                                         |
| `config get [KEY]` / `config set KEY VALUE`                                     | directory    | reads or sets a top-level member of `config.json`; VALUE is JSON, or else a string                                                                                        |
| `token create [--role R] [--label L] [--space S]… [--type T]… [--expires-in D]` | local or API | makes a grant; prints the token (role default `writer`)                                                                                                                   |
| `token list` / `token revoke ID`                                                | local or API | lists or revokes grants                                                                                                                                                   |
| `export`                                                                        | local or API | writes the tar to standard output (not to a terminal)                                                                                                                     |
| `replica list` / `replica sync`                                                 | local or API | lists replicas, or syncs them now                                                                                                                                         |
| `restore --from REPLICA`                                                        | directory    | section 48                                                                                                                                                                |
| `recovery split --threshold K --count N`                                        | –            | reads a recovery code and prints N shares (section 11.2)                                                                                                                  |
| `recovery combine`                                                              | –            | reads shares, one for each line, and prints the recovery code                                                                                                             |
| `rebuild-index`                                                                 | directory    | deletes the index and builds it again                                                                                                                                     |

`--expires-in` is a number and a unit: `s`, `m` or `min`, `h`, `d`, `w`. The end
time is computed with the client's clock.

### 52.3 Output

- Data goes to standard output. Notes such as `revoked {id}` and `no change` go
  to standard error.
- Tables have a header row, columns two spaces apart, wide (CJK) characters
  counted as two, and no white space at line ends. Times are UTC
  `YYYY-MM-DD HH:MM`. Sizes use the units B, kB, MB, GB and TB (powers of
  1000).
- `--json` prints the API's answer as pretty JSON.
- An error prints `hs: {message}` (or `hsctl: …`) to standard error.
- **Exit codes**: 0 for success; 1 for a failed command; 2 for a wrong command
  line.

### 52.4 Completion

The completion script runs the program again with `COMPLETE={shell}`. The
program then gives candidates and stops. Space names, grant IDs and
configuration keys come from the node (through the data directory or the API),
with at most 2 seconds for each step. On any failure there are no candidates.

# Appendices

## Appendix A. Constants

| Name                                  | Value                                              | Section    |
| ------------------------------------- | -------------------------------------------------- | ---------- |
| COSE alg                              | −8 (EdDSA)                                         | 8.2        |
| COSE protected header                 | `a1 01 27`                                         | 8.2        |
| CBOR nesting limit                    | 32                                                 | 7.3        |
| storage key context                   | `homeserver 2025 storage key v1`                   | 6          |
| token hash context                    | `homeserver 2025 token v1`                         | 6          |
| replica name context                  | `hs name v1`                                       | 46         |
| replica key context                   | `hs key v1`                                        | 46         |
| replica magic                         | `HSE1`                                             | 46         |
| target part length                    | 1–128 bytes                                        | 15.2       |
| proposal nonce (made by the node)     | 16 random bytes                                    | 15         |
| token                                 | `hs_` + base32 of 20 bytes                         | 29.2       |
| grant ID, OAuth client ID             | base32 of 10 bytes                                 | 29.1, 35.2 |
| entries in a segment                  | 256                                                | 19.2       |
| event channel                         | 1024 events                                        | 26.4       |
| index schema version                  | 1                                                  | 27.1       |
| search text limit                     | 64 KiB                                             | 27.3       |
| title lengths                         | 200 / 80 characters                                | 27.3       |
| default listen address                | `127.0.0.1:8080`                                   | 30.1       |
| request body limit                    | 256 MiB                                            | 30.1       |
| response body limit (outgoing client) | 256 MiB                                            | 43         |
| CLI body limit                        | 64 MiB                                             | 51.6       |
| TLS handshake limit                   | 10 s                                               | 30.1       |
| outgoing request limit                | 30 s                                               | 43         |
| access log queue / batch              | 1024 / 64                                          | 30.6       |
| list limit                            | default 100 (resources), 50 (others); at most 1000 | 31.5, 31.6 |
| blob piece                            | 64 KiB                                             | 31.7       |
| event stream retry / keep-alive       | 3000 ms / 25 s                                     | 31.9       |
| signature label                       | `sig1`                                             | 40         |
| signature window                      | 300 s                                              | 40.3       |
| log page                              | 1000 entries                                       | 41         |
| pull interval                         | 60 s                                               | 42.6       |
| replica sync interval                 | 3600 s                                             | 47.4       |
| S3 clock window                       | 900 s                                              | 49.2       |
| S3 list page                          | 1000                                               | 49.2       |
| session grant life                    | 30 days                                            | 33.2, 35.4 |
| forward-auth cookie life              | 604800 s (7 days)                                  | 35.5       |
| OAuth code life                       | 60 s                                               | 35.2       |
| OIDC token life                       | 3600 s                                             | 35.2       |
| passkey challenge life                | 300 s                                              | 35.4       |
| PBKDF2 rounds (vault)                 | 100000                                             | 33.1       |
| default vault KDF rounds              | 600000                                             | 33.1       |
| VAPID JWT life                        | 43200 s                                            | 36.2       |
| push TTL                              | 86400 s                                            | 36.3       |
| push gather wait                      | 5 s                                                | 36.3       |
| push message limit                    | 3993 bytes                                         | 36.3       |
| thumbnail side                        | 16–1024                                            | 37.4       |
| thumbnail workers / time / output     | 2 / 15 s / 8 MiB                                   | 37.4       |
| thumbnail pixel limit                 | 64,000,000                                         | 37.4       |
| JPEG quality                          | 80                                                 | 37.4       |
| PCP/NAT-PMP port                      | 5351                                               | 44         |
| port mapping lifetime                 | 7200 s                                             | 44         |
| MCP protocol version                  | `2025-06-18`                                       | 34         |
| MCP tool limit                        | default 20, 1–200                                  | 34         |
| completion wait                       | 2 s                                                | 52.4       |

## Appendix B. Not in this version

These are planned, but homeserver 0.1.0 does not have them:

- end-to-end encryption of shared spaces with MLS (the `mls` kind is there, and
  does nothing);
- QUIC and HTTP/3, the path layer, relays and mailboxes;
- finding a node's address from its identity log or DID;
- WebAssembly plugins (there is no plugin interface yet);
- making and using checkpoints, and removing old log segments;
- a web app (only a page that points to the command line);
- signed self-updates;
- restoring from an export.

## Appendix C. Known limits of this version

These are how 0.1.0 works today. A compatible program may do better, but then
it is not the same in these points.

1. **Blobs.** Any signed-in caller can read any blob by its hash, whatever its
   grant's space list (section 31.7).
2. **Signatures.** There is no memory of nonces, so a signed request can be
   sent again within 300 seconds (section 40.3).
3. **Sync.** Pushes are not sent again when they fail. Only the 60-second
   pull fills gaps (section 42.4). `GET /fed/spaces/{space}/head` is not used
   by any node. When the primary goes away, there is no automatic way to
   choose another.
4. **Trust of a new member.** Adding a member by URL trusts the node
   information at that URL without proof, and TLS certificates between nodes
   are not checked (section 39.3).
5. **Duplicates.** Only the primary refuses a second proposal with the same
   (author, nonce). Checkpoints, when they are used, will not carry the nonces
   (section 19.3).
6. **Erase.** In the index, delete and erase remove the row. In the space
   state, an erased resource keeps its media type, seq and time (section
   18.3).
7. **Conditions.** A 412 from a primary always comes back as
   `the resource has changed` through a member, also when it was
   "already exists" (section 42.7). No 428 is ever given.
8. **Lists.** Without `page=1`, and in search, near and related, type limits
   are applied after the limit, so pages can be short (section 31.5).
9. **Content negotiation.** All representations of a resource have the same
   ETag, and there is no `Vary: Accept` (section 31.3). Any JSON resource can be
   given as vCard or iCalendar.
10. **Events.** `Last-Event-ID` is not used. The caller's access is fixed when
    the stream starts (section 31.9).
11. **DAV.** `PROPFIND` and `REPORT` on a collection or item do not check the
    grant's type list. `supported-address-data` says vCard 3.0, but the node
    gives vCard 4.0. `sync-token` is given, but `sync-collection` is not
    supported. The privilege set always says "write". Time zones (`TZID`) are
    lost.
12. **Vault.** `expires_in` is 3600, but the grant works for 30 days.
    `revision-date` needs no sign-in. Any third path part after
    `api/ciphers/` is taken as an item ID.
13. **OIDC.** Discovery lists `EdDSA` and the scope `email`, but a new node
    makes only an ES256 key and gives no e-mail claim. There is no consent
    page. Codes are lost when the node stops. Without `url`, the issuer comes
    from the `Host` header.
14. **Passkeys.** `login/start` gives every credential ID to anyone. The key in
    the authenticator data is not compared with the given public key.
15. **Forward auth.** The default `rd=/` is always refused.
16. **Web Push.** Messages over 3993 bytes are not sent. Deletes never cause a
    message.
17. **Node.** The TLS private key `keys/self-signed.key` is not made with
    mode 0600. `init` is not atomic: if it stops half way, it cannot be run
    again on the same directory. A grant's `types` does not stop it from being
    the owner. Port mappings are not removed on stop. Only SIGINT stops the
    server.
18. **S3 endpoint.** A bad range gives the whole object (never 416). `HEAD`
    with a range gives the full length. The ETag is the BLAKE3 hash, not MD5.
19. **Command line.** `init` saves a sign-in for `http://127.0.0.1:8080` (or
    `--url`), whatever address `serve` uses later. `--insecure` trusts any
    certificate, with no pinning.

## Appendix D. Normative references

Some interfaces of the node copy interfaces that have no formal standard: the
S3 API with its Signature Version 4, the Bitwarden client API, and the SQLite
database of the index. This document gives every part of them that the node
uses, so it does not name other documents for them.

- **BCP 14**: RFC 2119, _Key words for use in RFCs to Indicate Requirement
  Levels_; RFC 8174, _Ambiguity of Uppercase vs Lowercase in RFC 2119 Key
  Words_.
- **RFC 3986**: _Uniform Resource Identifier (URI): Generic Syntax_.
- **RFC 4648**: _The Base16, Base32, and Base64 Data Encodings_.
- **RFC 4918**: _HTTP Extensions for Web Distributed Authoring and Versioning
  (WebDAV)_.
- **RFC 4791**: _Calendaring Extensions to WebDAV (CalDAV)_.
- **RFC 6352**: _CardDAV: vCard Extensions to WebDAV_.
- **RFC 5545**: _Internet Calendaring and Scheduling Core Object Specification
  (iCalendar)_.
- **RFC 6350**: _vCard Format Specification_.
- **RFC 5869**: _HMAC-based Extract-and-Expand Key Derivation Function (HKDF)_.
- **RFC 2104**: _HMAC_. **RFC 8018**: _PKCS #5 v2.1_ (PBKDF2).
- **FIPS 180-4**: _Secure Hash Standard_ (SHA-256).
- **FIPS 186-5** and **SEC 2**: ECDSA and the curve P-256.
- **NIST SP 800-38D**: _Galois/Counter Mode (GCM)_. **FIPS 197**: AES.
- **RFC 5208** / **RFC 5958**: PKCS #8. **RFC 5280**: SubjectPublicKeyInfo.
- **RFC 6749**: _The OAuth 2.0 Authorization Framework_.
- **RFC 6886**: _NAT Port Mapping Protocol (NAT-PMP)_.
- **RFC 6887**: _Port Control Protocol (PCP)_.
- **RFC 7396**: _JSON Merge Patch_.
- **RFC 7515**, **RFC 7517**, **RFC 7518**, **RFC 7519**: JWS, JWK, JWA, JWT.
  **RFC 8037**: EdDSA in JOSE. **RFC 7638**: _JWK Thumbprint_.
- **RFC 7636**: _Proof Key for Code Exchange (PKCE)_.
- **RFC 8030**: _Generic Event Delivery Using HTTP Push_.
- **RFC 8188**: _Encrypted Content-Encoding for HTTP_.
- **RFC 8291**: _Message Encryption for Web Push_.
- **RFC 8292**: _VAPID for Web Push_.
- **RFC 8032**: _Edwards-Curve Digital Signature Algorithm (EdDSA)_.
- **RFC 8259**: _The JavaScript Object Notation (JSON) Data Interchange
  Format_.
- **RFC 8446**: TLS 1.3. **RFC 5246**: TLS 1.2. **RFC 7301**: ALPN.
- **RFC 8949**: _Concise Binary Object Representation (CBOR)_.
- **RFC 9052**: _CBOR Object Signing and Encryption (COSE): Structures and
  Process_.
- **RFC 9068**: _JWT Profile for OAuth 2.0 Access Tokens_.
- **RFC 9110**: _HTTP Semantics_. **RFC 9112**: HTTP/1.1. **RFC 9113**: HTTP/2.
  **RFC 6265**: _HTTP State Management Mechanism_ (cookies).
- **RFC 9421**: _HTTP Message Signatures_.
- **RFC 9530**: _Digest Fields_.
- **RFC 9651**: _Structured Field Values for HTTP_.
- **BLAKE3**: _The BLAKE3 cryptographic hash function_, specification
  version 1 (J. O'Connor, J.-P. Aumasson, S. Neves, Z. Wilcox-O'Hearn, 2020).
- **Ed25519**: RFC 8032. **X.690**: ASN.1 DER.
- **W3C Decentralized Identifiers (DIDs) v1.0** and **W3C Controlled
  Identifiers v1.0** (Multikey).
- **W3C Web Authentication Level 2**.
- **W3C Push API**.
- **HTML Living Standard** (WHATWG), section on server-sent events.
- **OpenID Connect Core 1.0** and **OpenID Connect Discovery 1.0**.
- **OpenAPI Specification 3.1.0**.
- **Model Context Protocol**, specification version 2025-06-18.
- **JSON-RPC 2.0**.
- **POSIX tar** (IEEE Std 1003.1) with GNU extensions.
- **EXIF 2.3** (CIPA DC-008).
