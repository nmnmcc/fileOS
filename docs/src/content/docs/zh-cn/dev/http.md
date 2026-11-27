---
title: "HTTP 与浏览器"
description: App 和插件怎样通过 HTTP 提供服务，每个都有自己的 origin，用户不用配置任何域名，也不用 Cookie。
---

:::note[状态：设计，部分已实现]
网关和 App 作为 HTTP 服务器（[计划](#计划)的第 1、2 步）已经实现，其余部分还没有实现。
:::

## 目标

- **一切交互都通过 HTTP。** 命令行、网页界面和其他客户端都使用内核的 HTTP API。
- **App 和插件都是 HTTP 服务器。** 它们都可以通过 HTTP 向用户提供服务或界面。
- **用户不用配置域名。** 没有 DNS 记录，也没有证书。
- **App 能用完整的 Web 平台**，并且在浏览器里彼此隔离。

## 每个 App 一个 origin

浏览器的安全边界是 origin：协议、主机名和端口。两个同 origin 的 App 可以互相读取存储，也能冒充对方发请求。所以每个 App 都有自己的 origin：

```
http://fileos.localhost:<端口>/            内核：界面、登录、API
http://<app>.localhost:<端口>/       每个 App 一个 origin
http://<plugin-id>.localhost:<端口>/    带界面的插件
```

- **不用配置。** RFC 6761 规定 `.localhost` 下的所有名字都指向本机。Chrome、Firefox 和 Safari 都不经过 DNS，直接把 `*.localhost` 解析到本机。
- **不用证书。** 浏览器把 `localhost` 当作安全上下文，Service Worker、WebAuthn 等 Web API 都能用。
- **名字是 App 在身份中的名字**，在身份的每台设备上都相同，因为 App 随身份同步（见 [App 作为 Worker](../workers/)）。浏览器的存储跟着 origin 走，所以一个名字被另一个 App 再次使用时，内核会在它的第一个回应里发送 `Clear-Site-Data`。（原型目前仍在每个节点上由 App 文件夹生成 ID，例如 `photos-k3x9q2m7ab`。）

不同端口隔不开 Cookie（RFC 6265），路径在浏览器里也隔不开任何东西。所以 App 放在主机名里，而不是端口或路径里。

## App 和插件用 `wasi:http` 提供 HTTP

我们不发明自己的接口。提供 HTTP 的组件导出 WASI 0.2 的 `wasi:http/incoming-handler`，由 wasmtime 运行。

- 内核接收连接、读取请求。组件只拿到请求对象：没有 socket，也看不到其他 origin 的请求。
- 没有中间件。组件看不到、也改不了其他 origin 的请求。这守住了 “插件不改变 App 看到的东西” 这条规则。
- 每个请求都有 fuel、内存和时间的限制。请求和回应的内容以流的方式传递。

## 不用 Cookie：登录由内核负责

所有登录都由内核用标准协议完成。App 看不到密码、会话和令牌。

1. 用户打开 `http://photos.localhost/`。App 的静态文件不需要登录：它们是 App 的代码，不是数据。
2. App 没有令牌，就带着 PKCE challenge 把浏览器转到 `http://fileos.localhost/authorize`。
3. 内核的页面让用户登录（用 passkey），并询问是否允许这个 App 代表用户行事。这个页面在内核自己的 origin 上，App 伪造不了。
4. 内核带着一个授权码把浏览器转回 App。App 在 `http://fileos.localhost/token` 用授权码换取令牌（CORS 只允许这个 App 的 origin）。
5. App 在 `Authorization` 头里带上令牌。内核验证后删掉这个头，加上 `FileOS-User: did:fileos:…`，再把请求交给 App。

所用的标准是 OAuth 2.0（RFC 6749）加 PKCE（RFC 7636），做法和 “OAuth 2.0 for Browser-Based Applications” 一样。令牌用 DPoP（RFC 9449）绑定到一把无法从浏览器中取出的密钥上，所以被偷的令牌在别处没有用。刷新令牌每用一次就更换，也可以撤销（RFC 7009）。内核同时也是其他服务的 OpenID Connect 身份提供方。

没有 Cookie，浏览器就不会自动发送凭据，所以跨站请求伪造无从发生。`<img>`、`<video>` 和下载链接无法带上 `Authorization` 头，所以内核为它们签发短时有效的签名 URL。

## 网关做什么

- 只在本机（127.0.0.1 和 ::1）监听，使用一个用户可以修改的固定端口。其他设备通过 libp2p 连接，而不是通过这个端口。
- 只接受已知的主机名（`fileos.localhost`、`*.localhost`），防止 DNS rebinding。
- 按主机名把请求交给对应的组件。
- 删掉 App 回应里的 `Set-Cookie`，也从不把 `Cookie` 头发给 App。
- 加上安全的默认头：`Content-Security-Policy`、`X-Content-Type-Options: nosniff`、`Cross-Origin-Opener-Policy`、`Cross-Origin-Resource-Policy` 和 `Referrer-Policy`。App 可以把它们收得更紧；能否放宽是一项有默认值的策略。

HTTP API 遵循 RFC 9110：用 `ETag` 和 `If-Match` 处理版本，用 `Range` 处理大文件，错误用 `application/problem+json`（RFC 9457）。

## 浏览器和内核在哪里

| 场景                       | App 的地址                | 用户要配置    |
| -------------------------- | ------------------------- | ------------- |
| 装了节点的设备（增强模式） | `<app>.localhost`         | 什么都不用    |
| 你的另一台装了节点的设备   | 同上，由本机节点转发      | 什么都不用    |
| 没有节点的设备             | `<app>.<节点>.<共享域名>` | 什么都不用    |
| 你自己的域名（可选）       | `<app>.<你的域名>`        | 一条 DNS 记录 |

### 增强模式：设备上有节点

节点自己提供网页界面，界面文件打包在 `fileos` 程序里。代码来自你安装在自己机器上的程序，而不是每次从服务器下载。其他节点上的 App 由本机节点通过 libp2p 转发过来。项目的网页只负责找到本机节点，并把用户交给它。

### 没有节点的设备：项目的共享域名

我们采用 Sandstorm 的 sandcats.io 的做法。项目有一个共享域名，每个节点得到 `*.<节点>.<共享域名>`：

- 节点用自己的节点密钥向项目的 DNS 服务证明这个名字属于它，再用 ACME（DNS-01）为这个名字申请证书。证书的私钥只留在节点上。
- 中继按 TLS 的服务器名（SNI）转发连接，看不到内容：TLS 在你的节点上终止。
- 共享域名登记在 Public Suffix List 里，所以在浏览器看来，每个节点都是一个独立的站点。

和会合服务一样，这是一个没有特权的默认值。任何人都可以自己部署一套，有自己域名的人也可以用自己的域名。

### 基础模式：浏览器里的节点

一个任何人都能部署的静态网页，在浏览器里运行一个轻节点：portable crate 编译成 WebAssembly，存储用 OPFS，网络通过中继用 WebSocket 连接。它是一个入口，而不是你数据的家：

- 代码每次都来自服务器，所以你要信任这台服务器。项目让每次构建的结果都相同并公布哈希，任何人也都可以部署自己的副本。
- 浏览器可能删除一个网站的数据（Safari 会在 7 天没有使用后删除，安装为应用的网页除外）。浏览器里的节点永远不是唯一的副本。
- 标签页关闭时它就离线，所以它不当排序节点、会合节点或中继。

## 计划

1. **网关：** 固定端口，只在本机，只接受已知的主机名，安全的默认头。
2. **App 成为 HTTP 服务器：** 运行时支持 `wasi:http`，按主机名转发请求。
3. **登录：** `/authorize`、`/token` 和 `/revoke`，passkey，DPoP，`FileOS-User` 头，以及签名 URL。
4. **把网页界面打包进 `fileos`**，以及把用户转到本机节点的项目网页。
5. **为没有节点的设备提供共享域名。**
6. **浏览器里的节点。**

## 值得学习的项目

- **Sandstorm**：每个 App 一个 origin；以及 sandcats.io，它之所以诞生，是因为通配 DNS 和证书是自建服务器的人最难过的一关。
- **Tailscale Serve** 和 **oauth2-proxy**：由代理负责登录，再通过请求头把用户身份交给 App。
- **Figma 插件** 和 **VS Code webview**：App 在浏览器页面里能做什么、不能做什么。
