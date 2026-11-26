---
title: Writing apps and engines
description: The interfaces, the SDK, .app files and the manifest.
---

:::caution[The design is changing]
This page is about the prototype. In the new design there is no
authorization engine, no `kv` and no streams (everything is a file), grants
come at run time and apps do not see real paths (see
[Access](../access/)), and apps are workers that wake up for `fetch` and
`watch` events (see [Apps as workers](../workers/)).
:::

Apps and engines are WebAssembly components. The core runs them with
[wasmtime](https://wasmtime.dev). They talk to the core only through the
interfaces in [`wit/fos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fos.wit).

## Apps

An app is a component of the `app` world.

| The core gives (imports) | What it does                                                   |
| ------------------------ | -------------------------------------------------------------- |
| `fs`                     | `read`, `write`, `append`, `delete`, `list-dir`, on node paths |
| `kv`                     | A key-value store on this node only, for indexes               |
| `log`                    | Messages for the node's status and log                         |

| The app gives (exports)             | When the core calls it                       |
| ----------------------------------- | -------------------------------------------- |
| `events.on-change`                  | A path that the app watches has changed      |
| `wasi:http/incoming-handler.handle` | An HTTP request for the app's origin came in |

Every call to `fs` is made with the **app's own key**, which the node makes
from its own key. The engine checks every call. An app can only use the paths
that the user gave it when it was installed. Each call into an app has limits
on fuel and memory.

Apps work together only through files: one app writes, another watches.

A small app, with the Rust SDK in `apps/fos-app`:

```rust
use fos_app::app::exports::fos::app::events::Guest;
use fos_app::app::fos::app::{fs, kv};

struct Sizes;

impl Guest for Sizes {
    fn on_change(path: String) {
        if let Ok(bytes) = fs::read(&path) {
            kv::set(&format!("size:{path}"), bytes.len().to_string().as_bytes());
        }
    }
}

fos_app::app::export_app!(Sizes);
```

### HTTP

Every app is an HTTP server for its own origin,
`http://<app-id>.localhost:7341/`, with the standard `wasi:http` (WASI 0.2).
`fos app install` prints the address. Each request gets a new instance of
the app, with limits on fuel, memory and time (30 seconds). The kernel takes
the `cookie`, `authorization` and `fos-user` headers away from requests,
puts in `fos-user` with the DID of the person when someone is signed in, and
takes `set-cookie` away from responses. An app has no files, sockets or
outgoing HTTP of its own: only `fs`, `kv` and `log`.

The SDK reads the request and writes the response for you:

```rust
use fos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
use fos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
use fos_app::http::Response;

impl Http for Sizes {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        fos_app::http::serve(request, out, |req| {
            Response::text(200, format!("hello, {}", req.user().unwrap_or("no one")))
        });
    }
}
```

See "HTTP and the browser" in the architecture for the reasons.

## Engines

An engine is a component of the `engine` world. It exports `authz`. Each node
runs one engine, which its owner picks. The core keeps the state of the engine
for each folder as bytes, and gives no meaning to them.

| Function  | What the core asks                                             |
| --------- | -------------------------------------------------------------- |
| `init`    | The state of a new folder                                      |
| `apply`   | Read one policy entry; give the new state, or say why not      |
| `check`   | May this key do this (read, write, append, grant, store) here? |
| `readers` | The keys that may read the folder                              |
| `storers` | The keys of the nodes that may keep a copy                     |
| `grant`   | The policy entry that gives a key some abilities on a path     |

With `grant`, the core can give access to an app or a person without knowing
the format of policy entries. The default engine, `apps/capability`, uses
lines of text such as `grant <key> read,write /photos`.

The core puts no limits on the engine. But two nodes with different engines
can come to different answers about the same entry. Then their copies of a
folder fork. See [Architecture](../fileos/#a-shared-folder).

## .app files

You make an app as an `.app` folder:

```text
Photos.app/
  manifest.toml        what the app is and what it asks for
  bin/app.wasm         the component
  resources/locales/   text for people: en-us.toml, zh-cn.toml
```

```toml
id = "photos"
name = "Photos"
kind = "app"              # or "engine"
version = "0.1.0"
build = 1
requires = "fos:app@0.1.0"
main = "bin/app.wasm"

[[permissions]]           # given to the app's key when it is installed
path = "/photos"
access = "read"           # read, write, append or read-write
reason = "reason.photos"  # a key in resources/locales/

[[watch]]                 # the paths that on-change is called for
path = "/photos"
```

To give the app to others, pack the folder into an `.app` file:
`fos app pack Photos.app/ out/Photos.app`. It needs no running node. An
`.app` file is a zip with no compression in the zip itself:

| Entry             | What it is                                                     |
| ----------------- | -------------------------------------------------------------- |
| `manifest.toml`   | The metadata, as it is, so tools can read it with no unpacking |
| `payload.tar.zst` | Every other file, in a tar, compressed with zstd               |

The same folder always gives the same file: the tar has no times or owners.
`fos node init --engine` and `fos app install` take an `.app` file, or an
`.app` folder while you make it.

When an app is installed, its files become a fileOS folder at
`/apps/<name>.app`, so it syncs to your other nodes like any other folder.

## Building

Put the crate in `apps/`, with a `<Name>.app/` folder next to its
`Cargo.toml`. Then run `apps` (`scripts/apps.py`). It builds every crate in
`apps/` for `wasm32-unknown-unknown`, makes a component of it with
`wasm-tools component new`, and puts the `.app` folder in `target/apps/`.
`fos app pack` makes an `.app` file of it.
