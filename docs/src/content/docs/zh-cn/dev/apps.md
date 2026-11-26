---
title: 编写 App 和引擎
description: 接口、SDK、.app 文件和 manifest。
---

:::caution[设计正在变化]
本页描述的是原型。新设计中没有授权引擎，没有 `kv`，也没有流（一切皆文件）；授权在运行时进行，App 看不到真实路径（见[访问](../access/)）；App 是被 `fetch` 和 `watch` 事件唤醒的 Worker（见 [App 作为 Worker](../workers/)）。
:::

App 和引擎都是 WebAssembly 组件。内核用 [wasmtime](https://wasmtime.dev) 运行它们。它们只通过 [`wit/fos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fos.wit) 里的接口和内核通信。

## App

App 是 `app` world 的组件。

| 内核提供（导入） | 作用                                                         |
| ---------------- | ------------------------------------------------------------ |
| `fs`             | 在节点路径上 `read`、`write`、`append`、`delete`、`list-dir` |
| `kv`             | 只在本节点的键值存储，用来放索引                             |
| `log`            | 写入节点状态和日志的消息                                     |

| App 提供（导出）                    | 内核什么时候调用                        |
| ----------------------------------- | --------------------------------------- |
| `events.on-change`                  | App 监听的路径发生变化时                |
| `wasi:http/incoming-handler.handle` | 有发给这个 App 的 origin 的 HTTP 请求时 |

每次调用 `fs` 都使用 **App 自己的密钥**，这个密钥由节点从自己的密钥派生出来。引擎会检查每一次调用。App 只能使用安装时用户给它的路径。每次调用 App 都有 fuel 和内存的限制。

App 之间只通过文件协作：一个 App 写，另一个 App 监听。

一个小 App，使用 `apps/fos-app` 里的 Rust SDK：

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

每个 App 都用标准的 `wasi:http`（WASI 0.2）为自己的 origin `http://<app-id>.localhost:7341/` 提供 HTTP 服务，`fos app install` 会打印这个地址。每个请求都使用一个新的 App 实例，有 fuel、内存和时间（30 秒）的限制。内核会删掉请求里的 `cookie`、`authorization` 和 `fos-user` 头，有人登录时加上带有这个人 DID 的 `fos-user`，并删掉回应里的 `set-cookie`。App 没有自己的文件、socket 或外发 HTTP，只有 `fs`、`kv` 和 `log`。

SDK 会替你读取请求、写出回应：

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

理由见架构中的 “HTTP 与浏览器”。

## 引擎

引擎是 `engine` world 的组件，导出 `authz`。每个节点运行一个引擎，由节点主人选择。内核为每个文件夹保存引擎的状态（字节），但不解释它的含义。

| 函数      | 内核问什么                                                          |
| --------- | ------------------------------------------------------------------- |
| `init`    | 新文件夹的状态                                                      |
| `apply`   | 读取一条 policy 记录，给出新状态，或者说明为什么不接受              |
| `check`   | 这个密钥能不能在这里做这件事（read、write、append、grant、store）？ |
| `readers` | 能读这个文件夹的密钥                                                |
| `storers` | 能保存副本的节点的密钥                                              |
| `grant`   | 给某个密钥在某个路径上一些能力的 policy 记录                        |

有了 `grant`，内核不需要知道 policy 记录的格式，也能给 App 或别人授权。默认引擎 `apps/capability` 使用文本行，比如 `grant <key> read,write /photos`。

内核对引擎不加任何限制。但使用不同引擎的两个节点，可能对同一条记录得出不同的结论，这时它们的文件夹副本就会分叉。见[架构](../fileos/#共享文件夹)。

## .app 文件

App 以 `.app` 文件夹的形式编写：

```text
Photos.app/
  manifest.toml        App 是什么、申请什么权限
  bin/app.wasm         组件
  resources/locales/   给人看的文字：en-us.toml、zh-cn.toml
```

```toml
id = "photos"
name = "Photos"
kind = "app"              # 或 "engine"
version = "0.1.0"
build = 1
requires = "fos:app@0.1.0"
main = "bin/app.wasm"

[[permissions]]           # 安装时授予 App 的密钥
path = "/photos"
access = "read"           # read、write、append 或 read-write
reason = "reason.photos"  # resources/locales/ 里的键

[[watch]]                 # on-change 会通知的路径
path = "/photos"
```

要把 App 交给别人，就把文件夹打包成一个 `.app` 文件：`fos app pack Photos.app/ out/Photos.app`。这不需要正在运行的节点。`.app` 文件是一个本身不压缩的 zip：

| 条目              | 是什么                                      |
| ----------------- | ------------------------------------------- |
| `manifest.toml`   | 元数据，原样存放，工具不用解压就能读        |
| `payload.tar.zst` | 其余所有文件，放在一个 tar 里，用 zstd 压缩 |

同一个文件夹总是打包出同样的文件：tar 里不记录时间和所有者。`fos node init --engine` 和 `fos app install` 接受 `.app` 文件；在编写 App 的时候，也接受 `.app` 文件夹。

App 安装后，它的文件会变成 `/apps/<name>.app` 上的一个 fileOS 文件夹，所以它会像其他文件夹一样同步到你的其他节点。

## 构建

把 crate 放在 `apps/` 下，在它的 `Cargo.toml` 旁边放一个 `<Name>.app/` 文件夹。然后运行 `apps`（`scripts/apps.py`）。它会把 `apps/` 里的每个 crate 编译成 `wasm32-unknown-unknown`，用 `wasm-tools component new` 生成组件，再把 `.app` 文件夹放到 `target/apps/`。`fos app pack` 可以把它打包成 `.app` 文件。
