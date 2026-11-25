---
title: Getting started
description: Sign in to your homeserver with the hs command line and look around.
---

You need two things from the person who runs your homeserver (it may be
you):

- its **address**, such as `hs.example.org`, or `127.0.0.1:8080` on the
  same machine;
- a **token**: a long secret that lets you in. The person who set up the
  node got an owner token when they made it; other people get one from
  [`hs ctl token create`](../../admin/access/).

You also need the `hs` program on your computer. See
[Installation](../../admin/install/).

## Sign in

```sh
hs login hs.example.org
```

`hs` asks for the token. It does not show the token when you paste it. It
can also read the token from standard input or from `$HS_TOKEN`. When the
node takes the token, `hs` keeps this node in memory:

```
Signed in to Ada (https://hs.example.org) as owner
```

If the node uses a self-signed certificate, add `--insecure`. You can sign
in to more than one node. `hs` uses the last one, or the one you give with
`--server URL` (or `$HS_SERVER`). `hs logout` forgets a sign-in.

## Look around

```sh
hs status          # which node, who you are, and whether it is healthy
hs space list      # the spaces you can see
hs timeline        # recent items, newest first
```

Every command gives its own help with `--help`. Add `--json` to any command
to get exactly what the API sends back; this is useful for scripts.

## Complete commands as you type

`hs` can complete commands, space names and grants in your shell. Put one
line in your shell's startup file:

```sh
source <(hs completion bash)    # bash; use zsh for zsh
hs completion fish | source     # fish
```

Elvish and PowerShell work too.
