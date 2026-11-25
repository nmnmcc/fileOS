---
title: Running a node
description: Serve the node, with TLS, port mapping and as a system service.
---

```sh
hs ctl serve
```

serves plain HTTP on `127.0.0.1:8080`, reachable only from this machine. Logs
go to standard error. Stop it with Ctrl-C or `SIGINT`; it finishes what it is
writing first.

## Reachable from your network or the Internet

| You have                              | Run                                                 |
| ------------------------------------- | --------------------------------------------------- |
| Only this machine                     | `hs ctl serve`                                      |
| A home network, no domain             | `hs ctl serve --listen [::]:8443 --tls self-signed` |
| A domain pointing at a public address | `hs ctl serve --tls acme --email you@example.org`   |
| The same, behind a home router        | add `--map-port`                                    |

**`--tls self-signed`** makes a certificate once and keeps it in
`keys/self-signed.pem`. Apps will warn about it; `hs login --insecure`
accepts it.

**`--tls acme`** gets a certificate from Let's Encrypt for the host in the
configured `url` (or each `--domain`), and renews it automatically. It
listens on `[::]:443`, and on `[::]:80` for the ACME challenge; both must be
reachable from the Internet. Use `--acme-directory URL` for another ACME
authority, and `--http-listen` to move the challenge listener.

**`--map-port`** asks the home router to forward the ports (NAT-PMP or PCP)
and keeps renewing the mapping. Routers that only speak UPnP are not
supported in 0.1.0; forward the ports by hand there.

Ports below 1024 need root or the capability to bind them; the service
below grants that capability without running as root.

## As a system service

An example systemd unit, for a user called `hs` whose data lives in
`/var/lib/homeserver`:

```ini
# /etc/systemd/system/homeserver.service
[Unit]
Description=homeserver
After=network-online.target
Wants=network-online.target

[Service]
User=hs
Environment=HS_DIR=/var/lib/homeserver
ExecStart=/usr/local/bin/hs ctl serve --tls acme --email you@example.org
KillSignal=SIGINT
Restart=on-failure
AmbientCapabilities=CAP_NET_BIND_SERVICE
StateDirectory=homeserver

[Install]
WantedBy=multi-user.target
```

Create the node first as that user, with the same directory:
`sudo -u hs HS_DIR=/var/lib/homeserver hs ctl init --name … --url …`.
Then `systemctl enable --now homeserver`, and read the logs with
`journalctl -u homeserver`.

## One process at a time

The running node holds `node.lock`. While it runs, commands that can
(tokens, export, replicas) go through its API instead of the files. A few
commands need the node stopped: `hs ctl config set`, `hs ctl restore` and
`hs ctl rebuild-index`.
