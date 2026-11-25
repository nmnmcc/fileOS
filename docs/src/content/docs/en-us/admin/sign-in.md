---
title: Sign-in for other apps
description: Use the node as an OpenID Connect provider, or protect apps with forward auth.
---

Other apps you run (a media server, a wiki, a photo library) can use your
homeserver for sign-in, so you and your family have one identity and one
place to manage access.

:::caution[Version 0.1.0]
Both methods need a browser that is signed in to the node as its owner.
Without a web app, that sign-in is still basic. Treat this page as a preview.
:::

## OpenID Connect

The node is an OpenID Connect provider (authorization code flow with PKCE).
Apps find everything at `https://NODE/.well-known/openid-configuration`.

Register an app with the owner token:

```sh
curl -H "Authorization: Bearer $OWNER_TOKEN" \
  -d '{"name": "Wiki", "redirect_uris": ["https://wiki.example.org/oauth/callback"], "confidential": true}' \
  https://NODE/oauth/clients
```

The answer holds the `client_id`, the `client_secret` (only for
confidential clients) and the `issuer`. Enter them in the app. ID tokens are
signed with EdDSA and ES256; the keys are at `/oauth/jwks`. List registered
apps with `GET /oauth/clients`.

## Forward auth

For apps without OpenID Connect, a reverse proxy (Caddy, Traefik, nginx) can
ask the node before letting a request through.

1. Set the domain whose apps may use it, with the node stopped:
   `hs ctl config set forward_domain example.org`. Apps on that domain and
   its subdomains are allowed.
2. Point the proxy's forward-auth check at `https://NODE/auth/verify`,
   passing `X-Forwarded-Proto`, `X-Forwarded-Host` and `X-Forwarded-Uri`.
3. Signed-in requests get `200` with `Remote-User` (your identity) and
   `Remote-Name` (your name), which the proxy can pass to the app. Others
   are sent to `https://NODE/auth/login` and back after signing in.

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

The sign-in lasts a week, in a cookie for the forward-auth domain.
