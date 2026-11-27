---
title: Writing apps
description: The interfaces, the SDK, .app files and the manifest.
---

:::note[What comes next]
This page is the app interface of today. Next, grants come at run time and
apps do not see real paths (see [The model](../access/)), and apps become
workers that wake up for `fetch`, `watch` and `schedule` events (see
[Apps as workers](../workers/)). The interface only gets new things: apps
made today keep working.
:::

Apps are WebAssembly components. The kernel runs them with
[wasmtime](https://wasmtime.dev). They talk to the core only through the
interfaces in [`wit/fileos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fileos.wit).

## Apps

An app is a component of the `app` world.

| The core gives (imports) | What it does                                                   |
| ------------------------ | -------------------------------------------------------------- |
| `fs`                     | `read`, `write`, `append`, `delete`, `list-dir`, on node paths |
| `kv`                     | A cache on this node only, for indexes that can be made again  |
| `log`                    | Messages for the node's status and log                         |
| `wasi:clocks/wall-clock` | The time, for names that sort by time                          |

| The app gives (exports)             | When the core calls it                       |
| ----------------------------------- | -------------------------------------------- |
| `events.on-change`                  | A path that the app watches has changed      |
| `wasi:http/incoming-handler.handle` | An HTTP request for the app's origin came in |

Every call to `fs` is made with the **app's own key**, which the node makes
from its own key. Each call into an app has limits on fuel and memory.

**Paths are in the app's view**, not on the node:

- Each permission in the manifest is mounted in the view at the path the
  manifest gives: the photos app asks for `/photos`, and sees `/photos`.
- The person can give more at run time (`fileos app grant`); it comes at
  `/granted/<handle>/`, or at a name the person picks. The person can also
  point a name at another place: the app's `/photos` can be the space at
  `/pictures`. The app never sees where a mount really is.
- Every other path is in the app's **private data**: a space that the app's
  key owns, which no other app and no person can read unless given.
- `on-change` gives paths in the view too.

The kernel checks every call against the grants of the app's key in the
space that the path stands for.

Apps work together only through files: one app writes, another watches.

Keep data in files, and keep records in small files: a chat is a directory
with one file for each message, named so that the names sort by time and
never meet (the chat app uses the time and a hash). Two people can then write
at the same time on different nodes, and nothing is lost. When two nodes
change the same file at the same time, both versions are kept; `read` gives
the newest. See [Data in apps](../app-data/).

A small app, with the Rust SDK in `apps/fileos-app`:

```rust
use fileos_app::app::exports::fileos::app::events::Guest;
use fileos_app::app::fileos::app::{fs, kv};

struct Sizes;

impl Guest for Sizes {
    fn on_change(path: String) {
        if let Ok(bytes) = fs::read(&path) {
            kv::set(&format!("size:{path}"), bytes.len().to_string().as_bytes());
        }
    }
}

fileos_app::app::export_app!(Sizes);
```

### HTTP

Every app is an HTTP server for its own origin,
`http://<app-id>.localhost:7341/`, with the standard `wasi:http` (WASI 0.2).
`fileos app install` prints the address. Each request gets a new instance of
the app, with limits on fuel, memory and time (30 seconds). The kernel takes
the `cookie`, `authorization` and `fileos-user` headers away from requests,
puts in `fileos-user` with the DID of the person when someone is signed in, and
takes `set-cookie` away from responses. An app has no files, sockets or
outgoing HTTP of its own: only `fs`, `kv` and `log`.

The SDK reads the request and writes the response for you:

```rust
use fileos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
use fileos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
use fileos_app::http::Response;

impl Http for Sizes {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        fileos_app::http::serve(request, out, |req| {
            Response::text(200, format!("hello, {}", req.user().unwrap_or("no one")))
        });
    }
}
```

See "HTTP and the browser" in the architecture for the reasons.

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
kind = "app"
version = "0.1.0"
build = 1
requires = "fileos:app@0.1.0"
main = "bin/app.wasm"

[[permissions]]           # given to the app's key when it is installed
path = "/photos"
access = "read"           # a template (read, read-write, manage) or exact
                          # permissions such as "files.read,files.watch"
reason = "reason.photos"  # a key in resources/locales/

[[watch]]                 # the paths that on-change is called for
path = "/photos"
filter = ["*.jpg", "*.png"] # optional: names to hear of
kinds = ["made", "changed", "deleted"] # optional
recursive = true          # optional: also paths under it (the default)

[[schedule]]              # the kernel sends POST /.well-known/fileos/schedule/reindex
name = "reindex"          # with fileos-event: schedule, once a day
every = "1d"
```

To give the app to others, pack the folder into an `.app` file:
`fileos app pack Photos.app/ out/Photos.app`. It needs no running node. An
`.app` file is a zip with no compression in the zip itself:

| Entry             | What it is                                                     |
| ----------------- | -------------------------------------------------------------- |
| `manifest.toml`   | The metadata, as it is, so tools can read it with no unpacking |
| `payload.tar.zst` | Every other file, in a tar, compressed with zstd               |

The same folder always gives the same file: the tar has no times or owners.
`fileos app install` takes an `.app` file, or an `.app` folder while you make
it.

When an app is installed, its files go into a new space mounted at
`/apps/<name>.app`, and its key gets the grants that its manifest asks for.

## Building

Put the crate in `apps/`, with a `<Name>.app/` folder next to its
`Cargo.toml`. Then run `apps` (`scripts/apps.py`). It builds every crate in
`apps/` for `wasm32-unknown-unknown`, makes a component of it with
`wasm-tools component new`, and puts the `.app` folder in `target/apps/`.
`fileos app pack` makes an `.app` file of it.
