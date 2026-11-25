---
title: REST API
description: 有类型的 REST API、它的 OpenAPI 文档和 Rust 客户端。
---

`hs` 命令行和其他所有客户端使用同一套 API，所以它能做的，你的程序也能做。

## OpenAPI 文档

API 由一份 OpenAPI 3.1 文档描述，从服务器使用的 Rust 类型派生：

- 每个节点在 `/api/openapi.json` 提供；
- 由 `hs openapi` 打印；
- 作为 `docs/openapi.json` 保存在仓库中，由 CI 检查。

可以用它为任何语言生成客户端，或查阅每个请求和响应的细节。

## 认证

以 bearer 令牌发送令牌：

```sh
curl -H "Authorization: Bearer $TOKEN" https://NODE/api/spaces
```

节点也接受把令牌作为 HTTP Basic 认证的密码（供 DAV 客户端使用），以及作为
会话 Cookie（`POST /api/session`）。令牌由
[`hs ctl token create`](../../admin/access/) 生成；令牌能做什么由它的授权
决定：一个角色，以及可选的空间和资源类型列表。节点之间的请求则改用 HTTP
Message Signatures（RFC 9421）签名。

## 资源

条目位于所属空间之下：

```
GET    /api/spaces/{space}/resources?type=note&limit=50
GET    /api/spaces/{space}/resources/{type}/{id}
PUT    /api/spaces/{space}/resources/{type}/{id}
PATCH  /api/spaces/{space}/resources/{type}/{id}    application/merge-patch+json
DELETE /api/spaces/{space}/resources/{type}/{id}
```

HTTP 语义按标准使用：每个资源都有 `ETag`；发送 `If-Match` 只更新你读到的
版本（过期的写入得到 `412`），发送 `If-None-Match: *` 只在不存在时创建。
blob 按 BLAKE3 哈希存放在 `/api/blobs/{hash}`，永不改变，支持 `Range`。

跨空间：`/api/search`、`/api/timeline`、`/api/near`、
`/api/related/{space}/{type}/{id}`。变化以 Server-Sent Events 从
`/api/events` 推送。

## Rust 客户端

`hs-client` crate 包含请求和响应类型，以及基于它们的有类型客户端；`hs` 就
建立在它之上。它的 OpenAPI schema 放在一个可选 feature 后面，客户端不必为此
付出代价。
