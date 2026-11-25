---
title: Sign-in for other apps
description: Use the node as an OpenID Connect provider, or keep apps safe with forward auth.
---

Other apps you run (a media server, a wiki, a photo library) can use your
homeserver for sign-in. Then you and your family have one identity, and one
place to control access.

:::caution[Version 0.1.0]
Both ways need a browser that is signed in to the node as its owner. With no
web app, that sign-in is still very simple. Take this page as an early look.
:::

## OpenID Connect

The node is an OpenID Connect provider (authorization code flow with PKCE).
Apps get all the details from `https://NODE/.well-known/openid-configuration`.

Register an app with the owner token:

```sh
curl -H "Authorization: Bearer $OWNER_TOKEN" \
  -d '{"name": "Wiki", "redirect_uris": ["https://wiki.example.org/oauth/callback"], "confidential": true}' \
  https://NODE/oauth/clients
```

The answer has the `client_id`, the `client_secret` (only for confidential
clients) and the `issuer`. Put them into the app. ID tokens are signed with
EdDSA and ES256; the keys are at `/oauth/jwks`. To list registered apps, use
`GET /oauth/clients`.

## Forward auth

For apps without OpenID Connect, a reverse proxy (Caddy, Traefik, nginx) can
ask the node before it lets a request through.

1. With the node stopped, set the domain whose apps may use it:
   `hs ctl config set forward_domain example.org`. Apps on that domain and
   its subdomains are allowed.
2. Point the proxy's forward-auth check at `https://NODE/auth/verify`, and
   send `X-Forwarded-Proto`, `X-Forwarded-Host` and `X-Forwarded-Uri`.
3. Signed-in requests get `200` with `Remote-User` (your identity) and
   `Remote-Name` (your name); the proxy can send these on to the app. Other
   requests go to `https://NODE/auth/login`, and come back after sign-in.

For example, with Caddy:

```
wiki.example.org {
  forward_auth https://hs.example.org {
    uri /auth/verify
    header_up Host {upstream_hostport}
    copy_headers Remote-User Remote-Name
  }
  reverse_proxy localhost:3000
}
```

The sign-in lasts one week. It is kept in a cookie for the forward-auth
domain.
