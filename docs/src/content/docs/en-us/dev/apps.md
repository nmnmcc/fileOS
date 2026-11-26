---
title: Writing apps and engines
description: The interfaces, the SDK, the .app folder and the manifest.
---

Apps and engines are WebAssembly components. The core runs them with
[wasmtime](https://wasmtime.dev). They talk to the core only through the
interfaces in [`wit/fos.wit`](https://github.com/nmnmcc/homeserver/blob/main/wit/fos.wit).

## Apps

An app is a component of the `app` world.

| The core gives (imports) | What it does                                                   |
| ------------------------ | -------------------------------------------------------------- |
| `fs`                     | `read`, `write`, `append`, `delete`, `list-dir`, on node paths |
| `kv`                     | A key-value store on this node only, for indexes               |
| `log`                    | Messages for the node's status and log                         |

| The app gives (exports) | When the core calls it                  |
| ----------------------- | --------------------------------------- |
| `events.on-change`      | A path that the app watches has changed |

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

## The .app folder

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

When an app is installed, its `.app` folder becomes a fileOS folder at
`/apps/<name>.app`, so it syncs to your other nodes like any other folder.

## Building

Put the crate in `apps/`, with a `<Name>.app/` folder next to its
`Cargo.toml`. Then run `apps` (`scripts/apps.py`). It builds every crate in
`apps/` for `wasm32-unknown-unknown`, makes a component of it with
`wasm-tools component new`, and puts the `.app` folder in `target/apps/`.
