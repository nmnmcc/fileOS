# homeserver

**English** | [简体中文](README.zh-CN.md)

> A personal server you own, for the data you keep and share.

homeserver is one program, `hs`, that covers what people usually assemble from several self-hosted apps: photo backup, file sync, chat, passwords, notes, calendars. It organizes them around people, time, places and things, needs no server administration, and connects directly to the homeservers of people you know.

> [!NOTE]
> **Status: experimental.** Version 0.1.0 is an MVP that runs the core design end to end; see the [changelog](CHANGELOG.md). Data formats may still change, so keep an export. Feedback and discussion are welcome.

---

## Why

Cloud services provide sync, sharing and access from anywhere, but they keep your data. Photos, chats, files, calendars and passwords usually sit with different companies, each of which can change prices, shut down, change its terms or close your account.

Self-hosting is an alternative: Immich for photos, Vaultwarden for passwords, Matrix for chat. It currently takes engineering skill: many containers and accounts, data that is not connected, and ongoing upgrades and debugging.

AI assistants add a reason. A useful assistant needs context about your relationships, places and work. That context is private and should stay on a machine you control.

**homeserver is a server that belongs to you, does not require you to understand it, keeps your data and works only for you.**

|               | Cloud services                           | homeserver                          |
| ------------- | ---------------------------------------- | ----------------------------------- |
| Compute       | The platform's                           | **Yours**                           |
| Data          | Available anywhere, held by the platform | **Available anywhere, held by you** |
| Identity      | An account the platform can close        | **Your own keys**                   |
| Relationships | Inside each platform                     | **Direct, person to person**        |
| Operations    | Done by the platform                     | **Automatic; no administration**    |

## What homeserver is

This describes the design; the [changelog](CHANGELOG.md) lists what is implemented.

- **Organized around your life, not apps.** A person's page shows every chat, shared photo, file and event with them. A day, place or project shows everything that belongs to it. Files, photos and chat are implementation details.
- **An appliance, not a service.** One program and one data directory, with no external dependencies. Encryption and NAT traversal are automatic, so no public IP or domain is needed. Upgrades are automatic and roll back on failure. Nothing requires SSH.
- **Portable for life.** Identity is your own key; a domain is only an alias. When you move servers, your contacts and groups follow. You can export everything in open formats at any time.
- **Federated between people.** Your homeserver talks directly to your friends' homeservers. Albums, folders and documents can be shared across servers, updated live and revoked at any time. End-to-end encrypted.
- **Mutual aid.** Every homeserver stores your data and can act as a relay, mailbox and encrypted backup for people you trust. No blockchain or tokens.
- **A home for your personal AI.** Your memory, preferences and relationships stay on your homeserver. Assistants you authorize get only the context you allow, through standard interfaces such as MCP. Local models by default; you decide what, if anything, leaves.
- **Compatible with existing clients.** Standard protocols (WebDAV, CalDAV/CardDAV, the Bitwarden API and others) let you keep the clients you use.
- **Everyday needs built in, specialist needs through plugins.** The core covers what about 80% of people use every day. Media libraries, home automation, code hosting and similar needs connect through plugins. Plugins run in a sandbox, reach only the data you allow, and sign in with your homeserver identity.
- **Your identity provider.** homeserver is the identity provider for you and your family, through standard protocols such as OpenID Connect. Plugin apps and other services that support standard sign-in use the same identity, and access is managed in one place, so no separate authentication gateway is needed.

## Why federation alone is not enough

Matrix and Mastodon are federated, yet most users are on a few large servers. For most people, federation means choosing someone else's server. Single-person servers work but are at a disadvantage, and few people run their own.

We see these causes, and homeserver addresses each:

| Why people end up on a large server                                                                                                                            | What homeserver does                                                                                                                                     |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Running a server needs a domain, a public IP, certificates, a database and ongoing maintenance                                                                 | It is an appliance: no public IP or domain needed, automatic upgrades, nothing to administer                                                             |
| Identity is tied to the server's domain (`@alice:matrix.org`, `@alice@mastodon.social`), so leaving means starting over                                        | Identity is your own key; a server is only where it is hosted. Contacts, groups and history move with you                                                |
| Cost grows with the network: a small server in a large room must replicate and resolve the room's whole state; following more people means fetching more media | Cost grows with your own data and relationships. Each group has a home server that orders its events; others subscribe instead of replicating everything |
| A small server has an empty local timeline and weak search and discovery                                                                                       | Relationships are person to person. You meet people through people, not through a server's public timeline                                               |
| Moderation and spam handling fall on each server's admin, and small servers are blocked more easily                                                            | Rules belong to each person and are based on trust between people. No dedicated admin is needed                                                          |
| Apps register you on a large public server by default                                                                                                          | The default and easiest path is your own homeserver                                                                                                      |

**Success is measured by how many people and families run their own homeserver, not by the number of accounts.**

## Principles

We provide mechanisms and sensible defaults. **You decide the rules; we do not decide for anyone.**

Ranked by priority. When two principles conflict, the lower number wins.

