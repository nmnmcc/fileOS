# 参与 homeserver 开发

[English](CONTRIBUTING.md) | **简体中文**

感谢你参与 homeserver。项目正处于设计阶段，讨论和代码同样重要：欢迎开 issue 质疑某条原则、某个设计决定，或者这份文件里的任何一句话。

请先阅读 [README](README.zh-CN.md)。每一项贡献都会对照其中的原则来评判。

## 开发环境

**我们推荐使用 [devenv](https://devenv.sh) 配合 [direnv](https://direnv.net)。这是推荐做法，不是强制做法。** 用什么环境都可以，决定改动能否合并的只有 CI。

### 推荐做法：devenv + direnv

仓库在 `devenv.nix` 中定义了开发环境：`rust-toolchain.toml` 锁定的 Rust 工具链（目前为 1.100.0，含 clippy、rustfmt、rust-analyzer），以及 `cargo-deny`、`cargo-machete`、`cargo-llvm-lines`、运行脚本用的 Python 和用于文档站点的 Node.js。

```sh
# 首次：安装 Nix、devenv 和 direnv，然后在仓库目录中运行：
direnv allow
```

之后每次进入目录，shell 都会自动加载开发环境。推送前运行 `check`，运行 `budget` 可以检查编译时间预算，运行 `docs` 可以预览文档。用 `devenv shell -- artifacts` 构建发布产物。只构建发布版服务端时，运行 `devenv build outputs.hs`。不用 direnv 的话，运行 `devenv shell` 即可。个人定制写在 `devenv.local.nix` 中，它已被 git 忽略。

### 不使用 Nix

自行安装以下工具：

- 通过 [rustup](https://rustup.rs) 安装 Rust。`rust-toolchain.toml` 会自动安装锁定的版本（1.100.0）及其组件。
- `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`
- [uv](https://docs.astral.sh/uv/)，用于运行 `scripts/` 中的脚本。它会安装 `.python-version` 锁定的 Python 版本（3.14）和锁定的开发工具：运行 `uv sync`。
- 只有修改文档时才需要：[Node.js](https://nodejs.org) 22.23.4 或更高版本。在 `docs/` 中运行 `npm install` 和 `npm run dev`。

homeserver 需要的就这些。

## 编译时间是功能

> 让一台普通笔记本变慢的 PR，和让测试失败的 PR 一样，都不能合并。

每个人都应该能用一台普通笔记本参与开发。Rust 的编译可能很慢，而慢的原因大多出在代码的写法上。CI 会对每个 PR 运行 [`scripts/budget.py`](scripts/budget.py)，强制执行以下预算：

| 指标                          | 预算     |
| ----------------------------- | -------- |
| 全新 debug 编译整个 workspace | ≤ 180 秒 |
| 改动叶子 crate 后重新编译     | ≤ 5 秒   |
| 改动 `hs-core` 后重新编译     | ≤ 20 秒  |

参考机器是一台 4 核、8GB 内存的笔记本。有了真实代码之后，我们会重新校准这些预算。

### 依赖政策

1. **每个新增的外部依赖都要评审。** 在 PR 中说明为什么需要它，以及它让全新编译多花了多少时间（对比前后的 `cargo build --timings`）。评判一个依赖，看的是构建时间、二进制体积和它带进来了什么（第二套加密库或 HTTP 栈、被禁用的 crate、沉重的过程宏、构建脚本、C 或 C++），而不是它被拆成了多少个 crate：一个仔细拆分成小 crate 的库，代价可能比一个大 crate 还小。
2. 只在根目录 `Cargo.toml` 的 `[workspace.dependencies]` 中声明一次，使用 `default-features = false`，只开启用到的功能。
3. 每个库只允许一个版本。`cargo deny check` 会拒绝重复版本，因为每个副本都要单独编译。
4. 不使用需要 cmake 的 C 或 C++ 代码。由 `cc` crate 编译的小型 C 代码可以使用，例如通过 SQLx SQLite 驱动的 `bundled` 特性编译的 SQLite。TLS 使用 rustls 的 `ring` 后端。`hs` 以单个可执行文件发布，运行时不下载、不加载任何东西。
5. 重型依赖（网络、加密、搜索、WASM）只放在需要它的 crate 里，这样日常改动其他代码时不会重编它们。

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
- **在边界处给 future 装箱**（`Box::pin`），避免出现巨大的嵌套 future 类型。不要无必要地叠加中间件。
- **少用过程宏。** 原生 trait 中的 `async fn` 已经稳定，因此禁用 `async-trait`。只给真正需要序列化的类型派生 `serde`。
- **不在 `build.rs` 中生成代码。** 生成的代码直接提交到仓库。
- 用 `cargo llvm-lines` 找出生成代码最多的函数。

### 本机小贴士

- 编辑时运行 `cargo check`，它会跳过代码生成。
- Linux 上使用 `lld` 或 `mold` 链接器。新版 Rust 在 x86_64 Linux 上已默认使用 `lld`。
- 给 rust-analyzer 单独的 target 目录（`"rust-analyzer.cargo.targetDir": true`），避免它和命令行编译互相阻塞。

## Workspace 结构

```
crates/
  hs              可执行程序：只负责把各组件连起来
  hs-net          联邦与互助
  hs-plugin-host  沙箱化的 WASM 插件运行时
  hs-node         把各组件组装成一个节点
  hs-store        SQLite 中的派生索引：资源、链接、搜索
  hs-identity     基于密钥的身份、设备、迁移（可移植）
  hs-plugin-api   插件接口（MIT OR Apache-2.0，no_std，可移植）
  hs-core         对象模型（依赖图的最底层，可移植）
```

标记为*可移植*的 crate 在 `Cargo.toml` 的 `[package.metadata.hs]` 下写有 `portable = true`。它们必须能为 `wasm32-unknown-unknown` 构建，因为插件、手机外壳和 Web 应用共用它们；`check` 会验证这一点。不要在其中直接使用线程、文件、socket 和时钟，而是通过由各个宿主实现的接口来获取。

文档站点位于 `docs/`，使用 [Astro Starlight](https://starlight.astro.build) 构建，自带全文搜索和多语言支持。英文页面放在 `docs/src/content/docs/en-us/`，简体中文页面放在 `docs/src/content/docs/zh-cn/`，两者文件名一一对应。

生产构建时，将 `DOCS_SITE_URL` 设为站点的公开源地址，以生成规范网址、`hreflang` 链接和站点地图。根路径 `/` 仅用于语言跳转，不参与搜索索引，也不列入站点地图。

依赖只能向下。`hs-core` 不依赖任何其他 homeserver crate。`hs-plugin-api` 没有任何依赖。`hs` 保持精简，改动它只需几秒就能重新编译。

## 只提供机制，不内置政策

我们提供机制和合理的默认值，规则由每个人自己决定。当代码需要决定“应该怎么做”（帮谁、分享什么、保留什么）时，把它做成可替换的策略并附带默认值，而不是写死的规则。

## 许可证

homeserver 采用 [AGPL-3.0-only](LICENSE) 授权，并为只使用插件接口的插件提供 [homeserver 插件例外](PLUGIN-EXCEPTION.md)。提交贡献即表示你同意你的贡献采用以下许可证：

- **AGPL-3.0-only**：除 `crates/hs-plugin-api` 以外的所有内容
- **MIT OR Apache-2.0**：`crates/hs-plugin-api`，以便插件可以使用任何许可证

你的作品版权仍归你所有。我们不使用贡献者许可协议（CLA），任何人，包括维护者，都无法单方面更改 homeserver 的许可证。

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

CI 的每个任务都通过同一份 `devenv.nix` 在 Nix 中执行，所以在 devenv shell 里通过的检查，在 CI 中也会通过：

```sh
treefmt  # 格式化所有文件（Rust、Nix、Python、TOML、Markdown、JS/TS 等）
check    # 格式、Clippy、测试、Rustdoc、依赖、WASM、Python、MSRV
budget   # 编译时间预算
```

工作区在 `Cargo.toml` 中声明最低 Rust 版本。Nix 使用该版本检查锁定依赖下的所有原生目标。

运行 `cargo bench -p hs-api --bench request_path` 可测量开启访问日志后的请求处理速度，用 `HS_BENCH_REQUESTS` 调整样本量。该基准包含日志入队和刷盘。队列满时，请求会异步等待。

格式化工具及其配置只在 `devenv.nix` 中定义（通过 treefmt-nix）。进入 shell 不会改写任何文件，需要你自己运行 `treefmt`。

不使用 devenv 的话，直接运行同样的脚本。`check` 需要 `PATH` 中有 `treefmt`，所以没有 Nix 时格式检查交给 CI：

```sh
uv run scripts/check.py
uv run scripts/budget.py
```
