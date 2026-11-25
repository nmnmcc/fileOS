---
title: 故障排除
description: 常见问题及处理方法。
---

## 检查节点健康状况

```sh
hs status
```

显示节点、你的登录、索引是否健康以及存了多少内容。`GET /api/health` 提供
同样的信息。节点自身的消息写到标准错误；在 systemd 下写到日志。

## 常见消息

**"another hs process is using …"**：节点正在运行并占用数据目录。改为通过
运行中的节点操作（登录后大多数命令会自动这样做），或者先停止节点。
`config set`、`restore` 和 `rebuild-index` 始终需要先停止节点。

**"… holds no node"**：该数据目录中没有节点。检查 `--dir` 或 `$HS_DIR`，
或用 `hs ctl init` 创建一个。

**"cannot reach …"**：`hs` 登录的节点没有运行或无法访问。用 `hs ctl serve`
启动它，或用 `--dir` 直接操作已停止节点的文件。

**"… did not accept that token"**：令牌错误、过期或已撤销。在节点上查看
`hs ctl token list`，并新建一个。

**`hs login` 报证书错误**：节点使用自签名证书。用 `hs login --insecure`。

**`--tls acme` 无法获取证书**：`url`（或 `--domain`）中的域名必须指向这台
机器，80 和 443 端口必须能从互联网访问。如果续期失败，节点会继续使用之前的
证书，并在一天后重试。

**"Could not map port"**：路由器不支持 NAT-PMP 或 PCP，或者关闭了它。请在
路由器上手动转发端口。

## 搜索或时间线不对

从日志重建索引，不会丢失任何东西：

```sh
hs ctl rebuild-index
```

## 机器没了

在另一台机器上安装 `hs`，用恢复码[从副本恢复](../backups/#恢复节点)。
如果恢复码也丢了，但你曾把它分给朋友，收集那些分片并运行
`hs ctl recovery combine`。
