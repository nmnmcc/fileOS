---
default: minor
---

# A grant to a person is for all their nodes

`fos space grant <path> did:fos:… <permissions>` gives access to a person, so to every node of their identity, also to nodes they add later. Each node shows its node certificate when it connects, and a node that has the space writes a vouch for it into the space; every node can check a vouch with nothing else. A new signing key for the identity ends the certificates of the old one, so a lost device loses access in every space.
