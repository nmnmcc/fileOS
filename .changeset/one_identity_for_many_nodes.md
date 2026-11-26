---
default: minor
---

# One identity for many nodes

A person now has an identity of their own, apart from the keys of their nodes. `fos id new` makes one, and the node that runs it holds its key. `fos id add-node` and `fos id join` add another node, and `fos id remove-node` takes one away. Each node has a certificate signed by the identity, in an identity folder at `/identity`. `fos id` shows the identity, its nodes and the link to give to other people.
