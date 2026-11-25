# homeserver

**English** | [简体中文](README.zh-CN.md)

> Your digital life, on a server you own.

homeserver is a personal server you own. One program, `hs`, replaces the pile of self-hosted apps most people run today (photo backup, file sync, chat, passwords, notes, calendars) and puts them together around your life. It needs no server administration, and it connects you directly to the people you care about.

> [!NOTE]
> **Status: design phase.** There is no code to run yet. This repository currently holds the vision, principles and design documents. Feedback and discussion are welcome.

---

## Why

The cloud brought sync, sharing and access from anywhere, and in exchange we handed over our lives. Photos live with one company, chats with another, files, calendars and passwords somewhere else. Each of them can raise prices, shut down, change the rules or lock your account, and each of them is working on how to keep you scrolling a little longer.

Self-hosting proved there is another way. People got their photos back with Immich, their passwords with Vaultwarden, their conversations with Matrix. But today that path belongs to a few engineers: dozens of containers, dozens of accounts, data that doesn't connect, and endless upgrades and debugging.

AI makes this urgent. A truly useful assistant needs to know your whole life: who you are close to, where you have been, what you are working on. That context is too private to hand to any platform. It belongs in your own home.

**So we are building homeserver: a server that belongs to you, that you don't have to understand, that keeps your digital life and works only for you.**

|               | Cloud era                             | homeserver                        |
| ------------- | ------------------------------------- | --------------------------------- |
| Compute       | The platform's                        | **Yours**                         |
| Data          | Everywhere, owned by the platform     | **Everywhere, owned by you**      |
| Identity      | A platform account that can be banned | **Your own keys, for life**       |
| Relationships | Locked inside platforms               | **Person to person**              |
| Operations    | Handled by the platform               | **No need to understand servers** |

## What homeserver is

- **Organized around your life, not around apps.** Open a person and see every chat, shared photo, file and event with them. Open a day, a place or a project and see everything that belongs to it. Files, photos and chat are implementation details.
- **An appliance, not a service.** One program and one data folder, with no external dependencies. TLS certificates and NAT traversal are automatic, so it works without a public IP. Upgrades are automatic and roll back on failure. Nothing requires SSH.
- **Yours for life.** Identity is based on your own keys, and a domain is only an alias. Move servers and your contacts and groups follow you. Export everything at any time in open formats.
- **Federated between people.** Your homeserver talks directly to your friends' homeservers. Share albums, folders and documents across servers, live, and revoke access at any time. End-to-end encrypted.
- **Neighbors help each other.** Every homeserver is both a home for your data and a relay, mailbox and encrypted backup for the people you trust. No blockchain, no tokens; just the ordinary give and take between people.
- **The home of your personal AI.** Your memory, preferences and relationships stay on your homeserver. Assistants you authorize get exactly the context you allow, through standard interfaces such as MCP. Local models by default; you decide what, if anything, leaves home.
- **Works with what you already use.** Standard protocols (WebDAV, CalDAV/CardDAV, the Bitwarden API and more) let you keep your favorite clients.
- **Everyday needs built in, specialist needs through plugins.** The core does only what 80% of people use every day. Specialist needs such as media libraries, home automation and code hosting connect through plugins. Plugins run in a sandbox, can only reach the data you allow, and sign in with your homeserver identity.
- **Your personal sign-in center.** homeserver is also the identity provider for you and your family, through standard protocols such as OpenID Connect. Apps connected through plugins, and other services that support standard sign-in, all use the same identity, with access managed in one place. No more authentication gateways to maintain.

## Why federation alone is not enough

Matrix and Mastodon are federated, yet most people still gather on a few large servers. For most users, federation only means choosing someone else's server. A server for one person does run, but it is at a disadvantage everywhere. A world where everyone has their own server never arrived.

We think these are the reasons. homeserver is designed against each one:

| Why people end up on a large server                                                                                                                                        | What homeserver does                                                                                                                   |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Running a server takes a domain, a public IP, certificates, a database and ongoing upkeep                                                                                  | It is an appliance: no public IP or domain needed, automatic upgrades, nothing to administer                                           |
| Identity is tied to the server's domain (`@alice:matrix.org`, `@alice@mastodon.social`), so leaving means starting over                                                    | Identity is your own key, and a server is only where it lives. Your contacts, groups and history move with you                         |
| Cost grows with the whole network: a small server joining a large room must replicate and resolve that room's whole state; following more people means fetching more media | Cost grows with your own life. Each group has a home server that orders its events; others subscribe instead of replicating everything |
| A small server is lonely: an empty local timeline, weak search and discovery                                                                                               | Relationships are person to person. You meet people through people, not through a server's public timeline                             |
| Moderation and spam are the work of each server's admin, and small servers are blocked more easily                                                                         | Rules belong to each person and rest on relationships of trust. No dedicated server admin is needed                                    |
| Apps sign you up on a large public server by default                                                                                                                       | The default path is a homeserver of your own, and it is the easiest one                                                                |

**We measure success by how many people and families run their own homeserver, not by how many accounts exist.**

## Principles

We provide mechanisms and sensible defaults. **You decide the rules. We don't make decisions for anyone.**

Ranked by priority. When two principles conflict, the lower number wins.

