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

- **Everything is a folder.** Your photos, your notes, a chat with your family
  and even the apps are folders.
- **A shared folder is a folder that more than one node keeps.** When you share
  a folder with a friend, their node keeps a copy, and changes go both ways.
- **Your node, your rules.** Each node has rules that say who may read, write
  or keep what. You can see every rule, and take any of them back.
- **Apps only see what you let them see.** An app is a small program in a
  sandbox. It can only use the folders you gave it.

## The `fos` command

`fos` works with the kernel of your node. Its commands are in groups:

| Group                     | What it is for                                          |
| ------------------------- | ------------------------------------------------------- |
| `fos node`                | This node: `init`, `run`, `status`, `connect`           |
| `fos fs`                  | Files and streams: `ls`, `cat`, `put`, `append`, `rm`   |
| `fos folder`              | Folders: `new`, `mount`, `grant`                        |
| `fos identity` (`fos id`) | Your identity: `new`, `add-node`, `remove-node`, `join` |
| `fos app`                 | `install` an app                                        |

What an app does is not in `fos`; you see it in the app. `fos help <group>`
says more about each group.

## Using your files

These commands work on the node that runs on your computer. Paths start with
the folder, for example `/photos/2026/beach.jpg`.

| Command                           | What it does                            |
| --------------------------------- | --------------------------------------- |
| `fos fs ls /photos`               | The names in a folder                   |
| `fos fs cat /notes/todo.txt`      | Print a file                            |
| `fos fs put /notes/todo.txt FILE` | Put a file (or the text you type)       |
| `fos fs rm /notes/todo.txt`       | Take a file away; old versions are kept |
| `fos fs append /family/chat "…"`  | Add a line to a stream, such as a chat  |
| `fos node status`                 | What your node is doing                 |

## Sharing a folder

1. Make a folder: `fos folder new family /family`. It prints the folder ID.
2. Give your friend's node access: `fos folder grant /family <their node key> all`.
3. Your friend mounts it on their node: `fos folder mount <folder ID> /family`.

Now you both see the same files. Your friend's writes go to your node, which
puts them in order, and then come back to everyone.

If your friend's node has different rules from yours, its copy can stop at
the first change its rules do not take. `fos node status` then says `FORK`. Nothing
is lost; your friend can change their rules or make a new folder.

## One identity, many nodes

Your identity is you; a node is one of your machines. Your identity is a DID
(`did:fos:…`), and with it all your nodes are known to be yours.

1. On the node that is to hold the keys of your identity (your laptop, for
   example), run `fos identity new`. It prints your DID. Keep a copy of the two key
   files that it names in a safe place.
2. On your other node, run `fos node status` and copy its node key.
3. On the first node, run `fos identity add-node <node key>`. It prints a command.
4. On the other node, run that command: `fos identity join <folder ID>`.

`fos identity` shows your identity, its nodes and its link. The link is what you
give to other people. Only the node that holds the signing key can add nodes
(`fos identity add-node`) or take them away (`fos identity remove-node`).

## Apps

An app is a folder with a `.app` name, such as `Photos.app`. Install one with
`fos app install Photos.app`. When you install it, your node gives the app only
the access that its manifest asks for. The photos app reads the photos in
`/photos` and keeps an index on your node. The `fos` command is for the
kernel only: what an app does, you see in the app.

To run a node, see the [administration manual](../admin/).
