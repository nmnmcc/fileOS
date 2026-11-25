---
title: Running a node
description: Serve the node, behind a reverse proxy, with port mapping and as a system service.
---

```sh
hs ctl serve
```

serves plain HTTP on `127.0.0.1:8080`, reachable only from this machine. Logs
go to standard error. Stop it with Ctrl-C or `SIGINT`; it finishes what it is
writing first.

## Reachable from your network or the Internet

| You have                              | Run                                                     |
| ------------------------------------- | ------------------------------------------------------- |
| Only this machine                     | `hs ctl serve`                                          |
| A home network, no domain             | `hs ctl serve --listen [::]:8443 --tls self-signed`     |
| A domain pointing at a public address | `hs ctl serve` behind [a reverse proxy](#https-on-a-public-domain) |
| A home router in the way              | add `--map-port`                                        |

**`--tls self-signed`** makes a certificate once and keeps it in
`keys/self-signed.pem`. Apps will warn about it; `hs login --insecure`
accepts it.

**`--map-port`** asks the home router to forward the ports (NAT-PMP or PCP)
and keeps renewing the mapping. Routers that only speak UPnP are not
supported in 0.1.0; forward the ports by hand there.

## HTTPS on a public domain

homeserver is built for reaching nodes directly, peer to peer. It does not
get certificates for public domains itself. If you want a public HTTPS
address, run a reverse proxy such as Caddy or nginx in front of
`hs ctl serve`, and set `url` in the config to the public address.

With Caddy, which gets and renews the certificate:

```
home.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

With nginx, given a certificate you manage:

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

Keep `hs` on `127.0.0.1` so that only the proxy reaches it.

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
ExecStart=/usr/local/bin/hs ctl serve
KillSignal=SIGINT
Restart=on-failure
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
