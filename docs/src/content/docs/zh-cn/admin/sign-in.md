---
title: 为其他应用提供登录
description: 把节点用作 OpenID Connect 身份提供方，或用 forward auth 保护应用。
---

你运行的其他应用（媒体服务器、wiki、相册）可以用 homeserver 登录，这样你和
家人只有一个身份，访问权限也在一处管理。

:::caution[0.1.0 版本]
两种方式都需要浏览器以所有者身份登录到节点。由于还没有网页应用，这种登录
仍很简陋。请把本页视为预览。
:::

## OpenID Connect

节点是 OpenID Connect 身份提供方（带 PKCE 的授权码流程）。应用可以从
`https://NODE/.well-known/openid-configuration` 获取全部信息。

用所有者令牌注册一个应用：

```sh
curl -H "Authorization: Bearer $OWNER_TOKEN" \
  -d '{"name": "Wiki", "redirect_uris": ["https://wiki.example.org/oauth/callback"], "confidential": true}' \
  https://NODE/oauth/clients
```

返回内容包含 `client_id`、`client_secret`（仅限机密客户端）和 `issuer`，
把它们填到应用里。ID 令牌用 EdDSA 和 ES256 签名，公钥在 `/oauth/jwks`。
用 `GET /oauth/clients` 列出已注册的应用。

## Forward auth

对于不支持 OpenID Connect 的应用，反向代理（Caddy、Traefik、nginx）可以
先询问节点，再决定是否放行请求。

1. 在节点停止时设置允许使用的域名：
   `hs ctl config set forward_domain example.org`。这个域名及其子域名下的
   应用都被允许。
2. 把代理的 forward auth 检查指向 `https://NODE/auth/verify`，并传递
   `X-Forwarded-Proto`、`X-Forwarded-Host` 和 `X-Forwarded-Uri`。
3. 已登录的请求得到 `200`，带有 `Remote-User`（你的身份）和
   `Remote-Name`（你的名字），代理可以把它们传给应用。其他请求被引导到
   `https://NODE/auth/login`，登录后返回原处。

以 Caddy 为例：

```
wiki.example.org {
  forward_auth https://hs.example.org {
    uri /auth/verify
    header_up Host {upstream_hostport}
    copy_headers Remote-User Remote-Name
  }
  reverse_proxy localhost:3000
}
```

登录有效期为一周，保存在 forward auth 域名下的 Cookie 中。
