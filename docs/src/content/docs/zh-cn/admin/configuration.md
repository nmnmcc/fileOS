---
title: 配置
description: config.json 中的设置，以及如何修改。
---

配置是数据目录中的 `config.json`。在节点停止时用 `hs ctl config` 读取和
修改：

```sh
hs ctl config get
hs ctl config get url
hs ctl config set url https://hs.example.org
hs ctl config set replicas '["/mnt/backup/hs"]'
```

值按 JSON 解析，解析失败则视为字符串。修改后重启节点。

## 设置项

| 键                   | 值                | 作用                                                                        |
| -------------------- | ----------------- | --------------------------------------------------------------------------- |
| `name`               | 字符串            | 你的名字，显示给与你分享的人                                                |
| `url`                | 字符串            | 其他人访问本节点的地址；也是 `--tls acme` 使用的域名                        |
| `replicas`           | 字符串列表        | 加密副本存放的位置，见[备份](../backups/)                                   |
| `s3_keys`            | 对象列表          | 允许别人把副本存在这里的密钥，见[备份](../backups/#替朋友保存副本)          |
| `bitwarden_register` | `true` 或 `false` | 允许创建一个 Bitwarden 账户；创建后改回 `false`                             |
| `forward_domain`     | 字符串            | 可以使用 forward auth 的应用所在的域名，见[为其他应用提供登录](../sign-in/) |
| `push_subject`       | 字符串            | 发给 Web Push 服务的联系方式，例如 `mailto:you@example.org`；默认取 `url`   |

`identity` 和 `private` 由 `hs ctl init` 写入，不得修改。

## 环境变量

| 变量               | 使用者           | 含义                          |
| ------------------ | ---------------- | ----------------------------- |
| `HS_DIR`           | 所有命令         | 数据目录（同 `--dir`）        |
| `HS_SERVER`        | 所有命令         | 要访问的节点（同 `--server`） |
| `HS_TOKEN`         | `hs login`       | 令牌，不再询问                |
| `HS_RECOVERY_CODE` | `hs ctl restore` | 恢复码，不再询问              |

`hs login` 的登录信息保存在 `$XDG_CONFIG_HOME/homeserver/cli.json`，通常是
`~/.config/homeserver/cli.json`。
