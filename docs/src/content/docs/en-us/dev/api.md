---
title: REST API
description: The typed REST API, its OpenAPI document and the Rust client.
---

The `hs` command line uses the same API as every other client, so anything
it does, your program can do.

## The OpenAPI document

The API is described by an OpenAPI 3.1 document, derived from the Rust types
the server uses:

- served by every node at `/api/openapi.json`;
- printed by `hs openapi`;
- kept in the repository as `docs/openapi.json`, checked by CI.

Generate a client from it in any language, or read it for the details of
each request and response.

## Authentication

Send a token as a bearer token:

```sh
curl -H "Authorization: Bearer $TOKEN" https://NODE/api/spaces
```

The node also accepts the token as the password of HTTP Basic
authentication (for DAV clients) and as a session cookie
(`POST /api/session`). Tokens are made with
[`hs ctl token create`](../../admin/access/); what a token may do is decided
by its grant: a role, and optionally a list of spaces and resource types.
Requests between nodes are signed with HTTP Message Signatures (RFC 9421)
instead.

## Resources

Items live under their space:

```
GET    /api/spaces/{space}/resources?type=note&limit=50
GET    /api/spaces/{space}/resources/{type}/{id}
PUT    /api/spaces/{space}/resources/{type}/{id}
PATCH  /api/spaces/{space}/resources/{type}/{id}    application/merge-patch+json
DELETE /api/spaces/{space}/resources/{type}/{id}
```

HTTP semantics are used as written: every resource has an `ETag`; send
`If-Match` to update only the version you read (a stale write gets `412`)
and `If-None-Match: *` to create only. Blobs are stored by BLAKE3 hash at
`/api/blobs/{hash}`, never change, and support `Range`.

Across spaces: `/api/search`, `/api/timeline`, `/api/near`,
`/api/related/{space}/{type}/{id}`. Changes stream as Server-Sent Events
from `/api/events`.

## The Rust client

The `hs-client` crate holds the request and response types and a typed
client over them; `hs` is built on it. Its OpenAPI schemas are behind an
optional feature, so a client does not pay for them.
