# 参与 fileOS 开发

[English](CONTRIBUTING.md) | **简体中文**

讨论和代码同样欢迎：可以开 issue 质疑某条原则、某个设计决定或本文件中的任何内容。

请先阅读 [README](README.zh-CN.md) 的“系统模型”。贡献会对照它和本文件中的规则评判。

## 语言：Basic English

项目自身发布的英文一律使用 [Basic English](https://zh.wikipedia.org/wiki/基本英语)，包括 README、文档、本文件、命令行帮助、错误信息、代码注释、提交信息、发布说明和 GitHub 模板。Basic English 句子短、只用少量常见词，母语不是英语的读者容易读懂，机器翻译效果也好。专有名称（Rust、SQLite、OpenID Connect）以及没有更简单说法的术语保持原样。

我们建议你在 issue、PR 和评论中也使用 Basic English，但不作要求，用中文写同样可以。如果你的 PR 修改了项目发布的文字，维护者可能在合并前调整措辞。

所有文档均以英文版为准，其他语言版本仅供参考。

部分旧文字还没有改成 Basic English，欢迎提交改写。

## 开发环境

**我们推荐使用 [devenv](https://devenv.sh) 配合 [direnv](https://direnv.net)。这是推荐做法，不是强制做法。** 任何环境都可以，改动能否合并只由 CI 决定。

### 推荐做法：devenv + direnv

仓库在 `devenv.nix` 中定义了开发环境：`rust-toolchain.toml` 锁定的 Rust 工具链（目前为 1.100.0，含 clippy、rustfmt、rust-analyzer），以及 `cargo-deny`、`cargo-machete`、`cargo-llvm-lines`、运行脚本用的 Python 和用于文档站点的 Node.js。

```sh
# 首次：安装 Nix、devenv 和 direnv，然后在仓库目录中运行：
direnv allow
```

之后进入目录时，shell 会自动加载开发环境。推送前运行 `check`，用 `budget` 检查编译时间预算，用 `docs` 预览文档。用 `devenv shell -- artifacts` 构建发布产物，或用 `devenv build outputs.hs` 只构建发布版服务端。不用 direnv 时，运行 `devenv shell`。个人定制写在 `devenv.local.nix` 中，它已被 git 忽略。

### 不使用 Nix

需要安装：

- 通过 [rustup](https://rustup.rs) 安装 Rust。`rust-toolchain.toml` 会自动安装锁定的版本（1.100.0）及其组件。
- `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`
- [uv](https://docs.astral.sh/uv/)，用于运行 `scripts/` 中的脚本。它会安装 `.python-version` 锁定的 Python 版本（3.14）和锁定的开发工具：运行 `uv sync`。
- 只有修改文档时才需要：[Node.js](https://nodejs.org) 22.23.4 或更高版本。在 `docs/` 中运行 `npm install` 和 `npm run dev`。

## 编译时间是功能

> 超出编译时间预算的 PR 与测试失败的 PR 同等对待，不予合并。

一台普通笔记本应当足以参与开发。Rust 的编译时间主要取决于代码的写法。CI 会对每个 PR 运行 [`scripts/budget.py`](scripts/budget.py)，强制执行以下预算：

| 指标                          | 预算     |
| ----------------------------- | -------- |
| 全新 debug 编译整个 workspace | ≤ 360 秒 |
| 改动叶子 crate 后重新编译     | ≤ 5 秒   |
| 改动 `fos-core` 后重新编译    | ≤ 20 秒  |

参考机器是一台 4 核、8GB 内存的笔记本。预算会随代码增长重新校准。

### 依赖政策

1. **每个新增的外部依赖都要评审。** 在 PR 中说明为什么需要它，以及它让全新编译多花了多少时间（对比前后的 `cargo build --timings`）。评判一个依赖，看的是构建时间、二进制体积和它带进来了什么（第二套加密库或 HTTP 栈、被禁用的 crate、沉重的过程宏、构建脚本、C 或 C++），而不是 crate 数量：拆分成小 crate 的库，代价可能比一个大 crate 更小。
2. 只在根目录 `Cargo.toml` 的 `[workspace.dependencies]` 中声明一次，使用 `default-features = false`，只开启用到的功能。
3. 每个库只允许一个版本。`cargo deny check` 会拒绝重复版本，因为每个副本都要单独编译。上游 crate 带来、我们无法去掉的第二个版本，写进 `deny.toml` 的 `skip`，并注明原因。
4. 不使用需要 cmake 的 C 或 C++ 代码。由 `cc` crate 编译的小型 C 代码可以使用，例如 `ring` 里的代码。TLS 使用 rustls 的 `ring` 后端。`fos` 以单个可执行文件发布，只加载用户给它的 `.app` 文件。
5. 重型依赖只放在需要它的 crate 里，这样改动其他代码时不会重编它们：wasmtime 只在 `fos-runtime` 里，libp2p 只在 `fos-net` 里。

### 写出编译快的代码

- **小 crate，浅依赖图。** Cargo 会并行编译互不依赖的 crate，并且只重编改动过的部分。
- **阻止泛型扩散。** 使用内部函数写法：
  ```rust
  pub fn open(path: impl AsRef<Path>) -> Result<Db> {
      fn inner(path: &Path) -> Result<Db> { /* 真正的逻辑 */ }
      inner(path.as_ref())
  }
  ```
- **在 crate 边界使用 `dyn Trait`**，不要让类型参数贯穿整个程序。
- **在边界处给 future 装箱**（`Box::pin`），避免出现巨大的嵌套 future 类型。避免不必要的中间件层。
- **少用过程宏。** 原生 trait 中的 `async fn` 已经稳定，因此禁用 `async-trait`。只给真正需要序列化的类型派生 `serde`。
- **不在 `build.rs` 中生成代码。** 生成的代码直接提交到仓库。
- 用 `cargo llvm-lines` 找出生成代码最多的函数。

### 本地建议

- 编辑时运行 `cargo check`，它会跳过代码生成。
- Linux 上使用 `lld` 或 `mold` 链接器。新版 Rust 在 x86_64 Linux 上已默认使用 `lld`。
- 给 rust-analyzer 单独的 target 目录（`"rust-analyzer.cargo.targetDir": true`），避免阻塞命令行编译。

## Workspace 结构

```
crates/
  fos             可执行程序：只负责把各组件连起来
  fos-node        把各组件组装成一个节点
  fos-gateway     给客户端用的本地 HTTP API
  fos-runtime     wasmtime：.app 文件和文件夹、引擎和 App
  fos-net         libp2p：在节点之间传送同步消息
  fos-store       数据目录：节点密钥、blob、文件夹日志、本地键值存储
  fos-sync        节点之间的同步（sans-IO，可移植）
  fos-fs          文件夹：签名日志、顺序、分叉、挂载（sans-IO，可移植）
  fos-id          身份：did:fos、节点证书、身份链接（可移植）
  fos-core        格式：CBOR、ed25519、BLAKE3（依赖图最底层，可移植）
  fos-sim         用于测试的确定性多节点模拟
apps/             第二个 workspace，由 `apps` 编译成 wasm32-unknown-unknown
  fos-app         App 和引擎的 SDK（MIT OR Apache-2.0）
  capability      默认引擎
  photos          照片 App
wit/
  fos.wit         内核与组件之间的接口（MIT OR Apache-2.0）
```

标记为*可移植*的 crate 在 `Cargo.toml` 的 `[package.metadata.fos]` 下写有 `portable = true`。它们必须能为 `wasm32-unknown-unknown` 构建，因为 App、手机外壳和 Web 应用共用它们；`check` 会验证这一点。其中不要直接使用线程、文件、socket 和时钟，应通过各宿主实现的接口访问。

文档站点位于 `docs/`，使用 [Astro Starlight](https://starlight.astro.build) 构建，支持全文搜索和多语言。英文页面在 `docs/src/content/docs/en-us/`，简体中文页面在 `docs/src/content/docs/zh-cn/`，文件名一一对应。页面分为三本手册，每本是侧栏顶部切换器中的一个主题：`user/` 写给使用 fileOS 的人，`admin/` 写给运行节点的人，`dev/` 写给开发者。新页面要用两种语言编写，加入 `docs/astro.config.mjs` 的 `topics` 列表，并为该手册的读者而写。

生产构建时，将 `DOCS_SITE_URL` 设为站点的公开源地址，以生成规范网址、`hreflang` 链接和站点地图。根路径 `/` 仅用于语言跳转，不参与搜索索引，也不列入站点地图。

依赖只能向下。`fos-core` 不依赖任何其他 fileOS crate。App 和引擎只依赖 `fos-app`。`fos` 保持精简，以便几秒内重新编译。

## 命令行

`fos` 用于内核。它的命令分为几组（`node`、`fs`、`folder`、`identity`、`app`）。App 或插件做什么不在其中，打包和安装它们除外。命令行不保存任何状态：它不保存自己的任何东西，也没有登录或其他会改变后续命令行为的命令。它从节点的数据目录读取网关的地址和令牌。

## 只提供机制，不内置政策

我们提供机制和合理的默认值，规则由每个人自己决定。当代码要决定“应该怎么做”（帮谁、分享什么、保留什么）时，把它做成可替换的策略并附带默认值，而不是写死的规则。

## 内核、App 和内核插件

fileOS 由内核和 App 构成，内核支持内核插件。见 [README](README.zh-CN.md) 的“系统模型”。

- **不破坏用户空间。** App 和其他节点都是用户空间。App 接口（`wit/fos.wit`）以及节点之间的格式和协议只增加新东西。旧的行为保持不变，即使它是错的，因为可能有人依赖它。破坏性的改动要出新版本的接口，旧版本继续可用。
- **App 不是插件。** App 使用内核，不改变内核。
- **内核插件不改变 App 和其他节点看到的东西。** 它只在契约已有的结果中选择，例如 `denied`。
- **内核检查内核插件返回的结果**，用哈希、签名或 AEAD。内核插件出错时，最坏的结果是报错，绝不会是一个错误的文件。
- **内核提问，内核插件回答。** 控制和状态都在内核手里。内核插件拿到一个小的输入，返回一个值。它不插进工作流程中间，不改写请求，也不调用别的插件。不要带 `next()` 的钩子、注册表、事件总线和依赖注入。
- **代价只由做出选择的人承担。** 会伤害到别人的选择（例如共享文件夹的加密）放进协议，不做成内核插件。
- **默认值写在代码里。** 不装内核插件、不写配置，fileOS 也是完整的。`fos-node` 用普通代码把各部分组装起来。
- **新的一种内核插件需要现实的理由：** 现在就有人需要第二种做法。在那之前，crate 内部的一个 `dyn Trait` 就够了。

## 许可证

fileOS 采用 [AGPL-3.0-only](LICENSE) 授权，并为只使用插件接口的 App 和引擎提供 [fileOS 插件例外](PLUGIN-EXCEPTION.zh-CN.md)。提交贡献即表示你同意你的贡献采用以下许可证：

- **AGPL-3.0-only**：除 `apps/fos-app` 和 `wit/` 以外的所有内容
- **MIT OR Apache-2.0**：`apps/fos-app` 和 `wit/`，以便 App 和引擎可以使用任何许可证

你的作品版权仍归你所有。项目不使用贡献者许可协议（CLA），因此任何人（包括维护者）都无法单方面更改 fileOS 的许可证。

每个源文件以 SPDX 文件头开始：

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

依赖必须使用与 AGPL-3.0-only 兼容的许可证，白名单见 [`deny.toml`](deny.toml)。GPL-2.0-only 不兼容。

### 开发者原创证明（DCO）

每个提交都必须签署，以证明你有权依照[开发者原创证明](https://developercertificate.org/)贡献这段代码：

```sh
git commit -s
```

这会添加一行 `Signed-off-by: 你的名字 <you@example.com>`。缺少这一行的提交会被 CI 拒绝。

## 提交 PR 之前

CI 的每个任务都通过同一份 `devenv.nix` 在 Nix 中执行，因此在 devenv shell 中通过的检查在 CI 中也会通过：

```sh
treefmt  # 格式化所有文件（Rust、Nix、Python、TOML、Markdown、JS/TS 等）
check    # 格式、Clippy、测试、Rustdoc、依赖、WASM、Python、MSRV
budget   # 编译时间预算
```

工作区在 `Cargo.toml` 中声明最低 Rust 版本。Nix 使用该版本检查锁定依赖下的所有原生目标。

`apps` 会把引擎和 App 编译到 `target/apps/`。`fos-runtime` 和 `fos-node` 的测试要用到它们，所以先运行 `apps`，再运行 `cargo test`。`check` 会自动做这一步。

格式化工具及其配置只在 `devenv.nix` 中定义（通过 treefmt-nix）。进入 shell 不会改写文件，需要手动运行 `treefmt`。

不使用 devenv 时，直接运行脚本。`check` 需要 `PATH` 中有 `treefmt`，因此没有 Nix 时格式检查由 CI 完成：

```sh
uv run scripts/check.py
uv run scripts/budget.py
```

## 变更如何进入 main

`main` 是唯一的长期分支。任何人（包括维护者）都不直接推送，所有改动都通过 PR 进入。

1. 从 `main` 建分支：在你的 fork 中，维护者也可以在本仓库中。
2. 提交时每个 commit 都要签署（sign-off）。用户能感知的改动要附一个变更文件（见下文）。
3. 向 `main` 发起 PR，GitHub 会自动请求代码所有者审查。
4. `ci` 检查通过、所有评论都已解决后即可合并。只用 “Squash and merge”：每个 PR 在 `main` 上对应一个提交，第一行是 PR 标题。合并后自动删除分支。

只有 [`.github/workflows/ci.yml`](.github/workflows/ci.yml) 中的所有任务（`check`、`budget`、`artifacts`，PR 还有 `dco`）都通过，`ci` 才会通过。修改了 `docs/` 的 PR 还会试构建文档站点；合并到 `main` 后，站点发布到 <https://hs.nmnm.cc/>。

Dependabot 会为依赖更新开 PR：GitHub Actions 每周一次，Cargo、npm 和 uv 每月一次。这些 PR 走同样的检查。主版本升级按新增依赖的标准审查（见“依赖政策”）。

## 变更记录与发布

我们用 [Knope](https://knope.tech) 维护变更记录。devenv shell 中已包含 Knope；不用 Nix 时可执行 `cargo install --locked knope` 安装。

### 变更文件

每个包含用户可感知改动的 PR 都要在 `.changeset/` 中附一个变更文件，用下面的命令创建：

```sh
knope document-change
```

Knope 会询问改动类型和一句摘要：

- `major`：不兼容的改动，例如删除了命令行选项、修改了数据格式。1.0 之前，这会提升次版本号（0.1.0 到 0.2.0）。
- `minor`：新功能。
- `patch`：修复。

摘要会成为这条改动的标题，下面可以再写几段简短说明，面向用户而不是开发者。变更文件用 Basic English 写，并和 PR 的其他改动一起提交。测试、重构、CI 等用户感知不到的改动不需要变更文件。

不要手动编辑 `CHANGELOG.md`，发布时由 Knope 把变更文件写进去。

### 发布新版本

维护者在最新且没有未提交改动的 `main` 上执行：

```sh
knope prepare-release
```

Knope 根据变更文件算出新版本号，写入 `Cargo.toml` 和 `apps/Cargo.toml`，把变更文件写进 `CHANGELOG.md` 中新版本的小节，然后删除这些文件。随后 [`scripts/release_pr.py`](scripts/release_pr.py) 建立分支 `release/v<版本>`，提交一个 commit 并开 PR。

该 PR 合并后，[Release 工作流](.github/workflows/release.yml) 发现 `Cargo.toml` 中的版本还没有对应标签，于是构建发布文件（包含 `fos` 和各个 `.app` 文件夹的 `fos-<版本>-linux-x86_64.tar.gz`、文档和 `SHA256SUMS`）并执行 `knope release`。Knope 创建标签 `v<版本>` 和 GitHub Release，发布说明取自 `CHANGELOG.md` 中该版本的小节。
