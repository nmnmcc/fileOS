---
title: 运行节点
description: 运行节点，启用 TLS、端口映射，或作为系统服务。
---

```sh
hs ctl serve
```

在 `127.0.0.1:8080` 上提供明文 HTTP，只有本机能访问。日志写到标准错误。
用 Ctrl-C 或 `SIGINT` 停止；它会先写完手头的数据。

## 从局域网或互联网访问

| 你的情况                 | 运行                                                |
| ------------------------ | --------------------------------------------------- |
| 只在本机使用             | `hs ctl serve`                                      |
| 家庭网络，没有域名       | `hs ctl serve --listen [::]:8443 --tls self-signed` |
| 有指向公网地址的域名     | `hs ctl serve --tls acme --email you@example.org`   |
| 同上，但在家用路由器后面 | 再加上 `--map-port`                                 |

**`--tls self-signed`** 生成一次证书并保存在 `keys/self-signed.pem`。应用
会对它发出警告；`hs login --insecure` 可以接受它。

**`--tls acme`** 为配置中 `url` 的主机名（或每个 `--domain`）向 Let's
Encrypt 申请证书，并自动续期。它监听 `[::]:443`，并在 `[::]:80` 上应答
ACME 验证，两者都必须能从互联网访问。用 `--acme-directory URL` 换用其他
ACME 服务，用 `--http-listen` 改变验证用的监听地址。

**`--map-port`** 请家用路由器转发端口（NAT-PMP 或 PCP），并持续续期映射。
0.1.0 不支持只会 UPnP 的路由器，请在路由器上手动转发端口。

监听 1024 以下的端口需要 root 或相应的权限；下面的服务配置在不以 root
运行的前提下授予这个权限。

## 作为系统服务

一个 systemd 单元示例，用户名为 `hs`，数据位于 `/var/lib/homeserver`：

```ini
# /etc/systemd/system/homeserver.service
[Unit]
Description=homeserver
After=network-online.target
Wants=network-online.target

[Service]
User=hs
Environment=HS_DIR=/var/lib/homeserver
ExecStart=/usr/local/bin/hs ctl serve --tls acme --email you@example.org
KillSignal=SIGINT
Restart=on-failure
AmbientCapabilities=CAP_NET_BIND_SERVICE
StateDirectory=homeserver

[Install]
WantedBy=multi-user.target
```

先以该用户、用同一目录创建节点：
`sudo -u hs HS_DIR=/var/lib/homeserver hs ctl init --name … --url …`。
然后运行 `systemctl enable --now homeserver`，用
`journalctl -u homeserver` 查看日志。

## 同一时间只有一个进程

运行中的节点持有 `node.lock`。节点运行时，能通过 API 完成的命令（令牌、
导出、副本）会改走 API，而不直接读写文件。少数命令需要先停止节点：
`hs ctl config set`、`hs ctl restore` 和 `hs ctl rebuild-index`。
