---
default: minor
---

# The commands of `fileos` are in groups

The commands of `fileos` are now in groups, and they are about the kernel only: `fileos node` (`init`, `run`, `status`, `connect`), `fileos fs` (`ls`, `cat`, `put`, `append`, `rm`), `fileos folder` (`new`, `mount`, `grant`), `fileos identity` or `fileos id`, and `fileos app install`. `fileos kv` and `fileos policy` are gone, because they were about what an app or the engine does. `fileos node status` no longer lists apps and their messages; the app folders are in `/apps`, as any folder.
