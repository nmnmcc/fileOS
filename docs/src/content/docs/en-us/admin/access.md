---
title: Tokens and grants
description: Give people and apps access, see who has it, and take it back.
---

Every person, app and assistant reaches the node with a token. Each token is
a **grant**: a role, and optionally the spaces and types of items it is
limited to. All grants are listed in one place and revoked in one place.

## Create a token

```sh
hs ctl token create --role reader --space Family --label "Grandma's phone"
```

The token is printed once; pass it on and do not keep a copy. Options:

| Option          | Meaning                                                    |
| --------------- | ---------------------------------------------------------- |
| `--role`        | `reader`, `writer` (the default) or `owner`                |
| `--space SPACE` | only this space; repeat for more [default: all]            |
| `--type TYPE`   | only this type of item, such as `contact`; repeat for more |
| `--expires-in`  | `90m`, `12h`, `30d`, `4w`                                  |
| `--label`       | what it is for, shown in the list                          |

Give each app its own token with as little as it needs. A calendar app needs
`--type event`; an AI assistant may need only `--role reader` on one space.

## See and revoke

```sh
hs ctl token list
```

```
ID                ROLE    SPACES   TYPES  CREATED           EXPIRES           LABEL
moxptcvmtqzt6ljj  reader  Family   all    2026-11-25 11:57  2026-12-25 11:57  Grandma's phone
zk5hxplo2lxjrofg  owner   all      all    2026-11-25 11:57                    owner (created by hs ctl init)
```

```sh
hs ctl token revoke moxptcvmtqzt6ljj
```

Revoking takes effect at once. Grants are entries in the private space's
log, so their history is kept, and every request made with a token is
recorded in `access.log` in the data directory.

## Roles

| Role     | Can                                              |
| -------- | ------------------------------------------------ |
| `reader` | read                                             |
| `writer` | read and write                                   |
| `owner`  | everything, including tokens, members and export |

Members of shared spaces are other nodes, not tokens. They are managed with
`hs space member`; see [Sharing](../../user/sharing/).
