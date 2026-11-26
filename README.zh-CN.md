# fileOS

[English](README.md) | **简体中文**

fileOS 是一个面向个人及其所属小群体的复制文件系统。系统状态由若干节点共同维护，每个节点只保存交给它、并且它同意保存的空间。系统不依赖全局共识，没有排序节点，也没有全局的内容 DHT。

> [!NOTE]
> 原型阶段，格式还会改变。

## 系统模型

**主体。** 设备、节点和 App 以 ed25519 公钥标识。组是没有密钥的主体，由成员代表它行事，它的所有者是一个密钥或另一个组。一个人是一个身份（`did:fos`），拥有若干节点。

**空间。** 空间是复制、所有权和保留的基本单位，它的 ID 是第一条记录的哈希。每个写入者在空间中有一条签名的、只能追加的日志；每条记录写明它在哪些记录之后，因此所有日志构成一个因果图。状态只由记录的集合决定，所以拥有相同记录的副本处于相同状态（强最终一致性），每个节点离线时也能写入。

**文件。** 空间内的名字是扁平的，目录只是前缀。文件有固定的 ID，即创建它的记录的哈希加上动作的序号。改名是一个新版本。文件的并发版本并排保留，不会在用户不知情时丢弃任何一个。内容在由内容决定的位置切块，按 BLAKE3 哈希寻址。

**授权。** 一项授权把具体的权限（允许或拒绝）作用于一个名字前缀，或按 ID 作用于一个文件，授予一个主体。转授只能相同或更小，收回一项授权时，由它转授出去的也一起收回。有效性由每条记录的因果过去决定，并采用强移除：与作者访问被移除同时发生的动作没有效果。不使用时钟。

**检查点。** 所有者对检查点之前的状态的哈希签名。与检查点并发的记录不被接受，由其作者在之上重新写入。节点压缩到检查点，从签名的快照开始，并按保留策略让旧版本变稀疏，因此存储不会无限增长。

**分层。** 内核包括空间、基于 libp2p 的同步、App 运行时（wasmtime）和本地 HTTP 网关。App 是通过 WIT 接口使用文件系统的 WebAssembly 组件。每个 App 运行在沙箱中，有自己的密钥，按它的授权受到检查。内核可以有承担一部分内核工作的插件，目前没有。

## 试一试

使用 devenv shell（`direnv allow`）：

```sh
cargo build -p fos                       # 程序
apps                                     # App，放在 target/apps/
fos node init                            # 在 ~/.fileos 创建节点
fos node run                             # 网络、App 和网关
```

在另一个终端里：

```sh
fos space new /photos                    # 创建空间，挂载到 /photos
fos app install target/apps/Photos.app   # 安装照片 App
fos fs put /photos/cat.png cat.png
fos fs ls /photos
fos node status
```

把 `/photos` 分享给另一个节点：在这里运行 `fos space grant /photos <对方节点密钥> read-write`，在对方那里运行 `fos space mount <空间 ID> /photos`。

## 已有功能

- [x] **空间**
  - [x] 每个写入者一条签名日志，构成一个因果图；没有排序节点；离线也能写入
  - [x] ID 固定的文件，并排保留的版本，改名后 ID 不变
  - [x] 按内容切分、按哈希命名的块
  - [x] 检查点、压缩、给新节点的快照、随时间变稀疏的保留策略
- [x] **访问**
  - [x] 授权：具体权限或模板，作用于前缀或文件，允许或拒绝
  - [x] 只能缩小转授；收回时连同由它而来的一起收回；强移除
  - [x] 组，由密钥或组拥有，组也可以放进组里
  - [ ] 由管理这些文件的 App 做写入检查；App 的选择器
- [ ] **App**
  - [x] App 以 `.app` 文件提供（本身不压缩的 zip：manifest，加上 `tar.zst` 里的其余文件）
  - [x] 每个 App 有自己的密钥，只得到 manifest 申请的权限，运行时有 fuel 和内存限制
  - [x] App 通过 `wasi:http` 在自己的 origin（`<app-id>.localhost`）上响应 HTTP
  - [x] 照片 App：图库、上传，以及图片尺寸的索引
  - [x] 聊天 App：聊天室中每条消息一个文件，放在你所分享的空间里
  - [ ] App 作为 Worker、`wasi:filesystem`、监听选项、定时任务
- [ ] **网络和密钥**
  - [x] libp2p：TCP、Noise、Yamux、mDNS、中继、打洞、AutoNAT
  - [ ] 加密：跟着前缀的密钥、HPKE（RFC 9180）、删除时销毁密钥
  - [ ] QUIC
- [ ] **身份**
  - [x] 一个人一个身份：一个 DID（`did:fos`，即没有目录服务器的 `did:plc` 方法），与节点的密钥分开
  - [x] 轮换密钥和签名密钥；节点证书保存在身份空间中
  - [ ] 一个身份代表它的所有设备
  - [ ] 用域名作为身份的另一个名字（和 AT Protocol 的 handle 一样）
  - [ ] 通过会合节点找到一个身份的所有节点
- [x] **运行节点**
  - [x] 一个程序（`fos`）加一个数据目录
  - [x] 本地网关和命令行

## 文档

[用户手册](docs/src/content/docs/zh-cn/user/index.md) · [运行节点](docs/src/content/docs/zh-cn/admin/index.md) · [架构](docs/src/content/docs/zh-cn/dev/fileos.md) · [编写 App](docs/src/content/docs/zh-cn/dev/apps.md) · [参与贡献](CONTRIBUTING.zh-CN.md)

## 许可证

[AGPL-3.0-only](LICENSE)，附[插件例外](PLUGIN-EXCEPTION.zh-CN.md)。App SDK（`apps/fos-app`）和 `wit/` 里的接口采用 MIT OR Apache-2.0。
