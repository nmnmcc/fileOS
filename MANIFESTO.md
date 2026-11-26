# The fileOS manifesto

**English** | [简体中文](MANIFESTO.zh-CN.md)

This is why fileOS is made, and the rules we keep to while making it. For what fileOS is and how to run it, see the [README](README.md).

## Why

Cloud services give you sync, sharing and use from anywhere, but they keep your data. Your photos, chats, files, calendars and passwords are often with different companies. Any of them is able to make its prices higher, stop its service, change its terms or close your account.

Self-hosting is another way: Immich for photos, Vaultwarden for passwords, Matrix for chat. At present it needs the skills of an engineer. You get many containers and accounts, data that is not connected, and upgrades and debugging that never end.

AI assistants give one more reason. A useful assistant needs to know about your relations with people, your places and your work. That knowledge is private, and it is best kept on a machine you control.

**fileOS runs on computers that are yours. You do not need to understand it. It keeps your data and works only for you.**

|               | Cloud services                           | fileOS                              |
| ------------- | ---------------------------------------- | ----------------------------------- |
| Compute       | The platform's                           | **Yours**                           |
| Data          | Used from anywhere, kept by the platform | **Used from anywhere, kept by you** |
| Identity      | An account the platform is able to close | **Your own keys**                   |
| Relationships | Inside each platform                     | **Direct, person to person**        |
| Operations    | Done by the platform                     | **Automatic; no administration**    |

## What fileOS will be

This part gives the design. The [README](README.md) says what is done at present.

- **Ordered by your life, not by apps.** A person's page has every chat, shared photo, file and event with that person. A day, place or project has everything that is part of it. Files, photos and chat are details of how it is made.
- **A device, not a service.** One program and one data directory, with nothing outside it that it depends on. Encryption and NAT traversal (getting through home routers) are automatic, so you need no public IP address and no domain. Upgrades are automatic, and when one goes wrong, the old version comes back. Nothing needs SSH.
- **Yours to take with you, for life.** Your identity is your own key; a domain is only another name for it. When you go to a different server, your contacts and groups come with you. At any time, you can export everything in open formats.
- **Federation between people.** Your nodes talk straight to the nodes of your friends. A group, an album or a document is a folder that your nodes and theirs keep together. They are updated as they change, and you can take back the sharing at any time. They are end-to-end encrypted.
- **Mutual aid.** Every node keeps your data. It is also able to be a relay, mailbox and encrypted backup for people you trust. There is no blockchain and there are no tokens.
- **A home for your personal AI.** Your memory, what you like and your relations with people stay on your nodes. Assistants that you give access get only the context you let them have, through standard interfaces such as MCP. Local models are the default; you say what leaves, if anything.
- **Works with the clients you have.** Standard protocols (WebDAV, CalDAV/CardDAV, the Bitwarden API and others) let you keep the clients you use now.
- **The core is only the file system; everything else is an app.** The apps for what about 80% of people use every day come with fileOS. Media libraries, home automation, code hosting and other such needs are apps too. Every app runs in a sandbox, gets only the folders you give it, and acts with its own key. Even the rules for access are a WebAssembly engine that you can change.
- **Your identity provider.** fileOS is the identity provider for you and your family, through standard protocols such as OpenID Connect. Apps and other services that have standard sign-in use the same identity. You control access in one place, so you do not need a separate authentication gateway.

## Why federation alone is not enough

Matrix and Mastodon are federated, but most of their users are on a small number of large servers. For most people, federation means picking a server that is someone else's. Servers for one person do work, but they are at a disadvantage, and not many people run their own.

We see these causes, and fileOS has an answer for each:

| Why people come to be on a large server                                                                                                                                | What fileOS does                                                                                                                                    |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Running a server needs a domain, a public IP address, certificates, a database and work that never ends                                                                | It is a device: no public IP address or domain needed, automatic upgrades, nothing to administer                                                    |
| Identity is fixed to the server's domain (`@alice:matrix.org`, `@alice@mastodon.social`), so going away means starting again from nothing                              | Identity is your own key; a server is only the place where it is hosted. Contacts, groups and history go with you                                   |
| Cost gets bigger with the network: a small server in a large room has to copy and work out the whole state of the room; following more people means getting more media | Cost gets bigger only with your own data and relations. Each group is a folder with one node that puts it in order; others keep only what they need |
| A small server has an empty local timeline, and its search and discovery are weak                                                                                      | Relations are person to person. You come to know people through people, not through the public timeline of a server                                 |
| Moderation and the work against spam are on each server's admin, and small servers are blocked more easily                                                             | Each person has their own rules, based on trust between people. No special admin is needed                                                          |
| Apps put you on a large public server by default                                                                                                                       | The default and simplest way is your own node                                                                                                       |

