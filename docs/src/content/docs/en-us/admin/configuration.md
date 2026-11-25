---
title: Configuration
description: The settings in config.json and how to change them.
---

The configuration is `config.json` in the data directory. Read and change it
with `hs ctl config`, while the node is stopped:

```sh
hs ctl config get
hs ctl config get url
hs ctl config set url https://hs.example.org
hs ctl config set replicas '["/mnt/backup/hs"]'
```

The value is read as JSON, or else taken as a string. Restart the node after
a change.

## Settings

| Key                  | Value             | What it does                                                                                    |
| -------------------- | ----------------- | ----------------------------------------------------------------------------------------------- |
| `name`               | string            | Your name, shown to the people you share with                                                   |
| `url`                | string            | The address others reach this node at; also the domain for `--tls acme`                         |
| `replicas`           | list of strings   | Where encrypted replicas go; see [Backups](../backups/)                                         |
| `s3_keys`            | list of objects   | Keys that let others keep their replica here; see [Backups](../backups/#hold-a-friends-replica) |
| `bitwarden_register` | `true` or `false` | Lets one Bitwarden account be created; set it back to `false` afterwards                        |
| `forward_domain`     | string            | The domain whose apps may use forward auth; see [Sign-in for other apps](../sign-in/)           |
| `push_subject`       | string            | Contact sent to Web Push services, such as `mailto:you@example.org`; defaults to `url`          |

`identity` and `private` are written by `hs ctl init` and must not be
changed.

## Environment

| Variable           | Used by          | Meaning                              |
| ------------------ | ---------------- | ------------------------------------ |
| `HS_DIR`           | every command    | The data directory (as `--dir`)      |
| `HS_SERVER`        | every command    | The node to talk to (as `--server`)  |
| `HS_TOKEN`         | `hs login`       | The token, instead of asking         |
| `HS_RECOVERY_CODE` | `hs ctl restore` | The recovery code, instead of asking |

Sign-ins made with `hs login` are kept in
`$XDG_CONFIG_HOME/homeserver/cli.json`, usually
`~/.config/homeserver/cli.json`.
