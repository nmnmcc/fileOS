---
title: 运行节点
description: 构建 fileOS，创建并运行一个节点，再把它连接到其他节点。
---

:::caution[原型]
fileOS 还是原型。数据格式在不同版本之间还会改变。
:::

## 构建

在仓库中，使用 devenv shell（里面有所有工具）：

```sh
cargo build --release -p fos   # 程序：target/release/fos
apps                           # App：target/apps/*.app
```

发布包里也是同样的文件：`fos` 程序和 `apps/` 文件夹。

## 创建节点

```sh
fos node init
```

这会创建数据目录（默认是 `~/.fileos`，可以用 `--dir` 或 `FOS_DIR` 修改）、节点密钥和它的家空间。其他空间挂载在家空间中，例如 `/photos`。

## 运行

```sh
fos node run
```

这会启动网络、App 和网关。网关是节点的 HTTP 前端，只在本机（`127.0.0.1` 和 `::1`，端口 7341）上监听：`http://fos.localhost:7341/` 是内核，`http://<app-id>.localhost:7341/` 是某个 App。其他 `fos` 命令使用内核 API，需要数据目录里 `gateway.token` 中的令牌。`fos curl` 带着这个令牌运行 curl，例如 `fos curl /v0/status`。

| 选项            | 作用                                             |
| --------------- | ------------------------------------------------ |
| `--listen ADDR` | 监听的 libp2p 地址（默认：在所有地址上监听 TCP） |
| `--peer ADDR`   | 启动时连接的节点                                 |
| `--port PORT`   | 网关的端口（默认 7341；0 表示任意空闲端口）      |
| `--no-mdns`     | 不在家庭网络中寻找节点                           |

`fos node status` 显示节点密钥、监听地址（带 `/p2p/…`，可以直接给另一个节点）、对端和空间。

## 连接两个节点

在家庭网络中，节点通过 mDNS 互相发现。在其他地方，把一个节点的地址告诉另一个节点：

```sh
fos node connect /ip4/192.0.2.10/tcp/4001/p2p/12D3KooW…
```

网络使用 [libp2p](https://libp2p.io)：TCP 加 Noise 和 Yamux、中继和打洞。没有 DHT：节点只和你让它连接的节点通信，也只把空间发给有保存授权的节点（对整个空间有 `data.keep` 或 `files.read`）。

## 数据目录

```text
node.key          节点密钥（请保密）
identity-*.key    身份的密钥，只在持有它们的节点上
home              家空间的 ID
blobs/            内容：块和块列表，按哈希命名
spaces/*.log      每个空间的签名记录
spaces/*.base     压缩后，最近一个检查点的快照
local/*.kv        每个 App 只保存在本节点上的缓存
gateway.token     网关令牌
```

在节点停止时复制这个目录，就是一份完整的备份。节点启动时会重新读取每条日志，并检查每一条记录。

## 磁盘空间

节点在自己拥有的空间达到 1000 条记录后做一个检查点，然后丢掉它之前的记录。旧版本随时间变稀疏：一天内每小时一个，30 天内每天一个，一年内每周一个。删除的文件保留 30 天。`fos space checkpoint <path>` 可以立即做一个检查点。

## 出问题时

- **空间没有同步过来。** 确认节点已经连接（`fos node status` 会列出对端），并且空间的主人给了你的节点密钥保存它的授权（`fos space grant /photos <key> read`）。
- **写入没有到另一个节点。** 节点任何时候都能写，离线时也能；重新连接后会把写入发出去。
- **`fos node status` 里有 “entries have no effect”。** 有些记录不被允许：例如它们和访问被收回同时发生。`fos space grants <path>` 会列出授权。
- **需要更多信息。** 用 `RUST_LOG=debug fos node run` 运行节点。
