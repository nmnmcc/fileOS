---
title: 开始使用
description: 用 hs 命令行登录你的 homeserver，并四处看看。
---

你需要从运行 homeserver 的人（可能就是你自己）那里拿到两样东西：

- 它的**地址**，例如 `hs.example.org`，在同一台机器上则是
  `127.0.0.1:8080`；
- 一个**令牌**：让你进入的一长串密文。搭建节点的人在创建节点时拿到了所有者
  令牌；其他人的令牌由 [`hs ctl token create`](../../admin/access/) 生成。

你的电脑上还需要有 `hs` 程序，见[安装](../../admin/install/)。

## 登录

```sh
hs login hs.example.org
```

`hs` 会询问令牌，粘贴时不显示。它也可以从标准输入或 `$HS_TOKEN` 读取令牌。
令牌被接受后，`hs` 会记住这个节点：

```
Signed in to Ada (https://hs.example.org) as owner
```

如果节点使用自签名证书，加上 `--insecure`。你可以登录多个节点；`hs` 使用
最近登录的那个，或用 `--server URL`（或 `$HS_SERVER`）指定。`hs logout`
会忘掉一次登录。

## 四处看看

```sh
hs status          # 哪个节点、你是谁、是否健康
hs space list      # 你能看到的空间
hs timeline        # 最近的条目，新的在前
```

每个命令都可以用 `--help` 查看说明。给任何命令加上 `--json`，就会原样输出
API 的返回内容，方便写脚本。

## 输入时补全命令

`hs` 可以在 shell 里补全命令、空间名和授权。在 shell 的启动文件里加一行：

```sh
source <(hs completion bash)    # bash；zsh 换成 zsh
hs completion fish | source     # fish
```

也支持 Elvish 和 PowerShell。
