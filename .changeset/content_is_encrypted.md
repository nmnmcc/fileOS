---
default: minor
---

# Content is encrypted

The content of files is now encrypted, with a key for each file. The key of a file is wrapped with the key of the prefix it is in, and a read grant on a prefix gives the keys of that prefix only. Keys go to each device with HPKE (RFC 9180). Taking back access starts a new epoch, so a reader who was taken away cannot read what comes after. When a deleted file leaves the bin, its key is dropped at the next checkpoint, and copies of its bytes on other nodes can no longer be opened. A node that only keeps a space (a backup or a relay) sees only ciphertext. Names, sizes and authors are not encrypted. Spaces made before this change cannot be read; make a new node.
