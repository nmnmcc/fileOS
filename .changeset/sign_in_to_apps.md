---
default: minor
---

# Sign in to apps

The kernel now signs people in to apps with OAuth 2.0 and PKCE, at `fileos.localhost/authorize`, `/token` and `/revoke`. An app that has a token gets the person in the `FileOS-User` header; it never sees how the person signed in. Today you sign in with a one-time code from `fileos node sign-in-code`; passkeys come next.
