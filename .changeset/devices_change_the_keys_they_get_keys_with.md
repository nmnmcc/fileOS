---
default: minor
---

# Devices change the keys they get keys with

Each node now has a recipient key that it uses only to get keys, and changes it every 30 days, keeping only the one before. Keys are sealed to it, never to the node key. Each epoch also lasts at most 30 days. So if a device is stolen later, keys sealed to its old recipient keys, and keys of deleted data, cannot be opened, even from copies. The secrets are kept in `recipients.key` in the data directory.
