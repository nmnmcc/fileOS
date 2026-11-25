---
title: 备份与恢复
description: 明文导出、加密副本、恢复节点和重建索引。
---

备份有两种，两种都需要：

- **导出**：明文形式的数据，没有 homeserver 也能读；
- **加密副本**：整个节点的副本，持续更新，只凭恢复码就能把节点恢复回来。

## 导出

```sh
hs ctl export > homeserver-2026-11-25.tar
```

一个包含日志、blob、身份和配置的普通 tar 包。它不含密钥，但数据是未加密
的，请放在安全的地方。节点运行时也可以导出。

## 加密副本

副本是数据目录（不含索引和访问日志）的加密拷贝。每个文件离开前都用由恢复码
派生的密钥加密，并以不泄露任何信息的名字存放。保存副本的一方只能看到密文。

在节点停止时添加一个或多个位置：

```sh
hs ctl config set replicas '["/mnt/usb/homeserver", "s3://KEY:SECRET@s3.example.com/my-bucket/hs"]'
```

| 形式                                          | 位置                                     |
| --------------------------------------------- | ---------------------------------------- |
| `/some/dir` 或 `dir:/some/dir`                | 一个目录：U 盘、NAS 挂载点               |
| `s3://KEY:SECRET@host[:port]/bucket[/prefix]` | 任何兼容 S3 的存储；需要时加 `?region=…` |
| `hs://KEY:SECRET@host[:port]/bucket[/prefix]` | 另一个 homeserver，例如朋友的（见下文）  |

不使用 TLS 的存储加上 `?http=1`。密钥中的 `/` 等字符需要百分号编码
（`%2F`）。

运行中的节点每小时同步一次所有副本，只发送有变化的文件。要立即同步并查看
结果：

```sh
hs ctl replica sync
hs ctl replica list
```

## 恢复节点

在新机器上，用空的数据目录，只凭恢复码：

```sh
hs ctl restore --from /mnt/usb/homeserver
```

`hs` 询问恢复码（或读取 `$HS_RECOVERY_CODE`），取回并解密每个文件，然后
重建索引。之后用 `hs ctl serve` 启动。令牌和共享空间照常可用。

## 替朋友保存副本

你的节点可以保存朋友的加密副本，朋友也可以保存你的。给每位朋友一个只能
访问一个存储桶的密钥：

```sh
hs ctl config set s3_keys '[{"access_key": "bea", "secret_key": "一串足够长的随机密文", "bucket": "bea"}]'
```

朋友的文件存放在你数据目录的 `buckets/bea/` 下，由朋友事先加密。朋友再把
`hs://bea:SECRET@your-node.example/bea` 加到自己的 `replicas` 里。`rclone`
等其他 S3 工具也可以通过 `/s3` 使用同一个端点。

## 重建索引

索引（`index.db`）包含搜索、时间线和地点，全部由日志派生。如果它看起来不
对，在节点停止时重建：

```sh
hs ctl rebuild-index
```

删除 `index.db` 的效果相同，下次启动时会重建。
