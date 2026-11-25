---
title: 应用与助手
description: 让通讯录、日历、Bitwarden 和 AI 助手使用你的 homeserver。
---

每个应用从运行节点的人那里拿到自己的令牌
（[`hs ctl token create`](../../admin/access/)）。每个应用要一个，只给它需要
的权限，这样可以单独撤销。下面的例子里，`NODE` 是你的 homeserver 的地址。

## 联系人和日历

homeserver 支持 CardDAV 和 CalDAV，手机和电脑上的通讯录、日历可以与它同步：
Android 上的 DAVx⁵、iOS 和 macOS 自带的账户、Thunderbird 等。

- **服务器：** `https://NODE/`（需要完整 URL 的应用：`https://NODE/dav/`）
- **用户名：** 随意
- **密码：** 令牌

每个空间显示为一个通讯录和一个日历。联系人保存为 `contact/…` 条目，日程
保存为 `event/…` 条目，所以它们和其他内容一样出现在搜索和时间线中。

## 密码

Bitwarden 应用和浏览器扩展可以把密码库放在你的 homeserver 上。应用在发送
之前加密一切，节点只保存密文。

1. 请运行节点的人开放注册（见[配置](../../admin/configuration/)）。
2. 在应用里选择自托管服务器，填入 `https://NODE/bitwarden`。
3. 创建账户。每个节点只有一个密码库。

主密码不会离开应用。如果忘了主密码，密码库就无法打开，你和其他任何人都
不行。

## AI 助手

支持 Model Context Protocol（MCP）的助手可以读写令牌允许的内容，仅此而已。

- **URL：** `https://NODE/mcp`（HTTP 传输）
- **请求头：** `Authorization: Bearer 令牌`

一个限定在单个空间、角色为 `reader` 的令牌是不错的起点。助手能访问的一切
都可以在 `hs ctl token list` 里看到，并随时撤销。

## 其他自托管应用

你的 homeserver 还可以作为你或家人运行的其他应用的登录方式。这由管理员
设置，见[为其他应用提供登录](../../admin/sign-in/)。
