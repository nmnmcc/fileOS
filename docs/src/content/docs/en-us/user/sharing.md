---
title: Sharing
description: Share a space with people on other homeservers.
---

You share by space. Each person keeps their own homeserver; the space is
kept in order by one of them, its **home node**, and the others follow it.
Everyone reads and writes on their own node, and changes reach the others
on their own.

## Share a space you keep

Create the space if it does not exist yet, and look up its ID:

```sh
hs space create "Family photos"
hs space list
```

Add the other person's node by its address. Your node asks it who it is:

```sh
hs space member add "Family photos" --url hs.friend.example --role writer
```

Roles:

| Role     | Can                                        |
| -------- | ------------------------------------------ |
| `reader` | read                                       |
| `writer` | read and write (the default)               |
| `owner`  | also add and remove members, and hand over |

Send them the space ID and your node's address.

## Join a space someone shared with you

After the owner has added your node:

```sh
hs space join --home hs.owner.example 4b1d…
```

The space now appears in `hs space list` and in search, and new changes are
pushed to your node as they happen. To catch up straight away, run
`hs space sync SPACE`.

## See and change who is in a space

```sh
hs space show "Family photos"
hs space member remove "Family photos" NODE_KEY
```

`hs space show` lists each member's node key, owner and role.

Removing a member stops new changes reaching them. What they already have
stays with them.

## Move a space to another node

If the home node is going away, an owner can make another member's node the
home node. Nothing is copied, because every member already has the log:

```sh
hs space handover "Family photos" NODE_KEY
```

:::note[Version 0.1.0]
Shared spaces are not yet end-to-end encrypted. Changes travel over TLS,
signed by each node, and are kept in plain form on every member's node.
Share only with people whose homeserver you trust.
:::
