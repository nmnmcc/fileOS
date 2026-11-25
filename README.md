# homeserver

**English** | [简体中文](README.zh-CN.md)

> A personal server you own, for the data you keep and share.

homeserver is one program, `hs`. It does the work that people now get from a number of self-hosted apps put together: photo backup, file sync, chat, passwords, notes and calendars. It puts all of this in order by people, time, places and things. You do not have to be a server administrator. It connects straight to the homeservers of people you know.

> [!NOTE]
> **Status: experimental.** Version 0.1.0 is an MVP (a first, minimal version) that runs the core design from start to end; see the [changelog](CHANGELOG.md). Data formats may still change, so keep an export. Feedback and talk are welcome.

---

## Why

Cloud services give you sync, sharing and use from anywhere, but they keep your data. Your photos, chats, files, calendars and passwords are often with different companies. Any of them is able to make its prices higher, stop its service, change its terms or close your account.

Self-hosting is another way: Immich for photos, Vaultwarden for passwords, Matrix for chat. At present it needs the skills of an engineer. You get many containers and accounts, data that is not connected, and upgrades and debugging that never end.

AI assistants give one more reason. A useful assistant needs to know about your relations with people, your places and your work. That knowledge is private, and it is best kept on a machine you control.

**homeserver is a server that is yours. You do not need to understand it. It keeps your data and works only for you.**

|               | Cloud services                           | homeserver                          |
| ------------- | ---------------------------------------- | ----------------------------------- |
| Compute       | The platform's                           | **Yours**                           |
| Data          | Used from anywhere, kept by the platform | **Used from anywhere, kept by you** |
| Identity      | An account the platform is able to close | **Your own keys**                   |
| Relationships | Inside each platform                     | **Direct, person to person**        |
| Operations    | Done by the platform                     | **Automatic; no administration**    |

## What homeserver is

This part gives the design. The [changelog](CHANGELOG.md) says what is done at present.

- **Ordered by your life, not by apps.** A person's page has every chat, shared photo, file and event with that person. A day, place or project has everything that is part of it. Files, photos and chat are details of how it is made.
- **A device, not a service.** One program and one data directory, with nothing outside it that it depends on. Encryption and NAT traversal (getting through home routers) are automatic, so you need no public IP address and no domain. Upgrades are automatic, and when one goes wrong, the old version comes back. Nothing needs SSH.
- **Yours to take with you, for life.** Your identity is your own key; a domain is only another name for it. When you go to a different server, your contacts and groups come with you. At any time, you can export everything in open formats.
- **Federation between people.** Your homeserver talks straight to the homeservers of your friends. You can share albums, folders and documents between servers. They are updated as they change, and you can take back the sharing at any time. They are end-to-end encrypted.
- **Mutual aid.** Every homeserver keeps your data. It is also able to be a relay, mailbox and encrypted backup for people you trust. There is no blockchain and there are no tokens.
- **A home for your personal AI.** Your memory, what you like and your relations with people stay on your homeserver. Assistants that you give access get only the context you let them have, through standard interfaces such as MCP. Local models are the default; you say what leaves, if anything.
- **Works with the clients you have.** Standard protocols (WebDAV, CalDAV/CardDAV, the Bitwarden API and others) let you keep the clients you use now.
- **Everyday needs are in the core; special needs come through plugins.** The core does what about 80% of people use every day. Media libraries, home automation, code hosting and other such needs connect through plugins. Plugins run in a sandbox, get only the data you let them have, and sign in with your homeserver identity.
- **Your identity provider.** homeserver is the identity provider for you and your family, through standard protocols such as OpenID Connect. Plugin apps and other services that have standard sign-in use the same identity. You control access in one place, so you do not need a separate authentication gateway.

## Why federation alone is not enough

Matrix and Mastodon are federated, but most of their users are on a small number of large servers. For most people, federation means picking a server that is someone else's. Servers for one person do work, but they are at a disadvantage, and not many people run their own.

We see these causes, and homeserver has an answer for each:

| Why people come to be on a large server                                                                                                                                | What homeserver does                                                                                                                                            |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Running a server needs a domain, a public IP address, certificates, a database and work that never ends                                                                | It is a device: no public IP address or domain needed, automatic upgrades, nothing to administer                                                                |
| Identity is fixed to the server's domain (`@alice:matrix.org`, `@alice@mastodon.social`), so going away means starting again from nothing                              | Identity is your own key; a server is only the place where it is hosted. Contacts, groups and history go with you                                               |
| Cost gets bigger with the network: a small server in a large room has to copy and work out the whole state of the room; following more people means getting more media | Cost gets bigger only with your own data and relations. Each group has a home server that puts its events in order; others subscribe and do not copy everything |
| A small server has an empty local timeline, and its search and discovery are weak                                                                                      | Relations are person to person. You come to know people through people, not through the public timeline of a server                                             |
| Moderation and the work against spam are on each server's admin, and small servers are blocked more easily                                                             | Each person has their own rules, based on trust between people. No special admin is needed                                                                      |
| Apps put you on a large public server by default                                                                                                                       | The default and simplest way is your own homeserver                                                                                                             |

**We see success in how many people and families run their own homeserver, not in the number of accounts.**

## Principles

We give mechanisms and sensible defaults. **You make the rules; we do not make them for anyone.**

The principles are in order of priority. When two principles are in conflict, the lower number wins.