| #   | Principle            | Summary                                                                                |
| --- | -------------------- | -------------------------------------------------------------------------------------- |
| 01  | Yours alone          | Your data, identity and relationships belong to you and can leave with you at any time |
| 02  | Centered on people   | Organized by people, time, places and things, not by data type or app                  |
| 03  | Zero operations      | A homeserver is a device, not a service to administer                                  |
| 04  | Long-lived           | Designed to last decades, including digital legacy                                     |
| 05  | In control           | You can see who has access to what, and revoke it                                      |
| 06  | Memory you own       | You can view, edit and delete your memory; it is not tied to any AI vendor             |
| 07  | Calm                 | The aim is a better life, not more screen time                                         |
| 08  | Person to person     | The unit of federation is a person                                                     |
| 09  | Personal tools       | Make small tools of your own from a one-sentence description                           |
| 10  | Open and replaceable | Standard protocols; no lock-in                                                         |
| 11  | Mutual aid           | Relay, mailbox and backup for people you trust, under rules you choose                 |

### Questions every feature must pass

1. Does it make the user's ownership clearer?
2. Is it organized around people and life rather than data types?
3. Can a non-technical family member use it, and recover alone when something breaks?
4. Will the data still be fully readable in twenty years?
5. Can the user see what data it touched, and revoke that access?
6. Does it take more of the user's time without improving their life?

Questions 1–5 must be answered "yes" and question 6 "no". Features that fail are deferred.

## What homeserver will not do

| Out of scope                          | Reason                                                                                                               |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Built-in specialist features          | Media libraries, home automation, code hosting and the like connect through plugins. The core covers everyday needs. |
| Enterprise features                   | LDAP, auditing, compliance, org charts. homeserver serves a person and their family.                                 |
| Outbound mail delivery                | Deliverability requires ongoing work outside the project's scope. At most, receiving and forwarding.                 |
| Our own AI models                     | We connect to local or cloud models and build the memory and permission layer.                                       |
| A general container platform          | homeserver is not a Portainer or an app store.                                                                       |
| Blockchain or tokens                  | Mutual aid is based on relationships and trust, not payment.                                                         |
| Anything that needs a central service | Official relays may exist, but nothing may depend on them.                                                           |

## Design goals

These are targets for v1, not properties of the current release.

- Serves a 10-person household on **1 CPU core and 1 GB RAM**, idling under **150 MB**
- **One binary (`hs`) and one data directory**; copying the directory is a backup
- From install to first use in **under 5 minutes**, without config files
- A personal **multi-node cluster** (home box, small VPS, laptop) acting as one identity, so a power cut at home does not take you offline
- No global replicated state: each group, album or document has a home server that orders its events, and others subscribe to it
- **A single-person server is first-class**: no feature works better on a large server than on a homeserver for one person or one family
- Resource use grows with your own data and relationships, not with the size of the network
- Group encryption based on **MLS (RFC 9420)**

Technical approach: Rust, shipped as one self-contained executable and built on Web standards (everything has a URL and speaks HTTP, between homeservers too). Signed, append-only logs and content-addressed blobs are the source of truth; an SQLite index is derived from them and can be rebuilt at any time. Every node speaks the same protocol, and browsers and phones are user agents of your homeserver. Sealed logs and blobs can be copied to a local directory, another `hs` (yours or a friend's) or any S3-compatible store, so sync, backup, migration and federation are one operation and storage needs nothing but `hs`. Networking is HTTP/3 over QUIC (quinn and rustls) with our own NAT traversal, and homeservers relay for each other. Each shared space has an ordered event log encrypted with MLS. Plugins are WebAssembly components. An OpenID Connect provider is built in. See [Architecture and technical route](docs/src/content/docs/en-us/dev/architecture.md) for reasons and measurements.

Slow Rust builds would exclude contributors without powerful computers, so compile time is treated as a feature: small crates, few and carefully chosen dependencies, no heavy code generation, and a build-time budget enforced in CI. An ordinary laptop should be enough to contribute.

## Roadmap

Tentative.

- [ ] **Design**: manifesto and principles, architecture white paper, federation and mutual-aid protocol drafts
- [ ] **Foundation**: single binary, storage, key-based identity, importers (Google Takeout, iCloud, chat exports, password managers)
- [ ] **Life**: person pages, timeline, places and projects; files and photos
- [ ] **Together**: chat, cross-server sharing, federation, mutual aid (relay, mailbox, encrypted backup)
- [ ] **Memory**: personal memory layer and assistant, MCP interface
- [ ] **Later**: calendar and contacts, collaborative documents, passwords, digital legacy, personal tools

## Contributing

Open an issue to discuss a principle, question a design decision or describe a use case. See [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup and rules.

## License

homeserver is free software, licensed under the [GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`).

**homeserver comes with ABSOLUTELY NO WARRANTY**, to the extent permitted by applicable law. See sections 15 and 16 of the license.

As with the Free Software Foundation's licenses, the aim is to protect the freedom of the people who use the software. The AGPL requires anyone who runs homeserver for others, including as a hosted service, to share their changes with those users, so neither we nor anyone else can turn it into a closed platform.

Plugins that talk to homeserver only through its Plugin Interface may use any license, under the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md). The plugin SDK, `hs-plugin-api`, is licensed under MIT OR Apache-2.0.

This software is based in part on the work of the Independent JPEG Group (through `jpeg-encoder`, used for thumbnails).
