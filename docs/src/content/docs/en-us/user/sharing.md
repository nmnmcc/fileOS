---
title: Sharing
description: Share a space with people on other homeservers.
---

You share one space at a time. Each person keeps their own homeserver. One
of them keeps the space in order: this is its **home node**, and the others
follow it. Everyone reads and writes on their own node, and changes get to
the others without any work from you.

## Share a space you keep

Make the space if it is not there yet, and look up its ID:

```sh
hs space create "Family photos"
hs space list
```

Add the other person's node by its address. Your node asks it who it is:

```sh
hs space member add "Family photos" --url hs.friend.example --role writer
```

Roles:

| Role     | Can                                                             |
| -------- | --------------------------------------------------------------- |
| `reader` | read                                                            |
| `writer` | read and write (the default)                                    |
| `owner`  | also add and remove members, and give the space to another node |

Send them the space ID and your node's address.

## Join a space someone shared with you

After the owner has added your node:

```sh
hs space join --home hs.owner.example 4b1d…
```

The space is now in `hs space list` and in search. New changes come to your
node as they happen. To get up to date at once, run `hs space sync SPACE`.

## See and change who is in a space

```sh
hs space show "Family photos"
hs space member remove "Family photos" NODE_KEY
```

`hs space show` lists each member's node key, owner and role.

When you remove a member, new changes stop going to them. What they already
have stays with them.

## Move a space to another node

If the home node is going away, an owner can make another member's node the
home node. Nothing is copied, because every member already has the log:

```sh
hs space handover "Family photos" NODE_KEY
```

:::note[Version 0.1.0]
Shared spaces do not have end-to-end encryption yet. Changes go over TLS,
signed by each node. Every member's node keeps them in plain form. Share
only with people whose homeserver you trust.
:::
