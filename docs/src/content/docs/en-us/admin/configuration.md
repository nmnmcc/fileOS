---
title: Configuration
description: The settings in config.json, and how to change them.
---

The configuration is `config.json` in the data directory. Read and change it
with `hs ctl config` while the node is stopped:

```sh
hs ctl config get
hs ctl config get url
hs ctl config set url https://hs.example.org
hs ctl config set replicas '["/mnt/backup/hs"]'
```

`hs` reads the value as JSON; if it is not JSON, it takes it as a string.
Start the node again after a change.

## Settings

| Key                  | Value             | What it does                                                                                    |
| -------------------- | ----------------- | ----------------------------------------------------------------------------------------------- |
| `name`               | string            | Your name, shown to the people you share with                                                   |
| `url`                | string            | The address others use to get to this node                                                      |
| `replicas`           | list of strings   | Where encrypted replicas go; see [Backups](../backups/)                                         |
| `s3_keys`            | list of objects   | Keys that let others keep their replica here; see [Backups](../backups/#hold-a-friends-replica) |
| `bitwarden_register` | `true` or `false` | Lets one Bitwarden account be made; set it back to `false` after that                           |
| `forward_domain`     | string            | The domain whose apps may use forward auth; see [Sign-in for other apps](../sign-in/)           |
| `push_subject`       | string            | Contact sent to Web Push services, such as `mailto:you@example.org`; the default is `url`       |

`hs ctl init` writes `identity` and `private`. Do not change them.

## Environment

| Variable           | Used by          | Meaning                                 |
| ------------------ | ---------------- | --------------------------------------- |
| `HS_DIR`           | every command    | The data directory (as `--dir`)         |
| `HS_SERVER`        | every command    | The node to talk to (as `--server`)     |
| `HS_TOKEN`         | `hs login`       | The token, so `hs` does not ask         |
| `HS_RECOVERY_CODE` | `hs ctl restore` | The recovery code, so `hs` does not ask |

Sign-ins made with `hs login` are kept in
`$XDG_CONFIG_HOME/homeserver/cli.json`, which is mostly
`~/.config/homeserver/cli.json`.
