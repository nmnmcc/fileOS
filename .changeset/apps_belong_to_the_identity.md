---
default: minor
---

# Apps belong to the identity

An identity now has a home space that it owns, and every node of the person uses it as its home: mounts, app views and apps are the same on every node. An app installed on one node runs on every node of the person, with the same ID and key, and its files and private data reach them all. A node that joins an identity brings the mounts it had. App keys now come from a seed in the home space, so apps installed before this change must be installed again.
