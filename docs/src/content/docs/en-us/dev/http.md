---
title: "HTTP and the browser"
description: How apps and plugins give their services over HTTP, each with an origin of its own, with no domains for people to configure and no cookies.
---

:::note[Status: design]
Nothing on this page is built yet.
:::

## The goal

- **Everything goes through HTTP.** The command line, the web interface and
  other clients all use the HTTP API of the kernel.
- **Apps and plugins are HTTP servers.** Any of them can give a service or an
  interface to people over HTTP.
- **People configure no domains.** No DNS records, no certificates.
- **Apps get the full web platform**, and they are kept apart from each other
  in the browser.

## One origin for each app

The edge of safety in a browser is the origin: the scheme, the host name and
the port. Two apps with the same origin can read each other's storage and
make requests as each other. So every app has an origin of its own:

```
http://fos.localhost:<port>/            the kernel: interface, sign-in, API
http://<app-id>.localhost:<port>/       one origin for each app
http://<plugin-id>.localhost:<port>/    a plugin with an interface
```

- **No configuration.** RFC 6761 says that every name under `.localhost` is
  this machine. Chrome, Firefox and Safari all go to this machine for
  `*.localhost` with no DNS.
- **No certificates.** Browsers take `localhost` as a secure context, so
  service workers, WebAuthn and the other web APIs work.
- **The app ID does not change and is never used again.** The storage of the
  browser goes with the origin. If a new app got the name of an app that was
  deleted, it would get that app's data. So the app ID comes from the ID of
  the app folder: its name and the start of the hash, for example
  `photos-k3x9q2m7ab` (at most 63 characters, as a DNS label).

Different ports do not keep cookies apart (RFC 6265), and paths do not keep
anything apart in a browser. That is why the app ID is in the host name, not
in the port or the path.

## Apps and plugins give HTTP with `wasi:http`

We do not make our own interface. A component that gives HTTP exports the
`wasi:http/incoming-handler` of WASI 0.2, which wasmtime runs.

- The kernel takes the connection and reads the request. The component gets
  only the request object: no sockets, and no requests of other origins.
- There is no middleware. A component cannot see or change the requests of
  another origin. This keeps the rule that a plugin does not change what apps
  see.
- Every request has limits on fuel, memory and time. Bodies are streams.

## No cookies: the kernel does sign-in

The kernel does all sign-in, with standard protocols. Apps do not see
passwords, sessions or tokens.

1. The app is opened at `http://photos-k3x9.localhost/`. Its static files
   need no sign-in: they are the code of the app, not data.
2. The app has no token, so it sends the browser to
   `http://fos.localhost/authorize`, with a PKCE challenge.
3. The kernel's page signs the person in (with a passkey) and asks if the app
   may act for them. The page is on the origin of the kernel, so the app
   cannot make a false one.
4. The kernel sends the browser back with a code. The app changes the code
   for a token at `http://fos.localhost/token` (CORS lets only that app's
   origin in).
5. The app sends the token in the `Authorization` header. The kernel checks
   it, takes the header away, puts in `Fos-User: did:fos:…`, and gives the
   request to the app.

The standards are OAuth 2.0 (RFC 6749) with PKCE (RFC 7636), as in "OAuth 2.0
for Browser-Based Applications". Tokens are bound to a key that cannot be
taken out of the browser, with DPoP (RFC 9449), so a stolen token is of no use
somewhere else. Refresh tokens change at each use, and can be taken back
(RFC 7009). The kernel is an OpenID Connect provider for other services too.

With no cookies, the browser sends no credentials by itself, so cross-site
request forgery is not possible. `<img>`, `<video>` and download links cannot
send an `Authorization` header, so the kernel gives short-lived signed URLs
for them.

## What the gateway does

- It listens on this machine only (127.0.0.1 and ::1), on a fixed port that
  people can change. Other devices come through libp2p, not through this
  port.
- It takes only known host names (`fos.localhost`, `*.localhost`), so that DNS
  rebinding does not work.
- It sends each request to the component of its host name.
- It takes away `Set-Cookie` from the answers of apps, and never sends a
  `Cookie` header to an app.
- It puts in safe default headers: `Content-Security-Policy`,
  `X-Content-Type-Options: nosniff`, `Cross-Origin-Opener-Policy`,
  `Cross-Origin-Resource-Policy` and `Referrer-Policy`. An app may make them
  stricter. If an app may make them less strict is a policy of the engine.

The HTTP API keeps to RFC 9110: `ETag` and `If-Match` for versions, `Range`
for large files, and `application/problem+json` (RFC 9457) for errors.

## Where the browser and the kernel are

| Where                                | The app is at                       | People configure |
| ------------------------------------ | ----------------------------------- | ---------------- |
| A device with a node (enhanced mode) | `<app-id>.localhost`                | Nothing          |
| Another device of yours with a node  | The same; the local node carries it | Nothing          |
| A device with no node                | `<app-id>.<node>.<shared domain>`   | Nothing          |
| Your own domain, if you want it      | `<app-id>.<your domain>`            | One DNS record   |

### Enhanced mode: a node on the device

The node gives the web interface itself, from files that are in the `fos`
program. The code comes from the program you put on your machine, not from
a server each time. Apps on other nodes come through the local node over
libp2p. The web page of the project only looks for a local node, and sends
people to it.

### A device with no node: the shared domain of the project

We take the way of Sandstorm's sandcats.io. The project has one shared domain.
Each node gets `*.<node>.<shared domain>`:

- The node proves the name is its own with its node key, to the DNS service of
  the project, and gets a certificate for the name with ACME (DNS-01). The
  secret key of the certificate stays on the node.
- Relays send the connection on by its TLS server name (SNI). They do not see
  inside: TLS ends on your node.
- The shared domain is in the Public Suffix List, so each node is a site of
  its own in the browser.

As with the rendezvous service, this is a default with no special power.
Anyone can run their own, and people with their own domain can use it.

### Basic mode: a node in the browser

A static web page, which anyone can put on a server, runs a light node in the
browser: the portable crates as WebAssembly, storage in OPFS, and the network
over WebSocket through a relay. It is a way in, not the home of your data:

- The code comes from a server each time, so you trust the server. The
  project makes the build the same every time and gives its hashes, and anyone
  can put up their own copy.
- A browser may delete the data of a site (Safari, after 7 days with no use,
  unless the page is installed as an app). A node in the browser is never the
  only copy.
- It is offline when the tab is closed, so it is not an ordering node, a
  rendezvous node or a relay.

## Plan

1. **The gateway:** a fixed port, only this machine, only known host names,
   safe default headers.
2. **Apps as HTTP servers:** `wasi:http` in the runtime, and requests sent by
   host name.
3. **Sign-in:** `/authorize`, `/token` and `/revoke`, passkeys, DPoP, the
   `Fos-User` header, and signed URLs.
4. **The web interface in `fos`**, and the web page of the project that sends
   people to their local node.
5. **The shared domain** for devices with no node.
6. **The node in the browser.**

## Projects we learn from

- **Sandstorm**: an origin for each app, and sandcats.io, made because
  wildcard DNS and certificates were the hardest part for people who run
  their own server.
- **Tailscale Serve** and **oauth2-proxy**: the proxy signs people in and
  gives their identity to the app in a header.
- **Figma plugins** and **VS Code webviews**: what an app may and may not do in
  a browser page.