| #   | Principle            | In one line                                                                            |
| --- | -------------------- | -------------------------------------------------------------------------------------- |
| 01  | Yours alone          | Your data, identity and relationships belong to you and can leave with you at any time |
| 02  | Centered on people   | Organized by people, time, places and things, not by data type or app                  |
| 03  | Zero operations      | A homeserver is a device, not a service to administer                                  |
| 04  | For a lifetime       | Designed for decades, including digital legacy                                         |
| 05  | In control           | You always know who can see what, and you can take it back                             |
| 06  | It remembers you     | Your memory is yours to view, edit and delete, and is not tied to any AI vendor        |
| 07  | Calm                 | The goal is a better life, not more screen time                                        |
| 08  | Person to person     | The unit of federation is a person                                                     |
| 09  | Shaped like you      | Make your own small tools from a single sentence                                       |
| 10  | Open and replaceable | Standard protocols; no lock-in                                                         |
| 11  | Mutual aid           | Relay, mailbox and backup for people you trust, under rules you choose                 |

### Every feature must pass these questions

1. Does it make the user feel more strongly that _this is mine_?
2. Is it organized around people and life, or around data types?
3. Can a non-technical family member use it, and recover on their own when something breaks?
4. Will this data still be fully readable in twenty years?
5. Can the user see what data it touched, and take that access back?
6. Does it take more of the user's time without making their life better?

The first five must be answered "yes" and the last one "no". Features that fail wait.

## What homeserver will not do

| We won't build                        | Because                                                                                                                                                                        |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Built-in specialist features          | Media libraries, home automation, code hosting and the like stay out of the core and connect through plugins. The core focuses on the everyday needs that 80% of people share. |
| Enterprise features                   | LDAP, auditing, compliance, org charts. homeserver serves a person and their family.                                                                                           |
| Outbound mail delivery                | Deliverability is a bottomless pit. At most, receiving and forwarding.                                                                                                         |
| Our own AI models                     | We connect to local or cloud models and build the memory and permission layer.                                                                                                 |
| A general container platform          | homeserver is not another Portainer or app store.                                                                                                                              |
| Blockchain or tokens                  | Mutual aid runs on relationships and trust, not money.                                                                                                                         |
| Anything that needs a central service | Official relays may exist, but nothing may stop working without them.                                                                                                          |

## Design goals

These are targets for v1, not claims about current software.

- Runs comfortably for a 10-person household on **1 CPU core and 1 GB RAM**, idling under **150 MB**
- **One binary (`hs`) and one data directory**: back up by copying the folder
- Install to first use in **under 5 minutes**, with no config files
- A personal **multi-node cluster** (home box + small VPS + laptop) acting as one identity, so a power cut at home doesn't take you offline
- No global replicated state: each group, album or document has a home server that orders its events, and others subscribe to it
- **A server for one person is first-class**: no feature works better on a large server than on the homeserver of one person or one family
- Resource use grows with your own data and relationships, not with the size of the network
- Group encryption based on **MLS (RFC 9420)**

Technical route: Rust, shipped as one executable with everything inside, built deeply on Web standards (everything has a URL and speaks HTTP, homeservers included); signed, append-only logs and content-addressed blobs as the truth, with an SQLite index derived from them that can always be rebuilt; one protocol everywhere, with browsers and phones as user agents of your homeserver; sealed logs and blobs copied to a local directory, another `hs` (yours or a friend's) or any S3-compatible cloud, so sync, backup, migration and federation are one operation and nothing but `hs` is needed to store; HTTP/3 over QUIC (quinn and rustls) with our own NAT traversal, where homeservers relay for each other; an ordered event log per shared space, encrypted with MLS; WebAssembly components for plugins; and a built-in OpenID Connect provider. See [Architecture and technical route](docs/src/content/docs/en-us/architecture.md) for the reasons and the measurements.

Rust can be slow to compile, and that would shut out anyone without a powerful computer. We treat compile time as a feature: small crates, few and carefully chosen dependencies, no heavy code generation, and a build-time budget enforced in CI. An ordinary laptop should be enough to contribute.

## Roadmap

Tentative, and subject to change.

- [ ] **Design**: manifesto and principles, architecture white paper, federation and mutual-aid protocol drafts
- [ ] **Foundation**: single binary, storage, key-based identity, importers (Google Takeout, iCloud, chat exports, password managers)
- [ ] **Life**: person pages, life timeline, places and projects; files and photos
- [ ] **Together**: chat, cross-server sharing, federation, mutual aid (relay, mailbox, encrypted backup)
- [ ] **Memory**: personal memory layer and assistant, MCP interface
- [ ] **Later**: calendar and contacts, collaborative docs, passwords, digital legacy, personal tools

## Contributing

The project is at the stage where ideas matter most. Open an issue to discuss the principles, challenge a design decision or propose a use case. Contribution guidelines will follow once there is code.

## License

homeserver is free software, licensed under the [GNU Affero General Public License v3.0 only](LICENSE) (`AGPL-3.0-only`).

**homeserver is free software and comes with ABSOLUTELY NO WARRANTY**, to the extent permitted by applicable law. See sections 15 and 16 of the license.

Like the Free Software Foundation, we believe software should defend the freedom of the people who use it. The AGPL guarantees that anyone who runs homeserver for others, including as a hosted service, must share their changes with those users. homeserver can never be turned into a closed platform, by us or by anyone else.

Plugins that talk to homeserver only through its Plugin Interface may use any license, under the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md). Your own small tools shouldn't be bound by our license. The plugin SDK, `hs-plugin-api`, is licensed under MIT OR Apache-2.0.
