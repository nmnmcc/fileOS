---
title: Running a node
description: Build fileOS, make a node, run it, and connect it to other nodes.
---

:::caution[Prototype]
fileOS is a prototype. Its data formats will change between versions.
:::

## Build

In the repository, with the devenv shell (it has every tool):

```sh
cargo build --release -p fileos   # the program: target/release/fileos
apps                           # the apps: target/apps/*.app
```

A release has the same files: the `fileos` program and the `apps/` folder.

## Make a node

```sh
fileos node init
```

This makes the data directory (by default `~/.fileos`; change it with `--dir`
or `FILEOS_DIR`), a key for the node, and its home space. Other spaces are
mounted in the home space at paths such as `/photos`.

## Run it

```sh
fileos node run
```

This starts the network, the apps and the gateway. The gateway is the HTTP
front of the node, on this machine only (`127.0.0.1` and `::1`, port 7341):
`http://fileos.localhost:7341/` is the kernel, and `http://<app-id>.localhost:7341/`
is an app. The other `fileos` commands use the kernel API, with the token in
`gateway.token` in the data directory. `fileos curl` runs curl with that token:
for example, `fileos curl /v0/status`.

| Option          | What it does                                                  |
| --------------- | ------------------------------------------------------------- |
| `--listen ADDR` | A libp2p address to listen on (default: TCP on every address) |
| `--peer ADDR`   | A node to connect to at the start                             |
| `--port PORT`   | The port of the gateway (default 7341; 0 is any free port)    |
| `--no-mdns`     | Do not look for nodes on the home network                     |

`fileos node status` shows the node key, the addresses it listens on (with `/p2p/…`,
ready to give to another node), the peers and the spaces.

## Connect two nodes

On the home network, nodes find each other with mDNS. In other places, give
one node the address of the other:

```sh
fileos node connect /ip4/192.0.2.10/tcp/4001/p2p/12D3KooW…
```

The network is [libp2p](https://libp2p.io): TCP with Noise and Yamux, relays
and hole punching. There is no DHT: a node only talks to the nodes you
connect it to, and only sends a space to the nodes that have a grant to keep
it (`data.keep` or `files.read` on the whole space).

## The data directory

```text
node.key          the key of the node (keep it secret)
identity-*.key    the keys of an identity, on the node that holds them
home              the ID of the home space
blobs/            content: chunks and lists of chunks, by their hash
spaces/*.log      the signed entries of each space
spaces/*.base     the snapshot at the last checkpoint, after compaction
local/*.kv        the cache each app keeps on this node only
gateway.token     the token for the gateway
```

A copy of the directory, made while the node is stopped, is a full backup.
When the node starts, it reads every log again and checks every entry.

## Space on disk

A node makes a checkpoint of a space it owns after 1000 entries, and then
drops the entries before it. Old versions get thinner with age: one an hour
for a day, one a day for 30 days, one a week for a year. Deleted files are
kept for 30 days. `fileos space checkpoint <path>` makes one now.

## When something is wrong

- **A space does not come.** Check that the nodes are connected
  (`fileos node status` lists the peers), and that the owner of the space gave your
  node key a grant to keep it (`fileos space grant /photos <key> read`).
- **Writes are not on the other node.** A node writes at any time, also
  offline, and sends its writes when it is connected again.
- **"entries have no effect" in `fileos node status`.** Some entries were not
  allowed: for example, they were made at the same time as access was taken
  back. `fileos space grants <path>` shows the grants.
- **More detail.** Run the node with `RUST_LOG=debug fileos node run`.
