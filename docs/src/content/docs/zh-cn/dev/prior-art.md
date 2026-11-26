---
title: 值得学习的项目
description: 在自己动手之前要研究的已有项目，以及从每个项目中学什么。
---

fileOS 自己编写核心，使它保持轻量、易于扩展，并符合我们的目标。
为此需要研究已有的工作：软件、协议、论文、文章，以及未能成功的项目。
本页按领域列出这些项目，写明各自值得学习之处；必要时也写明我们的不同做法。

**学习，而不是照搬。** 我们读代码和论文是为了理解思路。复制任何代码之前，
先确认它的许可证与 AGPL-3.0-only 兼容。GPL-2.0-only 的代码不得复制。

新增条目时，写上链接、值得学什么，以及我们的不同之处。

## 同一愿景的前人尝试

已经有人尝试让每个人拥有自己的服务器。需要弄清它们为什么都没能普及到普通人。

- **[FreedomBox](https://freedombox.org/)**：基于 Debian 的个人服务器，
  源自 Eben Moglen 的演讲 "Freedom in the Cloud"。它说明了在家里运行服务器难在哪里。
- **[Solid](https://solidproject.org/)**：个人数据 Pod，应用需要申请访问权限。
  学习它的数据模型、访问控制，以及它为什么推广缓慢。
- **[Sandstorm](https://sandstorm.io/)**：grain、基于能力的分享和沙箱化应用。
  学习它的设计，也学习它为什么难以持续。
- **[Cozy Cloud](https://github.com/cozy)**：所有应用共享同一个数据存储的个人云。
- **[Urbit](https://urbit.org/)**：拥有自己身份体系和网络的个人服务器。
  学习它的设计范围，以及随之而来的复杂度。_我们的不同：_ 没有稀缺身份，也没有代币。
- **[Upspin](https://upspin.io/)**：Google Go 团队设计的个人文件全局命名空间。
- **[Mail-in-a-Box](https://mailinabox.email/)**：不需要专业知识就能搭起来的自托管邮件服务器。
- **Helm**：以硬件形式销售的个人邮件服务器，2022 年停止服务。
  说明了依赖硬件的商业模式存在的风险。
- **Synology DSM** 和 **Apple iCloud**：闭源，但它们决定了普通人对易用性的预期。

## 思想与著作

- **[本地优先软件](https://www.inkandswitch.com/local-first/)**（Ink & Switch）：
  清楚地阐述了我们认同的理念。
- **[Freedom in the Cloud](https://www.softwarefreedom.org/events/2010/isoc-ny/)**
  （Eben Moglen，2010）：为个人服务器提出的论证。
- **《Tools for Conviviality》**（Ivan Illich）：工具应当服务于人，而不是让人依赖它。
- **《Governing the Commons》**（Elinor Ostrom，中译《公共事物的治理之道》）：
  社区如何在没有中央权威、也没有市场的情况下管理共享资源。是互助机制的理论背景。
- **[Calm technology](https://calmtech.com/)**：平时退居幕后、需要时才出现的技术。
- **《A Pattern Language》**（Christopher Alexander，中译《建筑模式语言》）：
  把设计变成人人都能使用的共同词汇。
- **[IndieWeb](https://indieweb.org/)**：拥有自己的数据，在自己的网站发布，再同步到别处（POSSE）。
  以及 Webmention 和 Micropub。
- **[Small Technology](https://small-tech.org/)**：为一个人而不是为规模设计的技术。
- **[Data Transfer Initiative](https://dtinit.org/)**：在不同服务之间迁移个人数据，
  以及各大平台的导出格式。

## 网络与 NAT 穿透

- **[iroh](https://github.com/n0-computer/iroh)**：按公钥而不是 IP 地址建立连接。
  我们决定自己编写网络层，不依赖它，但它和 `hs-net` 要做的事最接近，值得深入研究：
  - 打洞，以及直连失败时自动回退到中继。
  - 基于 HTTPS 和 WebSocket 的中继协议，用于封锁 UDP 的网络。
  - 网络探测（`net_report`）和多条路径之间的选择。
  - 用签名 DNS 记录（pkarr）查找地址。
  - 中继的访问控制（`AccessControl` trait）。

  _我们的不同：_ 默认不依赖任何公司的中继和 DNS，依赖树也更小。
  实测一个测试构建引入了 231 个 crate。

- **[Tailscale](https://github.com/tailscale/tailscale)**：DERP 中继、`magicsock`，
  以及 [How NAT traversal works](https://tailscale.com/blog/how-nat-traversal-works)
  这篇文章，是这个主题很好的入门材料。_我们的不同：_ 没有中心化的协调服务器。
- **[quinn](https://github.com/quinn-rs/quinn)**：Rust 的 QUIC 实现。
  重点看它如何把协议状态机和 I/O 分开（`quinn-proto`）。
- **[WireGuard](https://www.wireguard.com/papers/wireguard.pdf)**：
  一个下午就能读完的协议。它说明一组固定的少量选择（Noise、不协商加密算法）能做到多少事。
- **[Noise Protocol Framework](https://noiseprotocol.org/noise.html)**：
  在密钥之间建立认证加密的握手模式。
- **[libp2p](https://libp2p.io/)**：circuit relay v2、DCUtR 打洞，
  以及高度通用的模块化设计带来的代价。_我们的不同：_ 更少的选项，更少的抽象。
- **[Syncthing](https://syncthing.net/)**：社区维护的中继池、全局和本地发现、
  由密钥派生的设备 ID。它是普通人日常使用的点对点软件，积累了多年经验。
- **[Yggdrasil](https://yggdrasil-network.github.io/)** 和
  **[cjdns](https://github.com/cjdelisle/cjdns)**：地址由公钥派生的网状网络。
- **[Reticulum](https://reticulum.network/)** 和
  **[Meshtastic](https://meshtastic.org/)**：通过无线电等低速链路联网，完全不需要互联网。

## 要读的标准

直接读规范本身，而不只是读实现它们的库。

- **QUIC**：[RFC 9000](https://www.rfc-editor.org/rfc/rfc9000)（传输）和
  [RFC 9001](https://www.rfc-editor.org/rfc/rfc9001)（TLS）。
- **NAT 穿透**：[ICE](https://www.rfc-editor.org/rfc/rfc8445)、
  [STUN](https://www.rfc-editor.org/rfc/rfc8489)、
  [TURN](https://www.rfc-editor.org/rfc/rfc8656)，以及用于端口映射的
  [PCP](https://www.rfc-editor.org/rfc/rfc6887)。
- **群组加密**：[MLS](https://www.rfc-editor.org/rfc/rfc9420)。
- **邮件、日历和联系人**：[JMAP](https://jmap.io/)、
  [CalDAV](https://www.rfc-editor.org/rfc/rfc4791) 和
  [CardDAV](https://www.rfc-editor.org/rfc/rfc6352)。
- **登录**：[WebAuthn](https://www.w3.org/TR/webauthn-3/)（通行密钥）、
  [OpenID Connect](https://openid.net/developers/specs/) 和
  [OAuth 2.1](https://oauth.net/2.1/)：fileOS 的 login App 作为身份提供方要实现的协议。

## 身份与信任

- **[pkarr](https://github.com/pubky/pkarr)**：用 Ed25519 密钥签名的 DNS 记录，
  发布在 Mainline DHT 上。不需要拥有域名，别人也能通过密钥找到你。
- **[Keybase](https://book.keybase.io/docs/teams/sigchain)**：每台设备独立的密钥、
  签名链（sigchain）和社交证明。说明了一个人的身份如何比任何一台设备都长久。
- **[Matrix 交叉签名](https://spec.matrix.org/latest/client-server-api/#cross-signing)**：
  验证设备和人，以及把这件事做得好用有多难。
- **[Secure Scuttlebutt](https://ssbc.github.io/scuttlebutt-protocol-guide/)**：
  身份就是密钥，社交关系决定你为别人存储和转发什么。它是"邻里互助"最接近的前身。
  也要学习它的局限：无法裁剪的只追加日志，以及新用户难以加入。
- **[Dark Crystal](https://darkcrystal.pw/)**：借助可信的朋友找回丢失的密钥（Shamir 秘密共享）。
- **[通行密钥](https://passkeys.dev/)**：不用密码登录，普通人已经在用。
- **[Kanidm](https://kanidm.com/)**：用 Rust 编写的身份管理服务器，支持 OpenID Connect 和通行密钥。
- **[Authelia](https://www.authelia.com/)** 和 **[Authentik](https://goauthentik.io/)**：
  自托管用户目前需要单独运行和维护的认证网关。fileOS 的 login App 用来替代它们。
- **[IndieAuth](https://indieauth.spec.indieweb.org/)**：用你自己的网址作为身份登录别的网站。

## 加密

- **[Signal 协议](https://signal.org/docs/)**：双棘轮（double ratchet）、X3DH 和 sealed sender。
  私密通讯的参考。
- **[OpenMLS](https://github.com/openmls/openmls)**：Rust 的 MLS 实现，
  用于家庭这样的加密群组。
- **[age](https://age-encryption.org/)**：简单的文件加密，密钥短小、明确。
- **[Magic Wormhole](https://magic-wormhole.readthedocs.io/)**：用 PAKE 通过一个短码完成配对。
  是添加好友或新设备的范本。
- **[CryptPad](https://cryptpad.org/)**：服务器无法读取内容的协作文档。

## 联邦与社交协议

已有的联邦协议有一个共同的教训：联邦本身并不能让每个人拥有自己的服务器。
大多数人最后仍然聚集在少数几个大服务器上。原因分析和我们的对策见[宣言](https://github.com/nmnmcc/homeserver/blob/main/MANIFESTO.zh-CN.md)的
"为什么只有联邦还不够"。

- **[Matrix](https://spec.matrix.org/)**：跨服务器的房间状态解析。尽管是联邦的，
  大多数用户仍在少数几个大服务器上。需要研究原因：身份绑定在服务器域名上；
  加入房间的每个服务器都要复制并解析整个房间的状态，所以小服务器加入大房间代价很高；
  运行服务器本身也需要持续投入。它的点对点实验（Pinecone）也值得一读。
- **[ActivityPub](https://www.w3.org/TR/activitypub/)** 和 Mastodon：
  使用最广的联邦协议，以服务器而不是以人为中心。身份绑定在实例上，迁移会丢掉过去的帖子；
  小实例活跃度低，审核和反垃圾的工作又落在管理员身上，于是人们聚集到大实例上。
- **[AT Protocol](https://atproto.com/)**：个人数据服务器（PDS）、签名仓库（Merkle 搜索树）
  和在不同托管方之间迁移。PDS 可以自托管，但大多数人的数据仍在官方 PDS 上，
  中继和 AppView 也主要由一家公司运营。_我们的不同：_ 不需要全局的中继或索引服务。
- **[Nostr](https://github.com/nostr-protocol/nips)**：密钥加上简单的中继，
  看一个极简协议能走多远。身份不属于任何服务器，这一点和 fileOS 一致。
- **[Briar](https://briarproject.org/)** 及其
  [Mailbox](https://briarproject.org/download-briar-mailbox/)：替离线的联系人暂存消息，
  正是我们的"信箱"角色。
- **[Spritely Goblins](https://spritely.institute/goblins/)** 和
  [OCapN](https://ocapn.org/)：跨网络的对象能力（object capabilities）。
- **[ForgeFed](https://forgefed.org/)**：代码托管的联邦协议，正在接入 Forgejo。

## 同步与本地优先的数据

- **[Automerge](https://automerge.org/)**、**[Yjs](https://yjs.dev/)** 和
  **[Loro](https://loro.dev/)**：用于协作文档的 CRDT。比较它们的数据格式、性能和编译成本。
- **[Willow](https://willowprotocol.org/)**：Earthstar 团队设计的同步协议，
  内置能力（capabilities）和真正的删除。
- **[cr-sqlite](https://github.com/vlcn-io/cr-sqlite)**：在 SQLite 里实现 CRDT。
- **[Git](https://git-scm.com/book/en/v2/Git-Internals-Plumbing-and-Porcelain)**：
  按内容寻址的历史记录，每天有数百万人用它同步。

## 存储、数据库与搜索

- **[SQLite](https://www.sqlite.org/)**：小巧、快速、可靠的嵌入式数据库，带 FTS5 和 R*Tree，
  文件格式承诺可读到 2050 年。我们的派生索引使用它。可参考[它的测试方法](https://www.sqlite.org/testing.html)。
- **[redb](https://github.com/cberner/redb)** 和
  **[fjall](https://github.com/fjall-rs/fjall)**：纯 Rust 的嵌入式键值存储。
- **[Tantivy](https://github.com/quickwit-oss/tantivy)** 和
  [SQLite FTS5](https://www.sqlite.org/fts5.html)：覆盖各类数据的全文搜索。
- **[Perkeep](https://perkeep.org/)**：按内容寻址、为一辈子设计的个人存储，
  和我们的目标非常接近。
- **[Tahoe-LAFS](https://tahoe-lafs.org/)**：在朋友之间存储数据，存储方无法读取内容。
  这是加密互助备份的范本。
- **[Garage](https://garagehq.deuxfleurs.fr/)**：用 Rust 编写的轻量 S3 存储，
  专为由朋友们运营、分布在几个地点的自托管场景设计。
- **[restic](https://restic.net/)** 和 **[BorgBackup](https://www.borgbackup.org/)**：
  备份的去重、加密和仓库格式。
- **[BLAKE3](https://github.com/BLAKE3-team/BLAKE3)** 和
  **[Bao](https://github.com/oconnor663/bao)**：快速哈希，以及按内容寻址数据的可验证流式传输。
- **[Litestream](https://litestream.io/)**：SQLite 的流式复制。
- **[DuckDB](https://duckdb.org/)** 和 **[DuckLake](https://ducklake.select/)**：
  嵌入式分析数据库，以及把表以 Parquet 文件存放在对象存储中、支持快照和时间旅行的格式。
  我们测量过它们，最终选择了 SQLite，见总体架构页面。
- **[duckdb-tantivy](https://github.com/alitrack/duckdb-tantivy)**：用 Rust 编写，
  在 DuckDB 中以 SQL 函数提供 BM25 搜索和中文分词。
- **[Turso](https://github.com/tursodatabase/turso)**：用 Rust 重写的 SQLite。
  我们测量过它，见总体架构页面。

## Web 平台

- **[Solid](https://solidproject.org/)**：Web 的发明者提出的、建立在 Web 标准上的个人数据存储。
  和我们的目标很接近；不同之处在于我们为每个空间使用有序日志，用密钥而不是基于域名的 WebID。
- **[HTTP 消息签名（RFC 9421）](https://www.rfc-editor.org/rfc/rfc9421)**
  和 **[Web Push（RFC 8030）](https://www.rfc-editor.org/rfc/rfc8030)**：
  我们在服务器之间和通知中使用的标准。
- **[WebTransport](https://developer.mozilla.org/zh-CN/docs/Web/API/WebTransport)**：
  面向浏览器的 QUIC，让浏览器可以凭证书哈希连接节点。
- **[WinterTC](https://wintertc.org/)** 和
  **[wasi:http](https://github.com/WebAssembly/wasi-http)**：服务器和浏览器上同一套请求与响应 API，
  是我们插件接口的范本。

## 点对点分享与激励

- **[BitTorrent](https://www.bittorrent.org/beps/bep_0003.html)**：以牙还牙（tit-for-tat），
  节点优先回报愿意付出的节点。一条简单的本地规则影响整个网络的行为。
- **[IPFS](https://docs.ipfs.tech/)**：大规模的内容寻址，以及一个教训：
  没有人主动固定（pin）的数据，就没有人保存。_我们的不同：_ 没有代币经济（Filecoin）。
- **[Hyphanet](https://www.hyphanet.org/)**（原 Freenet）及其朋友对朋友模式：只连接你信任的人。

## 零运维自托管

- **[Umbrel](https://umbrel.com/)**、**[YunoHost](https://yunohost.org/)** 和
  **[Cloudron](https://www.cloudron.io/)**：自托管应用商店，以及更新出问题时会发生什么。
- **[Home Assistant](https://www.home-assistant.io/)**：庞大的集成生态，
  以及普通人也能接受的升级体验。智能家居本身是专业需求，通过插件接入。
- **[Caddy](https://caddyserver.com/)**：自动获取和续期 TLS 证书。
- **[NixOS](https://nixos.org/)** 和 **[OSTree](https://ostreedev.github.io/ostree/)**：
  原子升级和回滚。
- **[Android A/B 更新](https://source.android.com/docs/core/ota/ab)**、
  **[RAUC](https://rauc.io/)** 和 **[Mender](https://mender.io/)**：
  更新无人看管的设备，且不会让设备无法启动。
- **[PocketBase](https://pocketbase.io/)**：一个文件加 SQLite 就是完整的后端。

## 可靠性与测试

- **[FoundationDB 仿真测试](https://apple.github.io/foundationdb/testing.html)**：
  在一个进程里确定性地运行整个分布式系统，任何故障都能重放。
- **[TigerBeetle](https://github.com/tigerbeetle/tigerbeetle)** 和
  [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)：
  确定性仿真、处处断言、极少依赖。
- **[Jepsen](https://jepsen.io/analyses)**：真实的分布式系统是怎么丢数据的，以及如何发现。
- **[Erlang/OTP](https://www.erlang.org/doc/system/design_principles.html)**：
  监督树、"let it crash"，以及能连续运行多年的系统。

## Rust 工程与编译时间

- **[Fast Rust Builds](https://matklad.github.io/2021/09/04/fast-rust-builds.html)**
  （matklad）：降低编译时间的实用方法。
- **[rust-analyzer](https://github.com/rust-lang/rust-analyzer)**：
  一个为快速构建和清晰边界而组织的大型 Rust 工作区。
- **[tokio](https://tokio.rs/)** 和 **[smol](https://github.com/smol-rs/smol)**：
  一大一小两种异步运行时的思路。
- **[hyper](https://hyper.rs/)**、**[axum](https://github.com/tokio-rs/axum)**
  和 **[rustls](https://github.com/rustls/rustls)**：Rust 的 HTTP 和 TLS。

## 插件、沙箱与隔离

- **[Wasmtime](https://wasmtime.dev/)** 和
  [Component Model](https://component-model.bytecodealliance.org/)（WIT）：
  带类型化接口的沙箱扩展。
- **[Extism](https://extism.org/)**：基于 WebAssembly 的插件系统。
- **[Zed 扩展](https://zed.dev/docs/extensions)**：一个 Rust 应用如何承载 WebAssembly 扩展。
- **[OpenBSD pledge 和 unveil](https://man.openbsd.org/pledge.2)**、
  **[Linux Landlock](https://landlock.io/)** 和
  **[Capsicum](https://www.cl.cam.ac.uk/research/security/capsicum/)**：
  程序主动放弃自己用不到的权限。
- **[Fuchsia 组件](https://fuchsia.dev/fuchsia-src/concepts/components/v2)**：
  从一开始就建立在能力之上的操作系统。

## 客户端、通知与无障碍

- **[UnifiedPush](https://unifiedpush.org/)** 和 **[ntfy](https://ntfy.sh/)**：
  不依赖 Google 或 Apple 的手机推送通知。
- **[Fluent](https://projectfluent.org/)** 和
  **[ICU4X](https://github.com/unicode-org/icu4x)**：翻译与本地化，都有 Rust 实现。
- **[WCAG](https://www.w3.org/WAI/standards-guidelines/wcag/)**：
  让家里每个人都能用，包括老人和残障人士。

## 本地 AI

- **[llama.cpp](https://github.com/ggml-org/llama.cpp)** 和
  **[whisper.cpp](https://github.com/ggml-org/whisper.cpp)**：
  在普通硬件上运行的语言和语音模型，不需要云。
- **Immich 的机器学习**：人脸识别和按语义搜索（CLIP），都在你自己的服务器上完成。

## 功能范围内的应用

这些项目已经覆盖了 fileOS 的某一部分。学习它们的功能和数据模型，而不是架构。

只收录 80% 的人日常都会用到的通用需求。影音媒体库、智能家居、代码托管这类专业需求不进核心，见下一节。

- **[Immich](https://immich.app/)** 和 **[PhotoPrism](https://www.photoprism.app/)**：照片和视频。
- **[Nextcloud](https://nextcloud.com/)**：文件、日历和联系人。
  也说明了应用之间缺乏隔离会带来什么后果。
- **[Stalwart](https://stalw.art/)**：用 Rust 编写的一体化邮件服务器（IMAP、JMAP、SMTP）。
- **[Radicale](https://radicale.org/)**：小巧的日历和联系人服务器。
- **[Vaultwarden](https://github.com/dani-garcia/vaultwarden)**：用 Rust 编写的密码管理服务器。
- **[Paperless-ngx](https://docs.paperless-ngx.com/)**：扫描文档管理。
- **[Actual Budget](https://actualbudget.org/)**：本地优先的个人记账。
- **[OwnTracks](https://owntracks.org/)**：私密的位置历史。
- **[Obsidian](https://obsidian.md/)** 和 **[Logseq](https://logseq.com/)**：
  以你拥有的纯文本文件保存的笔记。
- **[Datasette](https://datasette.io/)** 和
  **[Dogsheep](https://dogsheep.github.io/)**：把分散在各个服务中的个人数据汇集到一处，便于浏览和查询。
- **[Conduit](https://conduit.rs/)**：用 Rust 编写的轻量 Matrix 服务器。

## 通过插件接入的专业软件

专业需求不进核心，而是通过插件接入。研究这些软件的 API 和集成方式，弄清插件接口需要提供什么：身份、授权、事件和数据访问。

- **[Jellyfin](https://jellyfin.org/)**：影音媒体库和转码。
- **[Home Assistant](https://www.home-assistant.io/)**：智能家居。
- **[Forgejo](https://forgejo.org/)**：代码托管。
- **[Miniflux](https://miniflux.app/)**：订阅阅读器。

## 社区与可持续性

- **[Debian 社会契约](https://www.debian.org/social_contract)**：以书面形式写下项目对用户的承诺。
- **[Rust RFC](https://rust-lang.github.io/rfcs/)** 和
  **[Python PEP](https://peps.python.org/)**：公开地做决定。
- **[NLnet](https://nlnet.nl/)**：资助开放互联网软件的公共基金。
- **[Software Freedom Conservancy](https://sfconservancy.org/)** 和
  **[Signal 基金会](https://signalfoundation.org/)**：自由软件的非营利归宿。
