# homeserver

[English](README.md) | **简体中文**

一台属于你自己的个人服务器，保存你的笔记、文件、照片、联系人、日历和密码，并与你认识的人的 homeserver 直接分享。

**原理：** 每次修改都是追加到日志末尾的一条签名记录，文件是按哈希命名的 blob；搜索、同步、分享和备份都由在节点之间复制这些日志和 blob 得到。

> [!NOTE]
> 实验阶段。0.1.0 是最小可用版本，数据格式仍可能变化，请保留导出。

## 功能

只有下面全部完成时，上一级才会勾选。

- [ ] **你的数据**
  - [x] 空间中的条目：笔记、文件、照片、联系人、日程，以及任何 JSON
    - [x] 添加、修改（JSON merge patch）、列出和读取条目；`hs edit` 在编辑器中打开
    - [x] 每次修改都签名保存，历史不会丢失
    - [x] 删除条目，也可以从所有成员的日志中抹去旧版本
  - [x] 文件与照片
    - [x] 文件按原样保存，按内容命名，同一文件只存一份
    - [x] 从照片 EXIF 读取时间、地点和相机
    - [x] JPEG 和 PNG 缩略图
  - [x] 查找
    - [x] 全文搜索，支持中文、日文和韩文
    - [x] 时间线，从新到旧，可指定起始日期
    - [x] 地点：查找某个经纬度附近的一切
    - [x] 链接：与某人、某地或某件事情有关的一切
    - [x] 实时变化（`hs watch`、Server-Sent Events）
  - [ ] 围绕生活组织
    - [ ] 人物页：与某人有关的聊天、照片、文件和日程
    - [ ] 日期、地点和事情的页面
  - [ ] 聊天
  - [ ] 协作文档
  - [ ] 导入工具
    - [ ] Google Takeout
    - [ ] iCloud
    - [ ] 聊天记录导出
    - [ ] 密码管理器
  - [ ] 网页应用
- [ ] **身份与授权**
  - [x] 身份
    - [x] 身份是你自己的密钥，而不是某台服务器上的账号
    - [x] 身份日志和 `did:hs` DID 文档
    - [x] 恢复码只显示一次，是恢复备份的唯一钥匙
    - [x] 恢复码可以拆分交给朋友保管（Shamir 秘密共享）
  - [x] 授权
    - [x] 每个应用和每个人一个令牌
    - [x] 按角色（reader、writer、owner）、空间和条目类型限制
    - [x] 令牌可设置过期时间
    - [x] 列出所有令牌，随时收回任意一个
  - [x] 为其他应用提供登录
    - [x] OpenID Connect 身份提供方（授权码流程 + PKCE）
    - [x] 为 Caddy、Traefik、nginx 后面的应用提供转发认证
    - [x] 通行密钥
  - [ ] 用通行密钥添加新设备，每台设备有自己的密钥
- [ ] **homeserver 之间的分享**
  - [x] 与其他 homeserver 上的人共享空间
    - [x] 成员角色：reader、writer、owner
    - [x] 修改实时发送给成员
    - [x] 成员把写入发给负责排序的节点
    - [x] 把空间交给另一个节点
    - [x] 节点之间用 HTTP Message Signatures 证明身份
  - [ ] 端到端加密空间（MLS，RFC 9420）
  - [ ] 信任的人之间互助
    - [x] 保管朋友的加密副本
    - [ ] 中继
    - [ ] 为离线节点代收的信箱
  - [ ] 网络
    - [x] HTTP/1.1 和 HTTP/2，使用自签名证书或放在反向代理后面
    - [x] 家用路由器端口映射（NAT-PMP/PCP）
    - [ ] QUIC 和 HTTP/3
    - [ ] 自己实现的 NAT 穿透，不需要公网 IP 和域名
- [ ] **兼容现有应用**
  - [x] REST API，附带 OpenAPI 3.1 文档和带类型的 Rust 客户端
  - [x] 通讯录和日历的 CardDAV 与 CalDAV
  - [x] 兼容 Bitwarden 应用的密码库
  - [x] 供 AI 助手使用的 Model Context Protocol（MCP）
  - [x] Web Push 通知
  - [x] 用于保存副本的 S3 兼容接口
  - [ ] 文件的 WebDAV
  - [ ] 插件
    - [ ] 在沙箱中运行的 WebAssembly 组件
    - [ ] 只能访问你授权的数据，并用你的身份登录
- [ ] **备份**
  - [x] 把一切导出为普通 tar 文件
  - [x] 加密副本，每小时同步
    - [x] 到本地目录（USB 硬盘、NAS）
    - [x] 到任何兼容 S3 的存储
    - [x] 到朋友的 homeserver
  - [x] 只用恢复码就能恢复节点
  - [x] 随时从日志重建索引
  - [ ] 检查点与旧日志压缩
- [ ] **运行节点**
  - [x] 一个程序（`hs`）加一个数据目录，无需安装其他东西
  - [x] 不需要配置文件，用 `hs ctl config` 设置
  - [x] 管理节点（`hs ctl`）和数据（`hs`）的命令行，可在本机或远程使用
  - [x] bash、zsh、fish、elvish 和 PowerShell 的命令补全
  - [ ] 签名的自动升级，失败时回到旧版本
  - [ ] 一个身份跨多台自己的节点（家里、VPS、笔记本）
- [ ] **个人 AI**
  - [x] 助手只能获取其令牌允许的内容
  - [ ] 可查看、修改和删除的记忆层
  - [ ] 用一句话描述生成自己的小工具
- [ ] **长期使用**
  - [ ] 数字遗产

## 文档

[用户手册](docs/src/content/docs/zh-cn/user/index.md) · [管理手册](docs/src/content/docs/zh-cn/admin/index.md) · [开发者手册](docs/src/content/docs/zh-cn/dev/index.md) · [宣言](MANIFESTO.zh-CN.md) · [更新日志](CHANGELOG.md) · [参与贡献](CONTRIBUTING.zh-CN.md)

## 许可证

[AGPL-3.0-only](LICENSE)，附 [插件例外](PLUGIN-EXCEPTION.zh-CN.md)。插件开发包 `hs-plugin-api` 采用 MIT OR Apache-2.0。本软件部分基于 Independent JPEG Group 的工作。
