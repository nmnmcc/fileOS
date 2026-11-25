---
title: Administration manual
description: Set up and look after a homeserver node for yourself or your family.
---

This manual is for whoever sets up a homeserver: for themselves, for their
family, or as a second node for a friend's backups. It assumes you are at
home in a terminal. The people who use the node need only the
[user manual](../user/).

:::caution[Version 0.1.0]
homeserver is experimental. Data formats may still change between versions,
so keep an export and an encrypted replica. Automatic updates and the web app
are not there yet.
:::

## What you are running

A node is **one program, `hs`, and one data directory**. There is no
database server, container or reverse proxy to set up. `hs` runs the node
(`hs ctl serve`), manages it (`hs ctl …`, or `hsctl`, a link to the same
program) and uses it (`hs ls`, `hs search`, …).

## Requirements

- Linux (x86_64 or aarch64) or macOS. macOS is fine for trying it out.
- Little memory and CPU: the design target is one core and 1 GB of RAM for a
  family of ten.
- Disk for what you keep, plus thumbnails.
- To be reached from outside your home: either a public address with ports
  80 and 443, or a router that supports NAT-PMP or PCP. Without either, the
  node works on your local network.

## The path through this manual

1. [Install](install/) `hs`.
2. Create the node and [run it](running/), with TLS if it is reachable from
   outside.
3. Give people and apps [tokens](access/).
4. Set up [backups](backups/), and keep the recovery code safe.
5. Optionally, let other apps [sign in](sign-in/) with the node.

[Configuration](configuration/) and [Troubleshooting](troubleshooting/)
cover the rest.
