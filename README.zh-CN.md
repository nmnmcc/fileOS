# homeserver

[English](README.md) | **简体中文**

> 一台属于你自己的个人服务器，存放你保存和分享的数据。

homeserver 是一个程序（`hs`）加一个数据目录。它保存你的照片、文件、聊天、密码、笔记和日历，按人、时间、地点和事情组织，并与你认识的人的 homeserver 直接相连。不需要服务器运维。

> [!NOTE]
> **当前状态：实验阶段。** 0.1.0 是一个最小可用版本。数据格式仍可能变化，请保留导出。已实现的功能见[更新日志](CHANGELOG.md)。

## 快速开始

```sh
cargo build --release --package hs   # 或：devenv build outputs.hs
hs ctl init --name "Ada"             # 记下它打印的恢复码
hs ctl serve                         # http://127.0.0.1:8080
```

详见[安装](docs/src/content/docs/zh-cn/admin/install.md)和[运行节点](docs/src/content/docs/zh-cn/admin/running.md)。

## 文档

- [用户手册](docs/src/content/docs/zh-cn/user/index.md)：登录、你的数据、分享、应用与助手。
- [管理手册](docs/src/content/docs/zh-cn/admin/index.md)：安装、运行、配置、备份。
- [开发者手册](docs/src/content/docs/zh-cn/dev/index.md)：参与贡献、REST API、[架构](docs/src/content/docs/zh-cn/dev/architecture.md)。
- [宣言](MANIFESTO.zh-CN.md)：为什么做 homeserver、原则、不做什么，以及路线图。

## 参与

欢迎开 issue 讨论原则、质疑设计决定或描述使用场景。开发环境和规则见 [CONTRIBUTING.zh-CN.md](CONTRIBUTING.zh-CN.md)。

## 许可证

homeserver 是自由软件，采用 [GNU Affero 通用公共许可证 3.0 only](LICENSE)（`AGPL-3.0-only`）授权，在适用法律允许的范围内**不提供任何担保（ABSOLUTELY NO WARRANTY）**。只通过插件接口与 homeserver 交互的插件可以使用任何许可证，详见 [homeserver 插件例外](PLUGIN-EXCEPTION.zh-CN.md)。插件开发包 `hs-plugin-api` 采用 MIT OR Apache-2.0 授权。

本软件部分基于 Independent JPEG Group 的工作（通过生成缩略图所用的 `jpeg-encoder`）。