| #   | Principle            | Summary                                                                           |
| --- | -------------------- | --------------------------------------------------------------------------------- |
| 01  | Yours alone          | Your data, identity and relations are yours, and they can go with you at any time |
| 02  | Centered on people   | Ordered by people, time, places and things, not by type of data or by app         |
| 03  | Zero operations      | A homeserver is a device, not a service to administer                             |
| 04  | Long-lived           | Made to last for tens of years, also for what you leave behind (digital legacy)   |
| 05  | In control           | You can see who has access to what, and take it back                              |
| 06  | Memory you own       | You can see, change and delete your memory; it is not tied to any AI company      |
| 07  | Calm                 | The aim is a better life, not more time looking at a screen                       |
| 08  | Person to person     | The unit of federation is a person                                                |
| 09  | Personal tools       | Make small tools of your own from a description of one sentence                   |
| 10  | Open and replaceable | Standard protocols; nothing keeps you locked in                                   |
| 11  | Mutual aid           | Relay, mailbox and backup for people you trust, under rules you make              |

### Questions every feature must pass

1. Does it make it clearer that the user owns the data?
2. Is it ordered by people and life, and not by types of data?
3. Is a family member who is not technical able to use it, and to get it working again without help when something goes wrong?
4. Will the data still be fully readable in twenty years?
5. Is the user able to see what data it touched, and take that access back?
6. Does it take more of the user's time without making their life better?

The answer to questions 1–5 has to be "yes", and the answer to question 6 has to be "no". Features that do not pass wait until later.

## What homeserver will not do

| Not part of the project               | Reason                                                                                                                      |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Special features in the core          | Media libraries, home automation, code hosting and other such things connect through plugins. The core does everyday needs. |
| Features for companies                | LDAP, auditing, compliance, org charts. homeserver is for a person and their family.                                        |
| Sending mail out                      | Getting mail delivered needs work that never ends, and that is outside the project. At most, getting and forwarding mail.   |
| Our own AI models                     | We connect to local or cloud models, and we make the layer for memory and permissions.                                      |
| A general container platform          | homeserver is not a Portainer or an app store.                                                                              |
| Blockchain or tokens                  | Mutual aid is based on relations and trust, not payment.                                                                    |
| Anything that needs a central service | There may be official relays, but nothing may depend on them.                                                               |

## Design goals

These are goals for v1. The present release does not do all of them.

- Serves a family of 10 people on **1 CPU core and 1 GB RAM**, using under **150 MB** when it has no work
- **One binary (`hs`) and one data directory**; a copy of the directory is a backup
- From install to first use in **under 5 minutes**, with no config files
- A personal **multi-node cluster** (a box at home, a small VPS, a laptop) with one identity, so you are still online when the power goes off at home
- No global state that is copied everywhere: each group, album or document has a home server that puts its events in order, and others subscribe to it
- **A server for one person is first-class**: no feature works better on a large server than on a homeserver for one person or one family
- The resources it uses get bigger with your own data and relations, not with the size of the network
- Group encryption based on **MLS (RFC 9420)**

How it is made: in Rust, as one executable that has everything in it, and on Web standards (everything has a URL and talks HTTP, between homeservers too). The source of truth is signed logs that only get added to, and blobs named by the hash of their content. An SQLite index is made from them, and it can be made again at any time. Every node talks the same protocol, and browsers and phones are user agents of your homeserver. Sealed logs and blobs can be copied to a local directory, another `hs` (yours or a friend's) or any S3-compatible store. So sync, backup, moving to a new server and federation are one operation, and storage needs nothing but `hs`. Networking is HTTP/3 over QUIC (quinn and rustls), with our own NAT traversal, and homeservers are relays for each other. Each shared space has an ordered event log, encrypted with MLS. Plugins are WebAssembly components. An OpenID Connect provider is part of it. See [Architecture and technical route](docs/src/content/docs/en-us/dev/architecture.md) for the reasons and the measurements.

Slow Rust builds would keep out contributors who do not have powerful computers. So we see compile time as a feature: small crates, a small number of carefully picked dependencies, no heavy code generation, and a limit on build time that CI checks. An ordinary laptop should be enough to contribute.

## Roadmap

Not fixed.

- [ ] **Design**: manifesto and principles, white paper on the architecture, first drafts of the federation and mutual-aid protocols
- [ ] **Foundation**: single binary, storage, identity based on keys, importers (Google Takeout, iCloud, chat exports, password managers)
- [ ] **Life**: person pages, timeline, places and projects; files and photos
- [ ] **Together**: chat, sharing between servers, federation, mutual aid (relay, mailbox, encrypted backup)
- [ ] **Memory**: personal memory layer and assistant, MCP interface
- [ ] **Later**: calendar and contacts, documents that people make together, passwords, what you leave behind (digital legacy), personal tools

## Contributing

Open an issue to talk about a principle, to question a design decision or to give a use case. See [CONTRIBUTING.md](CONTRIBUTING.md) for how to get ready for development, and for the rules.

## License

homeserver is free software, under the [GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`).

**homeserver comes with ABSOLUTELY NO WARRANTY**, to the degree that the law lets. See sections 15 and 16 of the license.

As with the licenses of the Free Software Foundation, the aim is to keep the freedom of the people who use the software safe. Under the AGPL, anyone who runs homeserver for others, also as a hosted service, has to share their changes with those users. So no one, not even we, is able to make it into a closed platform.

Plugins that talk to homeserver only through its Plugin Interface may use any license, under the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md). The plugin SDK, `hs-plugin-api`, is under MIT OR Apache-2.0.

This software is based in part on the work of the Independent JPEG Group (through `jpeg-encoder`, used for thumbnails).
