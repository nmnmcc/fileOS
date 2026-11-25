# Contributing to homeserver

**English** | [简体中文](CONTRIBUTING.zh-CN.md)

Thank you for helping build homeserver. The project is in its design phase, so discussion matters as much as code: open an issue to question a principle, a design decision or a line in this file.

Read the [README](README.md) first. Every contribution is judged against its principles.

## Development environment

**We recommend [devenv](https://devenv.sh) with [direnv](https://direnv.net). This is a recommendation, not a requirement.** Use whatever setup works for you; CI is the only thing that decides whether a change is accepted.

### Recommended: devenv + direnv

The repository defines its environment in `devenv.nix`: the Rust toolchain pinned in `rust-toolchain.toml` (currently 1.100.0, with clippy, rustfmt and rust-analyzer), plus `cargo-deny`, `cargo-machete`, `cargo-llvm-lines`, Python for the scripts and Node.js for the documentation site.

```sh
# once: install Nix, devenv and direnv, then in the repository:
direnv allow
```

Your shell now loads the environment whenever you enter the directory. Run `check` before pushing, `budget` to check the compile-time budget and `docs` to preview the documentation. Build release artifacts with `devenv build outputs.hs` and `devenv build outputs.docs`. Without direnv, run `devenv shell` instead. Personal tweaks go in `devenv.local.nix`, which is ignored by git.

### Without Nix

Install these yourself:

- Rust via [rustup](https://rustup.rs). `rust-toolchain.toml` installs the pinned version (1.100.0) and its components automatically.
- `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`
- [uv](https://docs.astral.sh/uv/), for the scripts in `scripts/`. It installs the Python version pinned in `.python-version` (3.14) and the locked dev tools: run `uv sync`.
- Only if you work on the documentation: [Node.js](https://nodejs.org) 22.23.4 or later. Run `npm install` and `npm run dev` in `docs/`.

That's all homeserver needs.

## Compile time is a feature

> A PR that makes an ordinary laptop slower is treated like a PR that fails the tests. It doesn't get merged.

Anyone should be able to contribute from an ordinary laptop. Rust can be slow to compile, and most of that slowness comes from how code is written. CI enforces these budgets on every PR with [`scripts/budget.py`](scripts/budget.py):

| Measure                            | Budget  |
| ---------------------------------- | ------- |
| Clean debug build of the workspace | ≤ 180 s |
| Rebuild after editing a leaf crate | ≤ 5 s   |
| Rebuild after editing `hs-core`    | ≤ 20 s  |

The reference machine is a 4-core laptop with 8 GB of RAM. The budgets will be recalibrated once real code exists.

### Dependency policy

1. **Every new external dependency is reviewed.** Say in the PR why it is needed and how much it adds to the clean build (compare `cargo build --timings` before and after). Judge it by build time, binary size and what it brings in (a second cryptography library or HTTP stack, a banned crate, heavy procedural macros, build scripts, C or C++), not by how many crates it is split into: a library split carefully into small crates can cost less than one big crate.
2. Declare it once in the root `Cargo.toml` under `[workspace.dependencies]`, with `default-features = false`, enabling only the features you use.
3. One version per crate. `cargo deny check` rejects duplicates, because each copy compiles separately.
4. No C or C++ that needs cmake. Small C compiled by the `cc` crate is fine, such as SQLite through rusqlite's `bundled` feature. For TLS, use rustls with the `ring` backend. `hs` ships as one executable; nothing is downloaded or loaded at run time.
5. Keep heavy dependencies (networking, cryptography, search, WASM) inside the crate that needs them, so day-to-day edits elsewhere don't rebuild them.

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
- **Box futures at boundaries** (`Box::pin`) to avoid huge nested future types. Don't stack middleware layers without need.
- **Few procedural macros.** Native `async fn` in traits is stable, so `async-trait` is banned. Derive `serde` traits only on types that are actually serialized.
- **No code generation in `build.rs`.** Commit generated code instead.
- Use `cargo llvm-lines` to find functions that generate the most code.

### Tips for your machine

- Run `cargo check` while editing. It skips code generation.
- On Linux, use the `lld` or `mold` linker. Recent Rust already uses `lld` on x86_64 Linux.
- Give rust-analyzer its own target directory (`"rust-analyzer.cargo.targetDir": true`) so it doesn't block your command-line builds.

## Workspace layout

```
crates/
  hs              binary: wires components together, nothing else
  hs-net          federation and mutual aid
  hs-plugin-host  sandboxed WASM plugin runtime
  hs-node         assembles the components into one node
  hs-store        derived index in SQLite: resources, links, search
  hs-identity     key-based identity, devices, migration (portable)
  hs-plugin-api   Plugin Interface (MIT OR Apache-2.0, no_std, portable)
  hs-core         object model (bottom of the graph, portable)
```

Crates marked _portable_ have `portable = true` under `[package.metadata.hs]` in their `Cargo.toml`. They must build for `wasm32-unknown-unknown`, because plugins, phone shells and the web app share them; `check` verifies this. Keep threads, files, sockets and clocks out of them: take those through an interface that each host implements.

The documentation site lives in `docs/` and is built with [Astro Starlight](https://starlight.astro.build), which provides full-text search and translations. English pages are in `docs/src/content/docs/en-us/` and Simplified Chinese pages are in `docs/src/content/docs/zh-cn/`, with the same file names.

For production builds, set `DOCS_SITE_URL` to the public site origin to generate canonical URLs, `hreflang` links, and the sitemap. The language redirect at `/` is not indexed or included in the sitemap.

Dependencies flow downward only. `hs-core` depends on no other homeserver crate. `hs-plugin-api` has no dependencies at all. `hs` stays thin so that editing it rebuilds in seconds.

## Mechanism, not policy

We provide mechanisms and sensible defaults; each person decides the rules. When code needs to decide _what should happen_ (whom to help, what to share, what to keep), make it a replaceable policy with a default, not a hard-coded rule.

## Licensing

homeserver is licensed under [AGPL-3.0-only](LICENSE), with the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md) for plugins that use only the Plugin Interface. By contributing, you agree that your contribution is licensed under:

- **AGPL-3.0-only**, for everything except `crates/hs-plugin-api`
- **MIT OR Apache-2.0**, for `crates/hs-plugin-api`, so that plugins can use any license

You keep the copyright to your work. We don't use a Contributor License Agreement, and nobody, including the maintainers, can relicense homeserver on their own.

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

CI runs every job inside Nix, through the same `devenv.nix`, so what passes in the devenv shell passes in CI:

```sh
treefmt  # format every file (Rust, Nix, Python, TOML, Markdown, JS/TS, ...)
check    # treefmt --ci, clippy, tests, cargo deny, Python types (basedpyright, standard)
budget   # compile-time budget
```

Formatters and their settings are defined only in `devenv.nix` (via treefmt-nix). Entering the shell never rewrites files; run `treefmt` yourself.

Without devenv, run the same scripts directly. `check` expects `treefmt` on your `PATH`, so without Nix the formatting check is left to CI:

```sh
uv run scripts/check.py
uv run scripts/budget.py
```
