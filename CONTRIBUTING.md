# Contributing to homeserver

**English** | [简体中文](CONTRIBUTING.zh-CN.md)

Discussion is as welcome as code: open an issue to question a principle, a design decision or anything in this file.

Read the [README](README.md) first; contributions are judged against its principles.

## Development environment

**We recommend [devenv](https://devenv.sh) with [direnv](https://direnv.net). This is a recommendation, not a requirement.** Any setup works; only CI decides whether a change is accepted.

### Recommended: devenv + direnv

The repository defines its environment in `devenv.nix`: the Rust toolchain pinned in `rust-toolchain.toml` (currently 1.100.0, with clippy, rustfmt and rust-analyzer), plus `cargo-deny`, `cargo-machete`, `cargo-llvm-lines`, Python for the scripts and Node.js for the documentation site.

```sh
# once: install Nix, devenv and direnv, then in the repository:
direnv allow
```

The shell then loads the environment whenever you enter the directory. Run `check` before pushing, `budget` to check the compile-time budget and `docs` to preview the documentation. Build release artifacts with `devenv shell -- artifacts`, or only the release server with `devenv build outputs.hs`. Without direnv, use `devenv shell`. Personal tweaks go in `devenv.local.nix`, which is ignored by git.

### Without Nix

Install:

- Rust via [rustup](https://rustup.rs). `rust-toolchain.toml` installs the pinned version (1.100.0) and its components automatically.
- `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`
- [uv](https://docs.astral.sh/uv/), for the scripts in `scripts/`. It installs the Python version pinned in `.python-version` (3.14) and the locked dev tools: run `uv sync`.
- Only if you work on the documentation: [Node.js](https://nodejs.org) 22.23.4 or later. Run `npm install` and `npm run dev` in `docs/`.

## Compile time is a feature

> A PR that exceeds the compile-time budget is treated like a PR that fails the tests: it is not merged.

An ordinary laptop should be enough to contribute. Most of Rust's compile time depends on how the code is written. CI enforces these budgets on every PR with [`scripts/budget.py`](scripts/budget.py):

| Measure                            | Budget  |
| ---------------------------------- | ------- |
| Clean debug build of the workspace | ≤ 180 s |
| Rebuild after editing a leaf crate | ≤ 5 s   |
| Rebuild after editing `hs-core`    | ≤ 20 s  |

The reference machine is a 4-core laptop with 8 GB of RAM. The budgets will be recalibrated as the code grows.

### Dependency policy

1. **Every new external dependency is reviewed.** Say in the PR why it is needed and how much it adds to the clean build (compare `cargo build --timings` before and after). Judge it by build time, binary size and what it brings in (a second cryptography library or HTTP stack, a banned crate, heavy procedural macros, build scripts, C or C++), not by its crate count: a library split into small crates can cost less than one large crate.
2. Declare it once in the root `Cargo.toml` under `[workspace.dependencies]`, with `default-features = false`, enabling only the features you use.
3. One version per crate. `cargo deny check` rejects duplicates, because each copy compiles separately.
4. No C or C++ that needs cmake. Small C compiled by the `cc` crate is allowed, such as SQLite through the `bundled` feature of SQLx's SQLite driver. For TLS, use rustls with the `ring` backend. `hs` ships as one executable; nothing is downloaded or loaded at run time.
5. Keep heavy dependencies (networking, cryptography, search, WASM) inside the crate that needs them, so edits elsewhere don't rebuild them.

### Writing code that compiles fast

- **Small crates, shallow graph.** Cargo builds independent crates in parallel and only rebuilds what changed.
- **Stop generics from spreading.** Use the inner-function pattern:
  ```rust
  pub fn open(path: impl AsRef<Path>) -> Result<Db> {
      fn inner(path: &Path) -> Result<Db> { /* the real work */ }
      inner(path.as_ref())
  }
  ```
- **Use `dyn Trait` at crate boundaries** instead of threading type parameters through the whole program.
- **Box futures at boundaries** (`Box::pin`) to avoid huge nested future types. Avoid unnecessary middleware layers.
- **Few procedural macros.** Native `async fn` in traits is stable, so `async-trait` is banned. Derive `serde` traits only on types that are actually serialized.
- **No code generation in `build.rs`.** Commit generated code instead.
- Use `cargo llvm-lines` to find the functions that generate the most code.

### Local tips

- Run `cargo check` while editing. It skips code generation.
- On Linux, use the `lld` or `mold` linker. Recent Rust uses `lld` on x86_64 Linux by default.
- Give rust-analyzer its own target directory (`"rust-analyzer.cargo.targetDir": true`) so it does not block command-line builds.

## Workspace layout

```
crates/
  hs              binary: wires the components together, nothing else
  hs-api          everything served over HTTP: REST, federation, DAV, MCP, Bitwarden, S3
  hs-node         assembles the components into one node
  hs-net          federation and mutual aid between servers
  hs-plugin-host  sandboxed WebAssembly plugin runtime
  hs-idp          OpenID Connect provider, passkeys, forward auth (no I/O)
  hs-objects      blobs, replicas, encryption at the trust boundary
  hs-store        derived index in SQLite: resources, links, search, places
  hs-space        space logs: ordering, membership, grants (sans-IO)
  hs-media        media metadata and thumbnails
  hs-sim          deterministic multi-node simulation, for tests
  hs-identity     key-based identity, devices, migration (portable)
  hs-plugin-api   Plugin Interface (MIT OR Apache-2.0, no_std, portable)
  hs-core         core formats: CBOR, COSE, hashes, logs (bottom of the graph, portable)
```

Crates marked _portable_ have `portable = true` under `[package.metadata.hs]` in their `Cargo.toml`. They must build for `wasm32-unknown-unknown`, because plugins, phone shells and the web app share them; `check` verifies this. Keep threads, files, sockets and clocks out of them; reach those through interfaces that each host implements.

The documentation site lives in `docs/` and is built with [Astro Starlight](https://starlight.astro.build) (full-text search, translations). English pages are in `docs/src/content/docs/en-us/`, Simplified Chinese pages in `docs/src/content/docs/zh-cn/`, with the same file names.

For production builds, set `DOCS_SITE_URL` to the public site origin to generate canonical URLs, `hreflang` links and the sitemap. The language redirect at `/` is not indexed or included in the sitemap.

Dependencies flow downward only. `hs-core` depends on no other homeserver crate. `hs-plugin-api` has no dependencies at all. `hs` stays thin so that it rebuilds in seconds.

## Mechanism, not policy

We provide mechanisms and sensible defaults; each person decides the rules. When code decides _what should happen_ (whom to help, what to share, what to keep), make it a replaceable policy with a default, not a hard-coded rule.

## Licensing

homeserver is licensed under [AGPL-3.0-only](LICENSE), with the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md) for plugins that use only the Plugin Interface. By contributing, you agree that your contribution is licensed under:

- **AGPL-3.0-only**, for everything except `crates/hs-plugin-api`
- **MIT OR Apache-2.0**, for `crates/hs-plugin-api`, so that plugins can use any license

You keep the copyright to your work. There is no Contributor License Agreement, so no one, including the maintainers, can relicense homeserver alone.

Start every source file with an SPDX header:

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

Dependencies must use licenses compatible with AGPL-3.0-only. The allow list is in [`deny.toml`](deny.toml). GPL-2.0-only is incompatible.

### Developer Certificate of Origin

Every commit must be signed off, certifying that you have the right to contribute it under the [Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line. CI rejects commits without it.

## Before opening a pull request

CI runs every job in Nix through the same `devenv.nix`, so what passes in the devenv shell passes in CI:

```sh
treefmt  # format every file (Rust, Nix, Python, TOML, Markdown, JS/TS, ...)
check    # formatting, Clippy, tests, Rustdoc, dependencies, WASM, Python, MSRV
budget   # compile-time budget
```

The workspace declares its MSRV in `Cargo.toml`. Nix supplies that compiler for a locked check of all native targets.

To measure request handling with access logging enabled, run `cargo bench -p hs-api --bench request_path`. Set `HS_BENCH_REQUESTS` to change the sample size. The benchmark includes queueing and flushing access events; the bounded queue applies backpressure under load.

Formatters and their settings are defined only in `devenv.nix` (via treefmt-nix). Entering the shell does not rewrite files; run `treefmt` yourself.

Without devenv, run the scripts directly. `check` expects `treefmt` on `PATH`, so without Nix the formatting check is left to CI:

```sh
uv run scripts/check.py
uv run scripts/budget.py
```
