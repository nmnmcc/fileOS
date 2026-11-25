---
title: Getting started
description: Sign in to your homeserver with the hs command line and look around.
---

You need two things from whoever runs your homeserver (it may be you):

- its **address**, such as `hs.example.org`, or `127.0.0.1:8080` on the
  same machine;
- a **token**: a long secret that lets you in. The person who set up the
  node got an owner token when they created it; others get one from
  [`hs ctl token create`](../../admin/access/).

You also need the `hs` program on your computer. See
[Installation](../../admin/install/).

## Sign in

```sh
hs login hs.example.org
```

`hs` asks for the token and does not show it as you paste. It can also read
it from standard input or from `$HS_TOKEN`. When the token is accepted, `hs`
remembers this node:

```
Signed in to Ada (https://hs.example.org) as owner
```

If the node uses a self-signed certificate, add `--insecure`. You can sign
in to several nodes; `hs` uses the last one, or the one given with
`--server URL` (or `$HS_SERVER`). `hs logout` forgets a sign-in.

## Look around

```sh
hs status          # which node, who you are, and whether it is healthy
hs space list      # the spaces you can see
hs timeline        # recent items, newest first
```

Every command explains itself with `--help`. Add `--json` to any command to
get exactly what the API returns, for scripts.

## Complete commands as you type

`hs` can complete commands, space names and grants in your shell. Add one
line to your shell's startup file:

```sh
source <(hs completion bash)    # bash; use zsh for zsh
hs completion fish | source     # fish
```

Elvish and PowerShell are supported too.
