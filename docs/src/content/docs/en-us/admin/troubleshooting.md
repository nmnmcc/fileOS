---
title: Troubleshooting
description: Common problems and what to do about them.
---

## Check the node's health

```sh
hs status
```

shows the node, your sign-in, whether the index is healthy and how much it
holds. The same is at `GET /api/health`. The node's own messages go to
standard error, or to the journal under systemd.

## Common messages

**"another hs process is using …"**: the node is running and holds the data
directory. Do it through the running node instead (most commands do this on
their own when you are signed in), or stop the node first. `config set`,
`restore` and `rebuild-index` always need it stopped.

**"… holds no node"**: there is no node in that data directory. Check
`--dir` or `$HS_DIR`, or create one with `hs ctl init`.

**"cannot reach …"**: `hs` is signed in to a node that is not running or
not reachable. Start it with `hs ctl serve`, or use `--dir` to work on a
stopped node's files directly.

**"… did not accept that token"**: the token is wrong, expired or revoked.
Check `hs ctl token list` on the node and create a new one.

**A certificate error with `hs login`**: the node uses a self-signed
certificate. Use `hs login --insecure`.

**`--tls acme` fails to get a certificate**: the domain in `url` (or
`--domain`) must point at this machine, and ports 80 and 443 must be
reachable from the Internet. If a renewal fails, the node keeps serving the
previous certificate and tries again a day later.

**"Could not map port"**: the router does not speak NAT-PMP or PCP, or has
it turned off. Forward the ports on the router by hand.

## Search or the timeline look wrong

Rebuild the index from the logs. Nothing is lost:

```sh
hs ctl rebuild-index
```

## The machine is gone

Install `hs` on another machine and
[restore from a replica](../backups/#restore-a-node) with the recovery code.
If the recovery code is lost too but you shared it among friends, collect
the shares and run `hs ctl recovery combine`.
