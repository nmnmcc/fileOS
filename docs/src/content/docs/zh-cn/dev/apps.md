---
title: 编写 App
description: 接口、SDK、.app 文件和 manifest。
---

:::note[接下来]
本页是现在的 App 接口。下一步：授权在运行时进行，App 看不到真实路径（见[模型](../access/)）；App 变成被 `fetch`、`watch` 和 `schedule` 事件唤醒的 Worker（见 [App 作为 Worker](../workers/)）。接口只会增加新东西：今天写的 App 会继续工作。
:::

App 是 WebAssembly 组件。内核用 [wasmtime](https://wasmtime.dev) 运行它们。它们只通过 [`wit/fos.wit`](https://github.com/nmnmcc/fileOS/blob/main/wit/fos.wit) 里的接口和内核通信。

## App

App 是 `app` world 的组件。

| 内核提供（导入）         | 作用                                                         |
| ------------------------ | ------------------------------------------------------------ |
| `fs`                     | 在节点路径上 `read`、`write`、`append`、`delete`、`list-dir` |
| `kv`                     | 只在本节点的缓存，用来放可以重新生成的索引                   |
| `log`                    | 写入节点状态和日志的消息                                     |
| `wasi:clocks/wall-clock` | 时间，用于按时间排序的名字                                   |
| `access`                 | `ask`：通过选择器请用户给一个文件或文件夹                    |

| App 提供（导出）                    | 内核什么时候调用                        |
| ----------------------------------- | --------------------------------------- |
| `events.on-change`                  | App 监听的路径发生变化时                |
| `wasi:http/incoming-handler.handle` | 有发给这个 App 的 origin 的 HTTP 请求时 |

每次调用 `fs` 都使用 **App 自己的密钥**，这个密钥由节点从自己的密钥派生出来。每次调用 App 都有 fuel 和内存的限制。

**路径在 App 的视图中**，而不是节点上的路径：

- manifest 中的每项权限，都挂载在视图中 manifest 给出的路径上：照片 App 申请 `/photos`，看到的就是 `/photos`。
- 用户可以在运行时授予更多（`fos app grant`）；它出现在 `/granted/<handle>/`，或者用户选的名字下。用户也可以让一个名字指向别的地方：App 的 `/photos` 可以是挂载在 `/pictures` 的空间。App 永远看不到一个挂载的真实位置。
- 其他所有路径都在 App 的**私有数据**中：一个由 App 密钥拥有的空间，没有授权时，别的 App 和任何人都读不到。
- `on-change` 给出的也是视图中的路径。

内核按 App 密钥在路径所代表的空间中的授权检查每一次调用。

App 之间只通过文件协作：一个 App 写，另一个 App 监听。

数据放在文件里，记录放在小文件里：聊天是一个目录，每条消息一个文件，文件名按时间排序且永不重复（chat App 用时间加哈希）。这样两个人可以在不同节点上同时写，什么都不会丢。两个节点同时修改同一个文件时，两个版本都会保留；`read` 给出最新的那个。见 [App 数据的最佳实践](../app-data/)。

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
kind = "app"
version = "0.1.0"
build = 1
requires = "fos:app@0.1.0"
main = "bin/app.wasm"

[[permissions]]           # 安装时授予 App 的密钥
path = "/photos"
access = "read"           # 模板（read、read-write、manage）或具体权限，例如 "files.read,files.watch"
reason = "reason.photos"  # resources/locales/ 里的键

[[watch]]                 # on-change 会通知的路径
path = "/photos"
filter = ["*.jpg", "*.png"] # 可选：要通知的名字
kinds = ["made", "changed", "deleted"] # 可选
recursive = true          # 可选：也包括它下面的路径（默认）

[[schedule]]              # 内核每天发送一次 POST /.well-known/fos/schedule/reindex，
name = "reindex"          # 带 fos-event: schedule 头
every = "1d"
```

要把 App 交给别人，就把文件夹打包成一个 `.app` 文件：`fos app pack Photos.app/ out/Photos.app`。这不需要正在运行的节点。`.app` 文件是一个本身不压缩的 zip：

| 条目              | 是什么                                      |
| ----------------- | ------------------------------------------- |
| `manifest.toml`   | 元数据，原样存放，工具不用解压就能读        |
| `payload.tar.zst` | 其余所有文件，放在一个 tar 里，用 zstd 压缩 |

同一个文件夹总是打包出同样的文件：tar 里不记录时间和所有者。`fos app install` 接受 `.app` 文件；在编写 App 的时候，也接受 `.app` 文件夹。

App 安装后，它的文件会放进一个挂载在 `/apps/<name>.app` 的新空间，它的密钥得到 manifest 所申请的授权。

## 构建

把 crate 放在 `apps/` 下，在它的 `Cargo.toml` 旁边放一个 `<Name>.app/` 文件夹。然后运行 `apps`（`scripts/apps.py`）。它会把 `apps/` 里的每个 crate 编译成 `wasm32-unknown-unknown`，用 `wasm-tools component new` 生成组件，再把 `.app` 文件夹放到 `target/apps/`。`fos app pack` 可以把它打包成 `.app` 文件。
