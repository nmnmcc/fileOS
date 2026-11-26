---
title: "Apps as workers"
description: Apps belong to an identity and sync with it, are found by the identity, and run as event-driven workers on the nodes that people choose.
---

:::note[Status: design]
Nothing on this page is built yet. Today an app is installed on one node,
has an ID that comes from that node, and serves HTTP itself (see
[HTTP and the browser](../http/)).
:::

## Apps belong to the identity, not to a node

Files sync between the nodes of an identity, so apps must too. Otherwise each
device has its own copy of an app, with its own address, its own access and
its own data, and the person does not get one experience.

- **Every identity has a home folder** that syncs to all of its nodes. A
  person's folders (`/photos`, …) and apps (`/apps/…`) are mounted there. The
  root folder of a node keeps only what is about that machine: its engine,
  caches and settings.
- **An app is installed into the home folder.** Every node of the identity gets
  the same app.
- **An app has one name in an identity**, and its address is the same on every
  device: `http://photos.localhost:7341/`.
- When a name is used again by a different app, the kernel sends
  `Clear-Site-Data` on its first answer, so that the browser data of the old
  app is not given to the new one.
- On a device with no node, an app is at `<app>.<identity>.<shared domain>`,
  and the request goes to a node of the identity that is online, found
  through [rendezvous](../identity/).
- **An app acts with one key on every node.** The identity gives its nodes a
  shared app seed (sent with HPKE when a node joins), and app keys come from
  it. So access is given once, not once for each node.

## Apps are workers, not servers

An app does not run all the time. It wakes up for an event, does its work,
and stops. This is the model of Cloudflare Workers and of the service workers
of browsers.

| Event      | When                                   | Interface                               |
| ---------- | -------------------------------------- | --------------------------------------- |
| `fetch`    | An HTTP request comes for its origin   | `wasi:http/incoming-handler` (standard) |
| `change`   | A path it watches changes              | `fos:app/events`                        |
| `schedule` | A time that it asked for comes         | to be added                             |
| `message`  | Another app or node sends it a message | later                                   |

- **A new instance for each event**, with limits on fuel, memory and time. An
  app that is not working takes no memory.
- **The manifest says which events the app takes**: which paths, which times.
  When it is installed, the person sees when it can wake up and what it can
  touch.
- **The interface of an app (`ui/`) is given by the kernel**, as files named by
  their hash, which browsers can keep for a long time. No app runs for these
  requests, so most requests run no WebAssembly.
- **An app keeps its data in synced storage** (`kv`, which merges, and its
  files). A `cache` on each node is only for speed: it can be deleted at any
  time. See [Data](../data/).

## Where an event is handled

### A list of nodes for each event

Each event has an **ordered list of nodes** that may handle it:

- People can set the list for each event of each app, or let only some nodes
  handle an event. The settings are in the home folder, so every node reads
  the same ones. The kernel follows them (mechanism); the person's settings are
  the policy.
- With no setting, the default is:
  - `fetch`: the node that got the request first, because it is fastest and
    works offline;
  - `change` and `schedule`: the nodes that are always on first (a VPS, a box
    at home), then the others.
- When no node in the list is online, the event waits. It is not lost.
- When a request comes to a node that is not in the list, the kernel sends it
  over libp2p to the first node in the list that is online, and sends the
  answer back. For example, all requests for the photos app can go to the box
  at home, where the photos are.

### Every node watches, and the next takes over

This is the way of SMTP's backup mail servers, of Amazon SQS's visibility
timeout, and of Temporal's heartbeats:

1. **Every event has an ID** that every node can work out: the hash of the log
   entry and the handler, or the app, the job and the time slot.
2. **The first node in the list handles the event.** The others wait. The k-th
   node waits k × T after it sees the event. If it sees no completion in that
   time, it handles the event itself.
3. **A long job sends heartbeats** ("I am working on it, done in T seconds")
   as messages between nodes, so that the others wait. Heartbeats are not kept
   in a log.
4. **The result and a "done" mark are written as one batch.** The output of an
   app is CRDT data or writes that can be done two times with the same result.
   So if two nodes handle an event (one was slow, or the network split), the
   two results merge into the same state. Only computing is wasted.
5. **Effects outside fileOS** (later: notices, outgoing requests) get the event
   ID as an idempotency key, so that they happen once.

There is no special "local" kind of handler: data that an event makes is
synced, so it is handled once and every node gets the result.

## What changes from today

| Part             | Today                                                   | New                                              |
| ---------------- | ------------------------------------------------------- | ------------------------------------------------ |
| Where apps are   | The root folder of a node; each device installs its own | The home folder of the identity; synced          |
| Address          | By node: `photos-<hash>.localhost`                      | By identity: `photos.localhost` on every device  |
| How apps run     | One instance kept for events; HTTP per request          | A worker: a new instance for each event          |
| Where events run | The node that gets the event                            | The list of nodes the person sets, with fallback |
| App state        | `kv` on one node only                                   | `kv` that syncs and merges; `cache` on each node |
| App keys         | From the node key                                       | From the shared app seed of the identity         |

## Order of work

1. Workers: a new instance for each event, the kernel gives `ui/`, the manifest
   lists events.
2. The home folder: apps and `kv` move into it; `kv` syncs; `cache` is added;
   the shared app seed.
3. Lists of nodes for each event, with fallback and heartbeats.
4. Addresses `<app>.localhost`, and sending requests between nodes.
5. Sign-in (see [HTTP and the browser](../http/)).
