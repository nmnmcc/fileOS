---
title: Backups and restore
description: Plain exports, encrypted replicas, restoring a node and rebuilding the index.
---

There are two kinds of backup, and you need both:

- an **export**: your data in plain form, which you can read without
  homeserver;
- an **encrypted replica**: a copy of the whole node, kept up to date. With
  only the recovery code, it brings the node back.

## Export

```sh
hs ctl export > homeserver-2026-11-25.tar
```

This is a plain tar archive of the logs, blobs, identity and configuration.
It does not have the keys in it, but it does have your data, not encrypted;
keep it in a safe place. It works while the node runs.

## Encrypted replicas

A replica is an encrypted copy of the data directory (without the index and
the access log). Each file is encrypted before it goes out, with a key made
from the recovery code. It is stored under a name that gives nothing away.
The holder sees only encrypted data.

Add one or more places, with the node stopped:

```sh
hs ctl config set replicas '["/mnt/usb/homeserver", "s3://KEY:SECRET@s3.example.com/my-bucket/hs"]'
```

| Form                                          | Where                                              |
| --------------------------------------------- | -------------------------------------------------- |
| `/some/dir` or `dir:/some/dir`                | a directory: a USB disk, a NAS mount               |
| `s3://KEY:SECRET@host[:port]/bucket[/prefix]` | any S3-compatible store; add `?region=…` if needed |
| `hs://KEY:SECRET@host[:port]/bucket[/prefix]` | another homeserver, such as a friend's (see below) |

Add `?http=1` for a store without TLS. Characters such as `/` in a secret
must be percent-encoded (`%2F`).

The running node syncs every replica each hour. It sends only the files that
changed. To sync now and see the result:

```sh
hs ctl replica sync
hs ctl replica list
```

## Restore a node

On a new machine, with an empty data directory and only the recovery code:

```sh
hs ctl restore --from /mnt/usb/homeserver
```

`hs` asks for the recovery code (or reads `$HS_RECOVERY_CODE`). It gets and
decrypts every file, then builds the index again. Then start the node with
`hs ctl serve`. Tokens and shared spaces work as before.

## Hold a friend's replica

Your node can keep a friend's encrypted replica, and they can keep yours.
Give each friend a key that gets to one bucket:

```sh
hs ctl config set s3_keys '[{"access_key": "bea", "secret_key": "a long random secret", "bucket": "bea"}]'
```

Their files go to `buckets/bea/` in your data directory; your friend has
already encrypted them. Your friend then adds
`hs://bea:SECRET@your-node.example/bea` to their own `replicas`. Other S3
tools, such as `rclone`, can use the same endpoint at `/s3`.

## Rebuild the index

The index (`index.db`) has search, the timeline and places in it. All of
them are made from the logs. If it seems wrong, build it again with the node
stopped:

```sh
hs ctl rebuild-index
```

If you delete `index.db`, the node builds it again at the next start; the
effect is the same.
