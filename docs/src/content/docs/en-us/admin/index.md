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
cargo build --release -p fos   # the program: target/release/fos
apps                           # the engine and apps: target/apps/*.app
```

A release has the same files: the `fos` program and the `apps/` folder.

## Make a node

```sh
fos node init --engine target/apps/Capability.app
```

This makes the data directory (by default `~/.fileos`; change it with `--dir`
or `FOS_DIR`), a key for the node, and a root folder. The engine is the set of
rules of the node. `Capability.app` is the default engine: the maker of a
folder may do everything in it, and gives others what they may do.

## Run it

```sh
fos node run
```

This starts the network, the apps and the gateway. The gateway is the HTTP
front of the node, on this machine only (`127.0.0.1` and `::1`, port 7341):
`http://fos.localhost:7341/` is the kernel, and `http://<app-id>.localhost:7341/`
is an app. The other `fos` commands use the kernel API, with the token in
`gateway.token` in the data directory. `fos curl` runs curl with that token:
for example, `fos curl /v0/status`.

| Option          | What it does                                                  |
| --------------- | ------------------------------------------------------------- |
| `--listen ADDR` | A libp2p address to listen on (default: TCP on every address) |
| `--peer ADDR`   | A node to connect to at the start                             |
| `--port PORT`   | The port of the gateway (default 7341; 0 is any free port)    |
| `--no-mdns`     | Do not look for nodes on the home network                     |

`fos node status` shows the node key, the addresses it listens on (with `/p2p/…`,
ready to give to another node), the peers and the folders.

## Connect two nodes

On the home network, nodes find each other with mDNS. In other places, give
one node the address of the other:

```sh
fos node connect /ip4/192.0.2.10/tcp/4001/p2p/12D3KooW…
```

The network is [libp2p](https://libp2p.io): TCP with Noise and Yamux, relays
and hole punching. There is no DHT: a node only talks to the nodes you
connect it to, and only sends a folder to the nodes that its rules allow.

## The data directory

```text
node.key          the key of the node (keep it secret)
root              the ID of the root folder
engine/           the engine of this node
blobs/            file contents, named by their hash
folders/*.log     the signed log of each folder
local/*.kv        the data each app keeps on this node only
gateway.token     the token for the gateway
```

A copy of the directory, made while the node is stopped, is a full backup.
When the node starts, it reads every log again and checks each entry with the
engine.

## When something is wrong

- **A folder does not come.** Check that the nodes are connected
  (`fos node status` lists the peers), and that the owner of the folder gave your
  node key `store` on it.
- **Writes wait.** `fos node status` says how many writes wait for their ordering
  node. They are sent when that node is connected again.
- **`FORK` in `fos node status`.** Your engine did not take an entry that the
  ordering node took. The status says which entry and why.
- **More detail.** Run the node with `RUST_LOG=debug fos node run`.
