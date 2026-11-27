---
default: minor
---

# Sign in to apps with a passkey

The kernel now signs people in to apps, with no cookies and no passwords. `fileos identity passkey` gives a one-time link to add a passkey. An app sends the browser to the node's `/authorize` page (OAuth 2.0 with PKCE); the person signs in with the passkey; the app gets tokens at `/token` that are bound to a key of the browser (DPoP), and its requests then reach it with the person's DID in `Fileos-User`. Refresh tokens change at each use, and `/revoke` takes a sign-in back.
