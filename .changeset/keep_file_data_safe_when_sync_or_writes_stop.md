---
default: patch
---

# Keep file data safe when sync or writes stop

A node can write again after a log write stops halfway. It keeps the
snapshot and the rest of the log together when it drops old entries.

A node starts from a snapshot signed by the space owner. Other
checkpoints keep the log that shows who had the right to sign them.

A node asks again for pieces lost during sync. File reads check each
piece, and `fos fs cat` reads large files a piece at a time.
