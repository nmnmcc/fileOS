---
title: Troubleshooting
description: Common problems and what to do about them.
---

## Check the node's health

```sh
hs status
```

shows the node, your sign-in, whether the index is healthy, and how much it
has in it. `GET /api/health` gives the same. The node's own messages go to
standard error, or to the journal under systemd.

## Common messages

**"another hs process is using …"**: the node is running and holds the data
directory. Do the work through the running node (most commands do this by
themselves when you are signed in), or stop the node first. `config set`,
`restore` and `rebuild-index` always need it stopped.

**"… holds no node"**: there is no node in that data directory. Check
`--dir` or `$HS_DIR`, or make a node with `hs ctl init`.

**"cannot reach …"**: `hs` is signed in to a node that is not running, or
that it cannot get to. Start the node with `hs ctl serve`, or use `--dir` to
work on a stopped node's files directly.

**"… did not accept that token"**: the token is wrong, or it has expired or
been revoked. Look at `hs ctl token list` on the node, and make a new token.

**A certificate error with `hs login`**: the node uses a self-signed
certificate. Use `hs login --insecure`.

**"Could not map port"**: the router does not talk NAT-PMP or PCP, or has it
turned off. Forward the port on the router by hand.

## Search or the timeline look wrong

Build the index again from the logs. Nothing is lost:

```sh
hs ctl rebuild-index
```

## The machine is gone

Install `hs` on another machine, and
[restore from a replica](../backups/#restore-a-node) with the recovery code.
If the recovery code is lost too, but you split it among friends, get the
shares back and run `hs ctl recovery combine`.
