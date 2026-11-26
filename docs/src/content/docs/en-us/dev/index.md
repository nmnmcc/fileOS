---
title: Developer manual
description: For people who build fileOS or build on it.
---

This manual is for two kinds of developer: people who work on fileOS itself,
and people who write apps and engines for it.

## Building on fileOS

Everything on top of the core is a WebAssembly component in an `.app`
folder: the apps, and the engine that makes the rules of a node.

- [Writing apps and engines](apps/): the interfaces, the SDK, the `.app`
  folder and the manifest.
- The interfaces are in [`wit/fos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fos.wit).

## Working on fileOS

- [Contributing](contributing/): how to set up, the rules a change must
  keep, and how to send it.
- [Architecture](fileos/): how fileOS is made, and why.
- [Projects to learn from](prior-art/): projects we study, and what we take
  from each.

Principles come first. See "System model" in the
[README](https://github.com/nmnmcc/fileOS/blob/main/README.md): the kernel keeps
its contracts, apps are user space, and kernel plugins never change what apps
see. We judge every change by this.
