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

- **一切都是文件夹。** 你的照片、笔记、家庭聊天，甚至 App，都是文件夹。
- **共享文件夹就是多个节点都保存的文件夹。** 你把一个文件夹分享给朋友，朋友的节点就保存一份副本，双方的修改会互相同步。
- **你的节点，你的规则。** 每个节点都有规则，决定谁可以读、写或保存什么。每条规则你都能看到，也都能收回。
- **App 只能看到你允许它看的东西。** App 是运行在沙箱里的小程序，只能使用你给它的文件夹。

## `fos` 命令

`fos` 用于和节点的内核交互。命令分为几组：

| 组                         | 用途                                               |
| -------------------------- | -------------------------------------------------- |
| `fos node`                 | 本节点：`init`、`run`、`status`、`connect`         |
| `fos fs`                   | 文件和流：`ls`、`cat`、`put`、`append`、`rm`       |
| `fos folder`               | 文件夹：`new`、`mount`、`grant`                    |
| `fos identity`（`fos id`） | 你的身份：`new`、`add-node`、`remove-node`、`join` |
| `fos app`                  | `pack` 打包或 `install` 安装 App                   |
| `fos curl`                 | 带着节点令牌运行 curl，例如 `fos curl /v0/status`  |

App 做了什么不在 `fos` 里，要在 App 里看。`fos help <组>` 会进一步说明每一组。

## 使用你的文件

这些命令作用于你电脑上运行的节点。路径以文件夹开头，比如 `/photos/2026/beach.jpg`。

| 命令                              | 作用                             |
| --------------------------------- | -------------------------------- |
| `fos fs ls /photos`               | 列出文件夹里的名字               |
| `fos fs cat /notes/todo.txt`      | 显示一个文件                     |
| `fos fs put /notes/todo.txt FILE` | 放入一个文件（或者你输入的文字） |
| `fos fs rm /notes/todo.txt`       | 删除文件；旧版本会保留           |
| `fos fs append /family/chat "…"`  | 在流（比如聊天）末尾加一行       |
| `fos node status`                 | 查看节点在做什么                 |

## 分享文件夹

1. 创建文件夹：`fos folder new family /family`，它会显示文件夹 ID。
2. 允许朋友的节点使用它：`fos folder grant /family <对方的节点密钥> all`。
3. 朋友在自己的节点上挂载它：`fos folder mount <文件夹 ID> /family`。

现在你们看到的是同样的文件。朋友的写入先发到你的节点，由你的节点排好顺序，再同步给所有人。

如果朋友节点的规则和你的不同，它的副本可能会停在它的规则不接受的第一处修改上。这时 `fos node status` 会显示 `FORK`。什么都不会丢：朋友可以修改自己的规则，或者另建一个文件夹。

## 一个身份，多个节点

身份是你这个人，节点是你的一台机器。你的身份是一个 DID（`did:fos:…`），有了它，你的所有节点都能被认出是你的。

1. 在要保存身份密钥的节点上（比如你的笔记本）运行 `fos identity new`。它会打印你的 DID，并给出两个密钥文件的位置，请把它们备份到安全的地方。
2. 在另一个节点上运行 `fos node status`，复制它的节点密钥。
3. 在第一个节点上运行 `fos identity add-node <节点密钥>`，它会打印一条命令。
4. 在另一个节点上运行这条命令：`fos identity join <文件夹 ID>`。

`fos identity` 显示你的身份、它的节点和它的链接。链接是你分享给别人的东西。只有保存签名密钥的节点才能添加节点（`fos identity add-node`）或移除节点（`fos identity remove-node`）。

## App

App 以 `.app` 文件的形式提供，比如 `Photos.app`。用 `fos app install Photos.app` 安装。安装时，节点只给 App 它的 manifest 里申请的权限。`fos app install` 会打印 App 的地址，例如 `http://photos-…localhost:7341/`，在浏览器里打开即可。`fos` 命令只用于内核：App 做了什么，要在 App 里看。

fileOS 自带两个 App：

- **照片**：显示 `/photos` 中的照片，可以上传新照片，并为照片尺寸建立索引。
- **聊天**：把聊天室保存在 `/chat` 中。用 `fos folder new chat /chat` 创建这个文件夹，把它分享给朋友的节点，然后你们各自通过自己节点上的聊天 App 交谈。

如何运行节点，见[管理手册](../admin/)。
