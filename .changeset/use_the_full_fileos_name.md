---
default: major
---

# Use the full fileOS name

Use `fileos` in shell commands, `FILEOS_DIR` for the data directory, and `fileos.localhost` for the kernel API. Update scripts that call the old command. The default data directory is still `~/.fileos`.

New apps use `fileos:app@0.1.0`. The node can still run apps built for the old interface. The command line shows identities as `did:fileos`. Old identity links, grants and signed records still work.

The documentation site is at <https://docs.fileos.app/>.
