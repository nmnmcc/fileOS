---
default: minor
---

# One identity for many nodes

A person now has an identity of their own, apart from the keys of their nodes. The identity is a DID of the method `did:fos`: the `did:plc` method of AT Protocol (a signed log, rotation keys with a priority, a signing key), with ed25519 keys and no directory server. `fos identity new` makes one, and the node that runs it holds its keys. `fos identity add-node` and `fos identity join` add another node, and `fos identity remove-node` takes one away. Each node has a certificate signed by the identity, in an identity folder at `/identity`. `fos identity` shows the DID, its keys, its nodes and the link to give to other people.