**We see success in how many people and families run their own nodes, not in the number of accounts.**

## Principles

We give mechanisms and sensible defaults. **You make the rules; we do not make them for anyone.**

The principles are in order of priority. When two principles are in conflict, the lower number wins.

| #   | Principle            | Summary                                                                           |
| --- | -------------------- | --------------------------------------------------------------------------------- |
| 01  | Yours alone          | Your data, identity and relations are yours, and they can go with you at any time |
| 02  | Centered on people   | Ordered by people, time, places and things, not by type of data or by app         |
| 03  | Zero operations      | A node is a device, not a service to administer                                   |
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

## What fileOS will not do

| Not part of the project               | Reason                                                                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Features in the core                  | The core is only the file system. Media libraries, home automation, code hosting and everything else are apps.            |
| Features for companies                | LDAP, auditing, compliance, org charts. fileOS is for a person and their family.                                          |
| Sending mail out                      | Getting mail delivered needs work that never ends, and that is outside the project. At most, getting and forwarding mail. |
| Our own AI models                     | We connect to local or cloud models, and we make the layer for memory and permissions.                                    |
| A general container platform          | Apps are WebAssembly components in a sandbox, not containers. fileOS is not a Portainer.                                  |
| Blockchain or tokens                  | Mutual aid is based on relations and trust, not payment.                                                                  |
| Anything that needs a central service | There may be official relays, but nothing may depend on them.                                                             |

## Design goals

These are goals for v1. The present release does not do all of them.

- Serves a family of 10 people on **1 CPU core and 1 GB RAM**, using under **150 MB** when it has no work
- **One program (`fos`) and one data directory**; a copy of the directory is a backup
- From install to first use in **under 5 minutes**, with no config files
- A personal **multi-node cluster** (a box at home, a small VPS, a laptop) with one identity, so you are still online when the power goes off at home
- No global state that is copied everywhere: each group, album or document is a folder with one node that puts it in order, and other nodes follow it
- **A server for one person is first-class**: no feature works better on a large server than on a node for one person or one family
- The resources it uses get bigger with your own data and relations, not with the size of the network
- Group encryption based on **MLS (RFC 9420)**

How it is made: in Rust. The core is only a file system: folders are signed logs that only get added to, and file contents are blobs named by their hash. Every rule for access is an engine, and every feature is an app; both are WebAssembly components that run in [wasmtime](https://wasmtime.dev). Nodes talk to each other over [libp2p](https://libp2p.io), with relays and hole punching, so no public IP address is needed. Sync, sharing, backup and moving to a new node are one operation: a node agrees to keep a folder. Folders are encrypted with MLS. See [Architecture](docs/src/content/docs/en-us/dev/fileos.md) for the reasons.

Slow Rust builds would keep out contributors who do not have powerful computers. So we see compile time as a feature: small crates, a small number of carefully picked dependencies, no heavy code generation, and a limit on build time that CI checks. An ordinary laptop should be enough to contribute.

## Why the AGPL

As with the licenses of the Free Software Foundation, the aim is to keep the freedom of the people who use the software safe. Under the AGPL, anyone who runs fileOS for others, also as a hosted service, has to share their changes with those users. So no one, not even we, is able to make it into a closed platform.

## Roadmap

Not fixed.

- [ ] **Design**: manifesto and principles, white paper on the architecture, first drafts of the federation and mutual-aid protocols
- [ ] **Foundation**: single binary, storage, identity based on keys, importers (Google Takeout, iCloud, chat exports, password managers)
- [ ] **Life**: person pages, timeline, places and projects; files and photos
- [ ] **Together**: chat, sharing between servers, federation, mutual aid (relay, mailbox, encrypted backup)
- [ ] **Memory**: personal memory layer and assistant, MCP interface
- [ ] **Later**: calendar and contacts, documents that people make together, passwords, what you leave behind (digital legacy), personal tools
