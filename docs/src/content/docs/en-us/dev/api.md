---
title: REST API
description: The typed REST API, its OpenAPI document and the Rust client.
---

The `hs` command line uses the same API as every other client. So anything
it does, your program can do too.

## The OpenAPI document

An OpenAPI 3.1 document describes the API. It is made from the Rust types
that the server uses, and you can get it in three ways:

- every node serves it at `/api/openapi.json`;
- `hs openapi` prints it;
- the repository keeps it as `docs/openapi.json`, and CI checks it.

Use it to make a client in any language, or read it for the details of each
request and response.

## Authentication

Send a token as a bearer token:

```sh
curl -H "Authorization: Bearer $TOKEN" https://NODE/api/spaces
```

The node also takes the token as the password of HTTP Basic authentication
(for DAV clients), and as a session cookie (`POST /api/session`). You make
tokens with [`hs ctl token create`](../../admin/access/). The token's grant
decides what the token may do: a role and, if you want, a list of spaces and
resource types. Requests between nodes do not use tokens; they are signed
with HTTP Message Signatures (RFC 9421).

## Resources

Items live under their space:

```
GET    /api/spaces/{space}/resources?type=note&limit=50
GET    /api/spaces/{space}/resources/{type}/{id}
PUT    /api/spaces/{space}/resources/{type}/{id}
PATCH  /api/spaces/{space}/resources/{type}/{id}    application/merge-patch+json
DELETE /api/spaces/{space}/resources/{type}/{id}
```

HTTP works here as the standard says. Every resource has an `ETag`. Send
`If-Match` to change only the version you read; a write based on an old
version gets `412`. Send `If-None-Match: *` to create a resource only if it
is not there yet. Blobs are stored by BLAKE3 hash at `/api/blobs/{hash}`.
They never change, and they support `Range`.

These work across all spaces: `/api/search`, `/api/timeline`, `/api/near`,
`/api/related/{space}/{type}/{id}`. `/api/events` sends changes as they
happen, as Server-Sent Events.

## The Rust client

The `hs-client` crate has the request and response types, and a typed
client that uses them. `hs` is built on it. Its OpenAPI schemas are behind an
optional feature, so a client that does not need them does not pay for
them.
