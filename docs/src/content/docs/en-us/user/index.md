---
title: User manual
description: What fileOS is, and how you use it.
---

:::caution[Prototype]
fileOS is a prototype. You use it from the command line, and its formats will
change. Keep a copy of anything that is important to you somewhere else too.
:::

## What fileOS is

fileOS keeps your files on computers that you control: a small box at home, a
server, your laptop. Each of these is a **node**. Your nodes, and the nodes of
people you know, keep each other up to date.

There are only a few ideas:

- **Everything is a file.** Your photos, your notes, each message of a chat
  with your family, and even the apps are files.
- **Files are in spaces.** A space is a set of files that syncs as one, such
  as `/photos`. When you share a space with a friend, their node keeps a
  copy, and changes go both ways. Every node can write at any time, also
  offline.
- **Access is a grant, and you can see it.** A grant says who may do what
  (read, change, give on, keep) on which files. You can see every grant, and
  take any of them back; what was given on from it goes too.
- **Apps only see what you let them see.** An app is a small program in a
  sandbox. It can only use the paths you gave it.

## The `fos` command

`fos` works with the kernel of your node. Its commands are in groups:

| Group                     | What it is for                                                  |
| ------------------------- | --------------------------------------------------------------- |
| `fos node`                | This node: `init`, `run`, `status`, `connect`                   |
| `fos fs`                  | Files: `ls`, `cat`, `put`, `append`, `rm`                       |
| `fos space`               | Spaces: `new`, `mount`, `grant`, `grants`, `take`, `checkpoint` |
| `fos identity` (`fos id`) | Your identity: `new`, `add-node`, `remove-node`, `join`         |
| `fos app`                 | `pack` or `install` an app                                      |
| `fos curl`                | curl with your node's token, for example `fos curl /v0/status`  |

What an app does is not in `fos`; you see it in the app. `fos help <group>`
says more about each group.

## Using your files

These commands work on the node that runs on your computer. Paths start with
the place a space is mounted, for example `/photos/2026/beach.jpg`.

| Command                            | What it does                         |
| ---------------------------------- | ------------------------------------ |
| `fos fs ls /photos`                | The names under a path               |
| `fos fs cat /notes/todo.txt`       | Print a file                         |
| `fos fs put /notes/todo.txt FILE`  | Put a file (or the text you type)    |
| `fos fs rm /notes/todo.txt`        | Delete a file; old versions are kept |
| `fos fs append /notes/log.txt "…"` | Add a line to the end of a file      |
| `fos node status`                  | What your node is doing              |

## Sharing a space

1. Make a space: `fos space new /family`. It prints the space ID.
2. Give your friend's node access:
   `fos space grant /family <their node key> read-write`. It prints the ID of
   the grant.
3. Your friend mounts it on their node: `fos space mount <space ID> /family`.

Now you both see the same files, and either of you can write, also offline.
When you both change the same file at the same time, both versions are kept.

What you can give:

| Template     | What it gives                               |
| ------------ | ------------------------------------------- |
| `read`       | List, read, see history, watch, keep a copy |
| `read-write` | `read`, and make, change and delete files   |
| `manage`     | `read-write`, and give and take back access |
| `relay`      | Keep a copy only (for a backup node)        |

You can also give exact permissions, such as `files.read,files.create`. A
grant on a path that is a file follows that file when it is renamed; a grant
on a directory is on every name under it.

To share with a person and all their devices, give the access to their
identity in place of a node key:
`fos space grant /family did:fos:… read-write`. Each of their nodes, also
ones they add later, then gets the space.

`fos space grants /family` lists the grants. `fos space take /family <grant ID>`
takes one back, and with it what was given on from it. What someone did at
the same time as you took their access back has no effect.

## One identity, many nodes

Your identity is you; a node is one of your machines. Your identity is a DID
(`did:fos:…`), and with it all your nodes are known to be yours.

1. On the node that is to hold the keys of your identity (your laptop, for
   example), run `fos identity new`. It prints your DID. Keep a copy of the two key
   files that it names in a safe place.
2. On your other node, run `fos node status` and copy its node key.
3. On the first node, run `fos identity add-node <node key>`. It prints a command.
4. On the other node, run that command: `fos identity join <space ID>`.

`fos identity` shows your identity, its nodes and its link. The link is what you
give to other people. Only the node that holds the signing key can add nodes
(`fos identity add-node`) or take them away (`fos identity remove-node`).

## Apps

An app comes as an `.app` file, such as `Photos.app`. Install one with
`fos app install Photos.app`. When you install it, your node gives the app only
the access that its manifest asks for. `fos app install` prints the address
of the app, such as `http://photos-…localhost:7341/`; open it in a browser.
The `fos` command is for the kernel only: what an app does, you see in the
app.

An app sees only its own view, never where your files really are:

| Command                                                 | What it does                                     |
| ------------------------------------------------------- | ------------------------------------------------ |
| `fos app grants photos`                                 | What the app sees, and what each part stands for |
| `fos app grant photos /pictures read`                   | Give it more; it sees it at `/granted/<handle>`  |
| `fos app grant photos /pictures read-write --as photos` | Make its `/photos` be your `/pictures`           |
| `fos app take photos granted/<handle>`                  | Take it away again                               |

Each app also has its own private data, which other apps cannot read.

An app can also ask you for something, such as a folder of photos to show.
`fos app requests` lists what apps ask for; `fos app answer <id> /holiday`
gives it, and `fos app refuse <id>` says no.

Two apps come with fileOS:

- **Photos** shows the photos in `/photos`, lets you upload more, and keeps an
  index of their sizes.
- **Chat** keeps rooms in `/chat`, one file for each message. Make the space
  with `fos space new /chat`, share it with your friends' nodes, and each of
  you talks through the chat app on your own node.

To run a node, see the [administration manual](../admin/).
