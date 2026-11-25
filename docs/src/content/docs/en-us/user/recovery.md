---
title: Your recovery code
description: Keep the recovery code safe, and split it among friends if you want.
---

When a homeserver is made, it prints a **recovery code** one time. It is the
key to your identity, and it decrypts your backups. It is not stored
anywhere: not on the node, and not in any other place.

With only the recovery code, you can bring a new node back from a backup.
Without it, nobody can read an encrypted backup.

## Keep it safe

- Write it on paper, and keep it with your important papers.
- Do not keep it on the homeserver. Do not keep it only in a password
  manager that is itself on the homeserver.
- Anyone who has it can get your data back. Take care of it like the key to
  your house.

## Split it among friends

In place of one copy in one place, you can split the code into shares. Then
a number of shares together give the code back. For example, five shares, of
which any three are enough:

```sh
hs ctl recovery split --threshold 3 --count 5
```

`hs` asks for the recovery code and prints five lines that start with
`hs-share-`. Give one to each person you trust. One or two shares by
themselves say nothing about the code.

To get the code back, put enough shares together, one on each line:

```sh
hs ctl recovery combine < shares.txt
```

If there are not enough shares, or one is damaged or comes from a different
code, `hs` says so. It does not print a wrong code.

To restore a node with the code, see
[Backups and restore](../../admin/backups/).
