---
default: minor
---

# One identity for many nodes

A person now has an identity of their own, apart from the keys of their nodes. The identity is a DID of the method `did:fos`: the `did:plc` method of AT Protocol (a signed log, rotation keys with a priority, a signing key), with ed25519 keys and no directory server. `fos id new` makes one, and the node that runs it holds its keys. `fos id add-node` and `fos id join` add another node, and `fos id remove-node` takes one away. Each node has a certificate signed by the identity, in an identity folder at `/identity`. `fos id` shows the DID, its keys, its nodes and the link to give to other people.
