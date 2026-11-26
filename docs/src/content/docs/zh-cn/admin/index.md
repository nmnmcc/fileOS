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
apps                           # 引擎和 App：target/apps/*.app
```

发布包里也是同样的文件：`fos` 程序和 `apps/` 文件夹。

## 创建节点

```sh
fos node init --engine target/apps/Capability.app
```

这会创建数据目录（默认是 `~/.fileos`，可以用 `--dir` 或 `FOS_DIR` 修改）、节点密钥和根文件夹。引擎就是节点的规则。`Capability.app` 是默认引擎：文件夹的创建者可以做任何事，并决定别人可以做什么。

## 运行

```sh
fos node run
```

这会启动网络、App 和网关。网关是 `127.0.0.1` 上的一个小 HTTP API，供其他 `fos` 命令使用，需要数据目录里 `gateway.token` 中的令牌。

| 选项             | 作用                                             |
| ---------------- | ------------------------------------------------ |
| `--listen ADDR`  | 监听的 libp2p 地址（默认：在所有地址上监听 TCP） |
| `--peer ADDR`    | 启动时连接的节点                                 |
| `--gateway ADDR` | 网关的监听地址（默认 `127.0.0.1`，任意空闲端口） |
| `--no-mdns`      | 不在家庭网络中寻找节点                           |

`fos node status` 显示节点密钥、监听地址（带 `/p2p/…`，可以直接给另一个节点）、对端、文件夹和 App。

## 连接两个节点

在家庭网络中，节点通过 mDNS 互相发现。在其他地方，把一个节点的地址告诉另一个节点：

```sh
fos node connect /ip4/192.0.2.10/tcp/4001/p2p/12D3KooW…
```

网络使用 [libp2p](https://libp2p.io)：TCP 加 Noise 和 Yamux、中继和打洞。没有 DHT：节点只和你让它连接的节点通信，也只把文件夹发给它的规则允许的节点。

## 数据目录

```text
node.key          节点密钥（请保密）
root              根文件夹的 ID
engine/           本节点的引擎
blobs/            文件内容，按哈希命名
folders/*.log     每个文件夹的签名日志
local/*.kv        每个 App 只保存在本节点上的数据
gateway.token     网关令牌
```

在节点停止时复制这个目录，就是一份完整的备份。节点启动时会重新读取每条日志，并用引擎检查每一条记录。

## 出问题时

- **文件夹没有同步过来。** 确认节点已经连接（`fos node status` 会列出对端），并且文件夹的主人给了你的节点密钥 `store` 权限。
- **写入在等待。** `fos node status` 会显示有多少写入在等待排序节点。那个节点重新连上后，它们就会发出去。
- **`fos node status` 里有 `FORK`。** 你的引擎没有接受排序节点已经接受的一条记录。状态里会写明是哪一条、为什么。
- **需要更多信息。** 用 `RUST_LOG=debug fos node run` 运行节点。
