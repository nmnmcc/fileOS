---
title: Administration manual
description: Set up and take care of a homeserver node for yourself or your family.
---

This manual is for the person who sets up a homeserver: for themselves, for
their family, or as a second node for a friend's backups. It takes for
granted that you are comfortable in a terminal. The people who use the node
need only the [user manual](../user/).

:::caution[Version 0.1.0]
homeserver is experimental. Data formats may still change between versions,
so keep an export and an encrypted replica. There are no automatic updates
and no web app yet.
:::

## What you are running

A node is **one program, `hs`, and one data directory**. You do not set up a
database server, a container or a reverse proxy. `hs` runs the node
(`hs ctl serve`), manages it (`hs ctl …`, or `hsctl`, a link to the same
program) and uses it (`hs ls`, `hs search`, …).

## Requirements

- Linux (x86_64 or aarch64) or macOS. macOS is good for trying it out.
- Little memory and CPU: the design goal is one core and 1 GB of RAM for a
  family of ten.
- Disk space for what you keep, and for thumbnails.
- To be reached from outside your home: a router that supports NAT-PMP or
  PCP, or a port you forward by hand. Without either, the node works on your
  local network. A public HTTPS domain is optional; it needs your own
  reverse proxy.

## The path through this manual

1. [Install](install/) `hs`.
2. Make the node and [run it](running/).
3. Give people and apps [tokens](access/).
4. Set up [backups](backups/), and keep the recovery code safe.
5. If you want, let other apps [sign in](sign-in/) with the node.

[Configuration](configuration/) and [Troubleshooting](troubleshooting/)
cover the rest.
