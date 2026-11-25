---
title: Installation
description: Get the hs executable and create a node.
---

`hs` is a single executable with everything inside, including SQLite. It
needs nothing else installed to run.

## Build it

There are no prebuilt downloads for 0.1.0 yet. Build from the repository:

```sh
# with Nix and devenv: a release build in ./result/bin
devenv build outputs.hs

# or with Rust (rustup picks the right version)
cargo build --release --package hs
```

Copy `hs` somewhere on your `PATH`, such as `/usr/local/bin`. Optionally
make a link called `hsctl`; started under that name, `hs` behaves as
`hs ctl`. The Nix build includes the link.

```sh
ln -s hs /usr/local/bin/hsctl
```

## Create the node

Run this as the user the node will run as:

```sh
hs ctl init --name "Ada" --url https://hs.example.org
```

- `--name` is shown to the people you share with.
- `--url` is the address other nodes and apps use to reach this one. Leave
  it out for a node used only on this machine; you can
  [set it later](../configuration/).

`hs ctl init` prints three things:

```
Identity: n22carktxylkeljiv6fipjzz3xyusp6hm6wxvl2xiksnoui75vgq

Recovery code (write it down; it is not stored anywhere):

    l6zf-miqq-zl5c-…

Owner token for your first browser or app:

    hs_d3sje7yqzelf…
```

**Write the recovery code down now.** It is shown only once, and it is the
only way to restore from an encrypted backup. See
[Your recovery code](../../user/recovery/).

The owner token can do everything. `hs` on this machine is signed in with it
already; keep it for yourself and give others their own
[tokens](../access/).

## The data directory

By default the node lives in `$XDG_DATA_HOME/homeserver`, usually
`~/.local/share/homeserver`. Choose another with `--dir DIR` or `$HS_DIR` on
every command.

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

`logs/`, `blobs/`, `identity`, `config.json` and `keys/` are the node.
Everything else can be rebuilt.

Next: [run the node](../running/).
