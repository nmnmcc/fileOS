# homeserver

[English](README.md) | **简体中文**

一台属于你自己的个人服务器，保存你的笔记、文件、照片、联系人、日历和密码，并与你认识的人的 homeserver 直接分享。

**原理：** 每次修改都是追加到日志末尾的一条签名记录，文件是按哈希命名的 blob；搜索、同步、分享和备份都由在节点之间复制这些日志和 blob 得到。

> [!NOTE]
> 实验阶段。0.1.0 是最小可用版本，数据格式仍可能变化，请保留导出。

## 功能

**你的数据**

- [x] 笔记、文件、照片、联系人、日程等条目，按空间存放
- [x] 搜索、时间线、地点和条目之间的链接
- [x] 照片元数据（时间、地点、相机）和缩略图
- [ ] 人物、日期、地点和事情的页面
- [ ] 聊天
- [ ] 协作文档
- [ ] 导入工具：Google Takeout、iCloud、聊天记录导出、密码管理器
- [ ] 网页应用

**身份与授权**

- [x] 身份是你自己的密钥（`did:hs`），附带恢复码
- [x] 恢复码可以拆分交给朋友保管（Shamir 秘密共享）
- [x] 每个应用和每个人一个令牌，按角色、空间和类型限制，随时可以收回
- [x] OpenID Connect 身份提供方、通行密钥，以及为其他自托管应用提供的转发认证

**homeserver 之间的分享**

- [x] 与其他 homeserver 上的人共享空间，实时更新
- [ ] 端到端加密空间（MLS）
- [ ] 为信任的人提供中继和信箱
- [ ] QUIC 和 HTTP/3，以及自己实现的 NAT 穿透

**兼容现有应用**

- [x] REST API，支持 Server-Sent Events，附带 OpenAPI 文档
- [x] CardDAV 和 CalDAV
- [x] 兼容 Bitwarden 的密码库
- [x] 供 AI 助手使用的 Model Context Protocol（MCP）
- [x] Web Push
- [ ] 文件的 WebDAV
- [ ] WebAssembly 插件

**备份与运行节点**

- [x] 一个程序（`hs`）加一个数据目录，无需配置文件
- [x] 导出为普通 tar 文件
- [x] 加密副本存到目录、S3 或朋友的 homeserver，每小时同步
- [x] 只用恢复码就能恢复节点
- [x] 家用路由器端口映射（NAT-PMP/PCP）
- [ ] 签名的自动升级，失败时回到旧版本
- [ ] 一个身份跨多台自己的节点（家里、VPS、笔记本）
- [ ] 个人 AI 记忆，以及用一句话描述生成的小工具

## 文档

[用户手册](docs/src/content/docs/zh-cn/user/index.md) · [管理手册](docs/src/content/docs/zh-cn/admin/index.md) · [开发者手册](docs/src/content/docs/zh-cn/dev/index.md) · [宣言](MANIFESTO.zh-CN.md) · [更新日志](CHANGELOG.md) · [参与贡献](CONTRIBUTING.zh-CN.md)

## 许可证

[AGPL-3.0-only](LICENSE)，附 [插件例外](PLUGIN-EXCEPTION.zh-CN.md)。插件开发包 `hs-plugin-api` 采用 MIT OR Apache-2.0。本软件部分基于 Independent JPEG Group 的工作。
