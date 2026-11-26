# fileOS

[English](README.md) | **简体中文**

一个由你的电脑和你认识的人的电脑共同保存的文件系统。你在上面用到的一切（照片、聊天、密码）都是 App。

> [!NOTE]
> 原型阶段，格式还会改变。

## 怎样构成

fileOS 分为两部分：**内核**和 **App**。内核支持**内核插件**（Kernel Plugin）。

- **内核**只提供一样东西：在每个节点上含义都相同的文件。文件夹是一条签名的修改日志，文件是按哈希命名的 blob。共享文件夹就是多个节点各自同意保存的文件夹。
- **App** 是用户空间。人们只通过 App 使用 fileOS。App 是一个 WebAssembly 组件，运行在沙箱里，有自己的密钥，只能看到你给它的文件夹。
- **内核插件**改变内核怎样工作，不改变 App 看到什么。第一个内核插件是访问规则（代码里叫 _引擎_）。接下来可能会有存放数据的位置和磁盘上数据的加密。

这一切之上只有一条规则：**不破坏用户空间**（do not break user space）。内核守着两份契约：与 App 的契约（[`wit/fos.wit`](wit/fos.wit) 里的 `app` world），以及与其他节点的契约（格式和协议）。我们只往里面增加，不修改也不删除已有的内容。内核插件只能在契约已有的结果中选择：它可以拒绝一次写入，但不能改变写入是什么。同一个 App，面对同样的数据和同样的规则，在每个节点上、无论装了哪些内核插件，行为都一样。

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

把 `/photos` 分享给另一个节点：在这里运行 `fos grant /photos <对方节点密钥> all`，在对方那里运行 `fos mount <文件夹 ID> /photos`。详见[用户手册](docs/src/content/docs/zh-cn/user/index.md)和[管理手册](docs/src/content/docs/zh-cn/admin/index.md)。

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
- [x] **运行节点**
  - [x] 一个程序（`fos`）加一个数据目录
  - [x] 本地网关和命令行

## 文档

[用户手册](docs/src/content/docs/zh-cn/user/index.md) · [运行节点](docs/src/content/docs/zh-cn/admin/index.md) · [架构](docs/src/content/docs/zh-cn/dev/fileos.md) · [编写 App 和引擎](docs/src/content/docs/zh-cn/dev/apps.md) · [参与贡献](CONTRIBUTING.zh-CN.md)

## 许可证

[AGPL-3.0-only](LICENSE)，附[插件例外](PLUGIN-EXCEPTION.zh-CN.md)：只使用插件接口的 App 和内核插件可以采用任何许可证。App SDK（`apps/fos-app`）和 `wit/` 里的接口采用 MIT OR Apache-2.0。
