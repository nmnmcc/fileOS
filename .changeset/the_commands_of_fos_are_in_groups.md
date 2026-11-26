---
default: minor
---

# The commands of fos are in groups

The commands of `fos` are now in groups, and they are about the kernel only: `fos node` (`init`, `run`, `status`, `connect`), `fos fs` (`ls`, `cat`, `put`, `append`, `rm`), `fos folder` (`new`, `mount`, `grant`), `fos identity` or `fos id`, and `fos app install`. `fos kv` and `fos policy` are gone, because they were about what an app or the engine does. `fos node status` no longer lists apps and their messages; the app folders are in `/apps`, as any folder.
