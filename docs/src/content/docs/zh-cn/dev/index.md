---
title: 开发者手册
description: 写给开发 homeserver 或基于它开发的人。
---

本手册写给两类开发者：开发 homeserver 本身的人，以及编写与 homeserver
通信的应用、脚本和服务的人。

## 基于 homeserver 开发

节点能做的一切都可以通过 HTTP 访问，所有客户端使用同一套权限模型。

- [REST API](api/)：有类型的 REST API、它的 OpenAPI 文档和 Rust 客户端。
- 标准协议，供已经支持它们的应用使用：CardDAV 和 CalDAV、Bitwarden API、
  Model Context Protocol、OpenID Connect 和 forward auth、Web Push，以及
  S3 的一个子集。见[应用与助手](../user/apps/)和
  [为其他应用提供登录](../admin/sign-in/)。
- 插件（WebAssembly 组件）已有设计，但不在 0.1.0 中。

## 开发 homeserver

- [参与贡献](contributing/)：环境搭建、修改必须遵守的规则，以及如何提交。
- [架构](architecture/)：homeserver 怎样构建，选了哪些技术，以及为什么。
- [值得学习的项目](prior-art/)：我们研究的项目，以及从每个项目中学什么。

原则优先。[宣言](https://github.com/nmnmcc/homeserver/blob/main/MANIFESTO.zh-CN.md)按优先级列出了这些原则，每项修改都以它们为准。
