---
title: Running a node
description: Run the node behind a reverse proxy, with port mapping, and as a system service.
---

```sh
hs ctl serve
```

serves plain HTTP on `127.0.0.1:8080`. Only this machine can get to it. Logs
go to standard error. Stop it with Ctrl-C or `SIGINT`; it first ends what it
is writing.

## Reachable from your network or the Internet

| You have                              | Run                                                                |
| ------------------------------------- | ------------------------------------------------------------------ |
| Only this machine                     | `hs ctl serve`                                                     |
| A home network, no domain             | `hs ctl serve --listen [::]:8443 --tls self-signed`                |
| A domain pointing at a public address | `hs ctl serve` behind [a reverse proxy](#https-on-a-public-domain) |
| A home router in the way              | add `--map-port`                                                   |

**`--tls self-signed`** makes a certificate one time and keeps it in
`keys/self-signed.pem`. Apps will give a warning about it;
`hs login --insecure` lets it through.

**`--map-port`** asks the home router to forward the port (NAT-PMP or PCP),
and keeps renewing the mapping. 0.1.0 does not support routers that only
talk UPnP; on those, forward the port by hand.

## HTTPS on a public domain

homeserver is made for nodes that talk to each other directly, peer to peer.
It does not get certificates for public domains itself. If you want a public
HTTPS address, run a reverse proxy such as Caddy or nginx in front of
`hs ctl serve`. Then set `url` in the config to the public address.

With Caddy, which gets and renews the certificate:

```
home.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

With nginx, and a certificate that you take care of:

```nginx
server {
    listen 443 ssl;
    server_name home.example.org;
    ssl_certificate     /etc/ssl/home.example.org.pem;
    ssl_certificate_key /etc/ssl/home.example.org.key;
    client_max_body_size 0;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_request_buffering off;
        proxy_buffering off;
    }
}
```

Keep `hs` on `127.0.0.1`, so that only the proxy can get to it.

## As a system service

An example systemd unit, for a user named `hs` whose data is in
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
ExecStart=/usr/local/bin/hs ctl serve
KillSignal=SIGINT
Restart=on-failure
StateDirectory=homeserver

[Install]
WantedBy=multi-user.target
```

First make the node as that user, with the same directory:
`sudo -u hs HS_DIR=/var/lib/homeserver hs ctl init --name … --url …`.
Then run `systemctl enable --now homeserver`, and read the logs with
`journalctl -u homeserver`.

## One process at a time

The running node holds `node.lock`. While it runs, commands that can
(tokens, export, replicas) go through its API, not the files. A few commands
need the node to be stopped: `hs ctl config set`, `hs ctl restore` and
`hs ctl rebuild-index`.
