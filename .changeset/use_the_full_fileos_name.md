---
default: major
---

# Use the full fileOS name

Use `fileos` in shell commands, `FILEOS_DIR` for the data directory, and `fileos.localhost` for the kernel API. Update scripts that call the old command.

Build apps for `fileos:app@0.1.0`. This change breaks old app bundles and identity records. Make new identities with `fileos identity new` on new nodes.

The documentation site is at <https://docs.fileos.app/>.
