---
title: 运行节点
description: 运行节点，放在反向代理后面，启用端口映射，或作为系统服务。
---

```sh
hs ctl serve
```

在 `127.0.0.1:8080` 上提供明文 HTTP，只有本机能访问。日志写到标准错误。
用 Ctrl-C 或 `SIGINT` 停止；它会先写完手头的数据。

## 从局域网或互联网访问

| 你的情况             | 运行                                                    |
| -------------------- | ------------------------------------------------------- |
| 只在本机使用         | `hs ctl serve`                                          |
| 家庭网络，没有域名   | `hs ctl serve --listen [::]:8443 --tls self-signed`     |
| 有指向公网地址的域名 | `hs ctl serve`，前面放[反向代理](#公网域名上的-https)   |
| 在家用路由器后面     | 再加上 `--map-port`                                     |

**`--tls self-signed`** 生成一次证书并保存在 `keys/self-signed.pem`。应用
会对它发出警告；`hs login --insecure` 可以接受它。

**`--map-port`** 请家用路由器转发端口（NAT-PMP 或 PCP），并持续续期映射。
0.1.0 不支持只会 UPnP 的路由器，请在路由器上手动转发端口。

## 公网域名上的 HTTPS

homeserver 为节点之间直接、点对点的连接而设计，不会自己为公网域名申请
证书。如果需要公网 HTTPS 地址，请在 `hs ctl serve` 前面放一个 Caddy 或
nginx 之类的反向代理，并把配置中的 `url` 设为公网地址。

用 Caddy，由它申请和续期证书：

```
home.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

用 nginx，证书由你自己管理：

```nginx
server {
    listen 443 ssl;
    server_name home.example.org;
    ssl_certificate     /etc/ssl/home.example.org.pem;
    ssl_certificate_key /etc/ssl/home.example.org.key;
    client_max_body_size 0;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_request_buffering off;
        proxy_buffering off;
    }
}
```

让 `hs` 只监听 `127.0.0.1`，这样只有代理能访问它。

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
ExecStart=/usr/local/bin/hs ctl serve
KillSignal=SIGINT
Restart=on-failure
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
