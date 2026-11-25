---
title: 令牌与授权
description: 给人和应用访问权限，查看谁有权限，并收回。
---

每个人、每个应用和每个助手都用令牌访问节点。每个令牌都是一项**授权**：
一个角色，以及可选的空间和条目类型限制。所有授权在同一处列出，在同一处
撤销。

## 创建令牌

```sh
hs ctl token create --role reader --space 家庭 --label "外婆的手机"
```

令牌只打印一次；转交出去，自己不要留副本。选项：

| 选项           | 含义                                 |
| -------------- | ------------------------------------ |
| `--role`       | `reader`、`writer`（默认）或 `owner` |
| `--space 空间` | 只限这个空间；可重复 [默认：全部]    |
| `--type 类型`  | 只限这种条目，例如 `contact`；可重复 |
| `--expires-in` | `90m`、`12h`、`30d`、`4w`            |
| `--label`      | 用途，显示在列表中                   |

给每个应用单独的令牌，权限尽量小。日历应用只需要 `--type event`；AI 助手
可能只需要某一个空间的 `--role reader`。

## 查看和撤销

```sh
hs ctl token list
```

```
ID                ROLE    SPACES   TYPES  CREATED           EXPIRES           LABEL
moxptcvmtqzt6ljj  reader  家庭     all    2026-11-25 11:57  2026-12-25 11:57  外婆的手机
zk5hxplo2lxjrofg  owner   all      all    2026-11-25 11:57                    owner (created by hs ctl init)
```

```sh
hs ctl token revoke moxptcvmtqzt6ljj
```

撤销立即生效。授权是私人空间日志中的条目，所以历史会保留；用令牌发出的
每个请求都记录在数据目录的 `access.log` 中。

## 角色

| 角色     | 可以                       |
| -------- | -------------------------- |
| `reader` | 读取                       |
| `writer` | 读取和写入                 |
| `owner`  | 一切，包括令牌、成员和导出 |

共享空间的成员是其他节点，而不是令牌，用 `hs space member` 管理，见
[分享](../../user/sharing/)。
