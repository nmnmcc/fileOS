---
title: Installation
description: Get the hs program and make a node.
---

`hs` is one executable with everything inside, SQLite too. You do not need
to install anything else to run it.

## Build it

There are no ready-made downloads for 0.1.0 yet. Build it from the
repository:

```sh
# with Nix and devenv: a release build in ./result/bin
devenv build outputs.hs

# or with Rust (rustup picks the right version)
cargo build --release --package hs
```

Copy `hs` to a directory on your `PATH`, such as `/usr/local/bin`. If you
want, make a link named `hsctl`. When `hs` starts under that name, it works
as `hs ctl`. The Nix build has the link already.

```sh
ln -s hs /usr/local/bin/hsctl
```

## Create the node

Run this as the user that the node will run as:

```sh
hs ctl init --name "Ada" --url https://hs.example.org
```

- `--name` is shown to the people you share with.
- `--url` is the address that other nodes and apps use to get to this one.
  Leave it out for a node that is used only on this machine; you can
  [set it later](../configuration/).

`hs ctl init` prints three things:

```
Identity: n22carktxylkeljiv6fipjzz3xyusp6hm6wxvl2xiksnoui75vgq

Recovery code (write it down; it is not stored anywhere):

    l6zf-miqq-zl5c-…

Owner token for your first browser or app:

    hs_d3sje7yqzelf…
```

**Write the recovery code down now.** You see it only this one time, and it
is the only way to restore from an encrypted backup. See
[Your recovery code](../../user/recovery/).

The owner token can do everything. `hs` on this machine is already signed in
with it. Keep it for yourself, and give other people their own
[tokens](../access/).

## The data directory

By default the node lives in `$XDG_DATA_HOME/homeserver`, which is mostly
`~/.local/share/homeserver`. To use another directory, give `--dir DIR` or
`$HS_DIR` on every command.

```
config.json   the configuration
identity      the identity log
keys/         node and storage keys, certificates (secret)
logs/         one directory per space: the signed logs
blobs/        file contents by BLAKE3 hash
index.db      search, timeline, places: derived, safe to delete
access.log    requests made to the node
node.lock     held by the running process
```

`logs/`, `blobs/`, `identity`, `config.json` and `keys/` are the node. You
can make everything else again from them.

Next: [run the node](../running/).
