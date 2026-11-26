# fileOS

[English](README.md) | **简体中文**

fileOS 是一个面向个人及其所属小群体的复制文件系统。系统状态由若干节点共同维护，每个节点只保存它明确同意保存的文件夹。系统不依赖全局共识，也没有全局的内容 DHT。

> [!NOTE]
> 原型阶段，格式还会改变。

## 系统模型

**主体。** 人、设备、节点和 App 都以 ed25519 公钥标识。

**文件夹。** 文件夹是复制、排序和授权的基本单位。它的状态完全由一条签名的、只能追加的日志决定，它的 ID 是日志中第一条记录的哈希。记录分为 `put`、`delete`、`append`、`mount` 和 `policy`。文件内容是按 BLAKE3 哈希寻址的 blob。文件夹可以在路径上挂载其他文件夹，由此构成一棵命名树。

**排序。** 每个文件夹有唯一的排序节点，它给出的全序就是这个文件夹的主线。排序职责可以移交给其他节点。对文件的写入取代先前的版本，旧版本保留在历史中。流上的追加可以交换次序，因此不会冲突。

**授权。** 内核验证事实：签名和哈希。决定由每个节点的引擎作出，引擎由节点主人选择。由于各节点独立作出决定，策略不同时副本可能分叉。系统不强制一致。

**分层。** 内核包括文件系统、基于 libp2p 的同步和 App 运行时（wasmtime）。App 是通过 WIT 接口使用文件系统的 WebAssembly 组件。每个 App 运行在沙箱中，有自己的密钥，只能访问交给它的路径。内核插件是承担一部分内核工作的 WebAssembly 组件，目前唯一的一个是引擎。

## 试一试

使用 devenv shell（`direnv allow`）：

```sh
cargo build -p fos                                   # 程序
apps                                                 # 引擎和 App，放在 target/apps/
fos init --engine target/apps/Capability.app         # 在 ~/.fileos 创建节点
fos run                                              # 网络、App 和网关
```

在另一个终端里：

```sh
fos new photos /photos                               # 创建文件夹，挂载到 /photos
fos install target/apps/Photos.app                   # 安装照片 App
fos put /photos/cat.png cat.png
fos kv photos                                        # 照片 App 找到的信息
fos status
```

把 `/photos` 分享给另一个节点：在这里运行 `fos grant /photos <对方节点密钥> all`，在对方那里运行 `fos mount <文件夹 ID> /photos`。

## 已有功能

- [x] **文件夹**
  - [x] 每个文件夹一条签名日志：文件及其所有版本、流、挂载
  - [x] 文件内容是按哈希命名的 blob
  - [x] 每个文件夹一个排序节点，可以把这项工作交给另一个节点
  - [x] 节点的规则不接受某条记录时，副本会分叉，并说明原因
- [x] **内核插件：访问规则（引擎）**
  - [x] 每个节点一个引擎，由节点主人选择，不加限制
  - [x] 默认引擎：capability，转授时只能变小，收回时连同由它而来的一起收回
- [ ] **App**
  - [x] App 是带 manifest 的 `.app` 文件夹，像其他文件夹一样同步
  - [x] 每个 App 有自己的密钥，只得到 manifest 申请的权限，运行时有 fuel 和内存限制
  - [x] 照片 App：把图片尺寸写入节点上的索引
  - [ ] 响应 HTTP 的 App（`handle-http`）、按文件类型的扩展、网页界面
  - [ ] 通过 WebDAV 访问文件、给 Bitwarden 客户端用的密码库、聊天、通讯录和日历
- [ ] **网络**
  - [x] libp2p：TCP、Noise、Yamux、mDNS、中继、打洞、AutoNAT
  - [x] 写入会等待排序节点，它重新上线后再发出
  - [ ] QUIC
  - [ ] 用 MLS（RFC 9420）加密文件夹
- [ ] **身份**
  - [x] 一个人一个身份，与节点的密钥分开
  - [x] 由身份签名的节点证书，保存在身份文件夹中
  - [ ] 通过会合节点找到一个身份的所有节点
- [x] **运行节点**
  - [x] 一个程序（`fos`）加一个数据目录
  - [x] 本地网关和命令行

## 文档

[用户手册](docs/src/content/docs/zh-cn/user/index.md) · [运行节点](docs/src/content/docs/zh-cn/admin/index.md) · [架构](docs/src/content/docs/zh-cn/dev/fileos.md) · [编写 App 和引擎](docs/src/content/docs/zh-cn/dev/apps.md) · [参与贡献](CONTRIBUTING.zh-CN.md)

## 许可证

[AGPL-3.0-only](LICENSE)，附[插件例外](PLUGIN-EXCEPTION.zh-CN.md)。App SDK（`apps/fos-app`）和 `wit/` 里的接口采用 MIT OR Apache-2.0。
