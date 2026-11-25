---
title: Your recovery code
description: Keep the recovery code safe, and split it among friends if you like.
---

When a homeserver is created, it prints a **recovery code** once. It is the
key to your identity, and it decrypts your backups. It is not stored
anywhere, on the node or elsewhere.

With the recovery code alone, a new node can be brought back from a backup.
Without it, an encrypted backup cannot be read by anyone.

## Keep it safe

- Write it on paper and keep it where you keep important documents.
- Do not keep it on the homeserver, or only in a password manager that is
  itself on the homeserver.
- Anyone who has it can restore your data. Treat it like the key to your
  home.

## Split it among friends

Instead of one copy in one place, you can split the code into shares, so
that some of them together bring it back. For example, five shares of which
any three are enough:

```sh
hs ctl recovery split --threshold 3 --count 5
```

`hs` asks for the recovery code and prints five lines starting with
`hs-share-`. Give one to each person you trust. One or two shares on their
own reveal nothing.

To get the code back, collect enough shares, one per line:

```sh
hs ctl recovery combine < shares.txt
```

If there are too few shares, or one is damaged or comes from another code,
`hs` says so instead of printing a wrong code.

Restoring a node with the code is described in
[Backups and restore](../../admin/backups/).
