---
title: 用户手册
description: fileOS 是什么，以及怎样使用它。
---

:::caution[原型]
fileOS 还是原型。目前通过命令行使用，数据格式还会改变。重要的东西请在别处也保留一份。
:::

## fileOS 是什么

fileOS 把你的文件保存在你自己控制的电脑上：家里的小盒子、服务器、你的笔记本。每一台都是一个**节点**。你的节点和你认识的人的节点会互相同步。

只有几个概念：

- **一切都是文件。** 你的照片、笔记、家庭聊天的每条消息，甚至 App，都是文件。
- **文件在空间里。** 空间是作为一个整体同步的一组文件，比如 `/photos`。你把一个空间分享给朋友，朋友的节点就保存一份副本，双方的修改会互相同步。每个节点任何时候都能写，离线时也能。
- **访问就是授权，而且你看得到。** 授权说明谁可以对哪些文件做什么（读、修改、转授、保存）。每项授权你都能看到，也都能收回；由它转授出去的也一起收回。
- **App 只能看到你允许它看的东西。** App 是运行在沙箱里的小程序，只能使用你给它的路径。

## `fos` 命令

`fos` 用于和节点的内核交互。命令分为几组：

| 组                         | 用途                                                          |
| -------------------------- | ------------------------------------------------------------- |
| `fos node`                 | 本节点：`init`、`run`、`status`、`connect`                    |
| `fos fs`                   | 文件：`ls`、`cat`、`put`、`append`、`rm`                      |
| `fos space`                | 空间：`new`、`mount`、`grant`、`grants`、`take`、`checkpoint` |
| `fos identity`（`fos id`） | 你的身份：`new`、`add-node`、`remove-node`、`join`            |
| `fos app`                  | `pack` 打包或 `install` 安装 App                              |
| `fos curl`                 | 带着节点令牌运行 curl，例如 `fos curl /v0/status`             |

App 做了什么不在 `fos` 里，要在 App 里看。`fos help <组>` 会进一步说明每一组。

## 使用你的文件

这些命令作用于你电脑上运行的节点。路径以空间的挂载位置开头，比如 `/photos/2026/beach.jpg`。

| 命令                               | 作用                             |
| ---------------------------------- | -------------------------------- |
| `fos fs ls /photos`                | 列出某个路径下的名字             |
| `fos fs cat /notes/todo.txt`       | 显示一个文件                     |
| `fos fs put /notes/todo.txt FILE`  | 放入一个文件（或者你输入的文字） |
| `fos fs rm /notes/todo.txt`        | 删除文件；旧版本会保留           |
| `fos fs append /notes/log.txt "…"` | 在文件末尾加一行                 |
| `fos node status`                  | 查看节点在做什么                 |

## 分享空间

1. 创建空间：`fos space new /family`，它会显示空间 ID。
2. 允许朋友的节点使用它：`fos space grant /family <对方的节点密钥> read-write`，它会显示这项授权的 ID。
3. 朋友在自己的节点上挂载它：`fos space mount <空间 ID> /family`。

现在你们看到的是同样的文件，任何一方都能写，离线时也能。你们同时修改同一个文件时，两个版本都会保留。

可以授予的：

| 模板         | 授予什么                             |
| ------------ | ------------------------------------ |
| `read`       | 列出、读取、查看历史、监听、保存副本 |
| `read-write` | `read`，再加创建、修改和删除文件     |
| `manage`     | `read-write`，再加授予和收回访问     |
| `relay`      | 只保存副本（用于备份节点）           |

也可以授予具体的权限，比如 `files.read,files.create`。作用在一个文件路径上的授权，在文件改名后仍然跟着这个文件；作用在目录上的授权，作用于它下面的每个名字。

要分享给一个人和他的所有设备，就把访问授予他的身份，而不是某个节点密钥：`fos space grant /family did:fos:… read-write`。他的每个节点，包括以后加入的，都会拿到这个空间。

`fos space grants /family` 列出授权。`fos space take /family <授权 ID>` 收回一项授权，由它转授出去的也一起收回。对方在你收回访问的同时做的事没有效果。

## 一个身份，多个节点

身份是你这个人，节点是你的一台机器。你的身份是一个 DID（`did:fos:…`），有了它，你的所有节点都能被认出是你的。

1. 在要保存身份密钥的节点上（比如你的笔记本）运行 `fos identity new`。它会打印你的 DID，并给出两个密钥文件的位置，请把它们备份到安全的地方。
2. 在另一个节点上运行 `fos node status`，复制它的节点密钥。
3. 在第一个节点上运行 `fos identity add-node <节点密钥>`，它会打印一条命令。
4. 在另一个节点上运行这条命令：`fos identity join <空间 ID>`。

`fos identity` 显示你的身份、它的节点和它的链接。链接是你分享给别人的东西。只有保存签名密钥的节点才能添加节点（`fos identity add-node`）或移除节点（`fos identity remove-node`）。

## App

App 以 `.app` 文件的形式提供，比如 `Photos.app`。用 `fos app install Photos.app` 安装。安装时，节点只给 App 它的 manifest 里申请的权限。`fos app install` 会打印 App 的地址，例如 `http://photos-…localhost:7341/`，在浏览器里打开即可。`fos` 命令只用于内核：App 做了什么，要在 App 里看。

App 只能看到它自己的视图，永远看不到你的文件真正在哪里：

| 命令                                                    | 作用                                    |
| ------------------------------------------------------- | --------------------------------------- |
| `fos app grants photos`                                 | App 看到什么，以及每一部分代表什么      |
| `fos app grant photos /pictures read`                   | 授予更多；它在 `/granted/<handle>` 看到 |
| `fos app grant photos /pictures read-write --as photos` | 让它的 `/photos` 就是你的 `/pictures`   |
| `fos app take photos granted/<handle>`                  | 再收回                                  |

每个 App 还有自己的私有数据，别的 App 读不到。

fileOS 自带两个 App：

- **照片**：显示 `/photos` 中的照片，可以上传新照片，并为照片尺寸建立索引。
- **聊天**：把聊天室保存在 `/chat` 中，每条消息一个文件。用 `fos space new /chat` 创建这个空间，把它分享给朋友的节点，然后你们各自通过自己节点上的聊天 App 交谈。

如何运行节点，见[管理手册](../admin/)。
