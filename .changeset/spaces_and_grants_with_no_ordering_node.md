---
default: major
---

# Spaces and grants, with no ordering node

The kernel is new. Folders are now spaces: every writer has its own signed log, and the logs make one causal graph, so every node can write at any time, also offline, and all nodes come to the same state. Files have fixed IDs; changes made at the same time are kept side by side. Content is cut into chunks, so a new version sends only what changed. Checkpoints let nodes drop old entries, and old versions get thinner with age.

Access is now grants in the kernel: exact permissions or templates (`read`, `read-write`, `manage`, `relay`), on a path or on one file, which can be taken back with everything given on from them. The engine and `Capability.app` are gone.

The command line changes: `fos node init` needs no engine, and `fos space new|mount|grant|grants|take|checkpoint` takes the place of `fos folder`. The data directory has a new layout, so make a new node. Apps keep the same interface; the chat app now keeps one file for each message.
