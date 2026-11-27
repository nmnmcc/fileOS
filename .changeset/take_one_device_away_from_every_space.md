---
default: minor
---

# Take one device away from every space

`fileos identity remove-node` now also signs a revocation of the node. Every node of the identity writes it into the spaces it has, so the removed node loses, in every space, what it had as a node of the person. Its old certificate never counts again; to add it back, the identity needs a new signing key.
