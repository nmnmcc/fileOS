# homeserver

**English** | [简体中文](README.zh-CN.md)

> A personal server you own, for the data you keep and share.

homeserver is one program, `hs`, and one data directory. It keeps your photos, files, chat, passwords, notes and calendars, and puts them in order by people, time, places and things. It talks straight to the homeservers of people you know. You do not need to be a server administrator.

> [!NOTE]
> **Status: experimental.** Version 0.1.0 is an MVP (a first, minimal version). Data formats may still change, so keep an export. See the [changelog](CHANGELOG.md) for what it does.

## Quick start

```sh
cargo build --release --package hs   # or: devenv build outputs.hs
hs ctl init --name "Ada"             # write down the recovery code it prints
hs ctl serve                         # http://127.0.0.1:8080
```

See [Installation](docs/src/content/docs/en-us/admin/install.md) and [Running a node](docs/src/content/docs/en-us/admin/running.md) for more.

## Documentation

- [User manual](docs/src/content/docs/en-us/user/index.md): signing in, your data, sharing, apps and assistants.
- [Administration manual](docs/src/content/docs/en-us/admin/index.md): installing, running, configuration, backups.
- [Developer manual](docs/src/content/docs/en-us/dev/index.md): contributing, the REST API, the [architecture](docs/src/content/docs/en-us/dev/architecture.md).
- [Manifesto](MANIFESTO.md): why homeserver is made, its principles, what it will not do, and the roadmap.

## Contributing

Open an issue to talk about a principle, question a design decision or give a use case. See [CONTRIBUTING.md](CONTRIBUTING.md) for how to get ready for development, and for the rules.

## License

homeserver is free software under the [GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`), and comes with **ABSOLUTELY NO WARRANTY**, to the degree that the law lets. Plugins that talk to homeserver only through its Plugin Interface may use any license, under the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md). The plugin SDK, `hs-plugin-api`, is under MIT OR Apache-2.0.

This software is based in part on the work of the Independent JPEG Group (through `jpeg-encoder`, used for thumbnails).
