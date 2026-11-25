---
title: 安装
description: 获取 hs 可执行文件并创建节点。
---

`hs` 是一个包含一切（包括 SQLite）的单一可执行文件，运行时不需要安装其他
任何东西。

## 构建

0.1.0 还没有预编译的下载。从仓库构建：

```sh
# 用 Nix 和 devenv：发布构建在 ./result/bin
devenv build outputs.hs

# 或用 Rust（rustup 会选择正确的版本）
cargo build --release --package hs
```

把 `hs` 复制到 `PATH` 中的某个位置，例如 `/usr/local/bin`。可以再建一个名为
`hsctl` 的链接；以这个名字启动时，`hs` 的行为等同于 `hs ctl`。Nix 构建已
包含这个链接。

```sh
ln -s hs /usr/local/bin/hsctl
```

## 创建节点

以将来运行节点的用户身份执行：

```sh
hs ctl init --name "Ada" --url https://hs.example.org
```

- `--name` 会显示给与你分享的人。
- `--url` 是其他节点和应用访问本节点的地址。只在本机使用的节点可以省略，
  之后也可以[再设置](../configuration/)。

`hs ctl init` 打印三样东西：

```
Identity: n22carktxylkeljiv6fipjzz3xyusp6hm6wxvl2xiksnoui75vgq

Recovery code (write it down; it is not stored anywhere):

    l6zf-miqq-zl5c-…

Owner token for your first browser or app:

    hs_d3sje7yqzelf…
```

**现在就把恢复码抄下来。** 它只显示这一次，也是从加密备份恢复的唯一途径。
见[恢复码](../../user/recovery/)。

所有者令牌可以做任何事。本机的 `hs` 已经用它登录；留给自己用，给其他人
发各自的[令牌](../access/)。

## 数据目录

节点默认位于 `$XDG_DATA_HOME/homeserver`，通常是
`~/.local/share/homeserver`。每个命令都可以用 `--dir 目录` 或 `$HS_DIR`
指定其他位置。

```
config.json   配置
identity      身份日志
keys/         节点密钥、存储密钥和证书（机密）
logs/         每个空间一个目录：签名日志
blobs/        按 BLAKE3 哈希存放的文件内容
index.db      搜索、时间线、地点：派生数据，可以删除
access.log    对节点的请求记录
node.lock     由运行中的进程持有
```

`logs/`、`blobs/`、`identity`、`config.json` 和 `keys/` 构成节点本身，其余都
可以重建。

下一步：[运行节点](../running/)。
