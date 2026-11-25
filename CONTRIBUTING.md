# Contributing to homeserver

**English** | [简体中文](CONTRIBUTING.zh-CN.md)

Talk is as welcome as code. Open an issue to question a principle, a design decision or anything in this file.

Read the [manifesto](MANIFESTO.md) first. We judge contributions by its principles.

## Language: Basic English

All English that the project itself puts out is in [Basic English](https://en.wikipedia.org/wiki/Basic_English): the README, the documentation, this file, help for the command line, error messages, comments in the code, commit messages, release notes and the GitHub templates. Basic English uses short sentences and a small number of common words, so it is easy to read for people whose first language is not English, and it goes well through machine translation. Names of things (Rust, SQLite, OpenID Connect) and words for things that have no simpler name stay as they are.

For your issues, pull requests and comments, we suggest Basic English, but it is not a rule. Writing in Chinese is also good. When your pull request changes text that the project puts out, the maintainers may make changes to the wording before it is merged.

The English form of every document is the one that counts; other languages are only for reference.

Some older text is not yet in Basic English. Changes that put it into Basic English are welcome.

## Development environment

**We suggest [devenv](https://devenv.sh) with [direnv](https://direnv.net). This is a suggestion, not a rule.** Any setup works; only CI says if a change is accepted.

### Suggested: devenv + direnv

The repository gives its environment in `devenv.nix`. It has the Rust toolchain fixed in `rust-toolchain.toml` (at present 1.100.0, with clippy, rustfmt and rust-analyzer). It also has `cargo-deny`, `cargo-machete`, `cargo-llvm-lines`, Python for the scripts and Node.js for the documentation site.

```sh
# once: install Nix, devenv and direnv, then in the repository:
direnv allow
```

After that, the shell loads the environment every time you go into the directory. Run `check` before you push, `budget` to check the compile-time budget and `docs` to see the documentation before it goes out. Build the release files with `devenv shell -- artifacts`, or only the release server with `devenv build outputs.hs`. Without direnv, use `devenv shell`. Put your own small changes in `devenv.local.nix`; git does not see that file.

### Without Nix

Install:

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` installs the fixed version (1.100.0) and its components for you.
- `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`
- [uv](https://docs.astral.sh/uv/), for the scripts in `scripts/`. Run `uv sync`; it installs the Python version fixed in `.python-version` (3.14) and the locked dev tools.
- Only if you work on the documentation: [Node.js](https://nodejs.org) 22.23.4 or later. Run `npm install` and `npm run dev` in `docs/`.

## Compile time is a feature

> A PR that goes over the compile-time budget is the same as a PR that fails the tests: we do not merge it.

An ordinary laptop should be enough to contribute. Most of Rust's compile time depends on how the code is written. On every PR, CI checks these budgets with [`scripts/budget.py`](scripts/budget.py):

| Measure                                | Budget  |
| -------------------------------------- | ------- |
| Clean debug build of the workspace     | ≤ 180 s |
| Rebuild after a change to a leaf crate | ≤ 5 s   |
| Rebuild after a change to `hs-core`    | ≤ 20 s  |

The budgets are for a laptop with 4 cores and 8 GB of RAM. We will change the budgets as the code gets bigger.

### Dependency policy

1. **We review every new external dependency.** Say in the PR why it is needed and how much time it adds to the clean build (compare `cargo build --timings` before and after). Judge it by build time, binary size and what it brings with it (a second cryptography library or HTTP stack, a banned crate, heavy procedural macros, build scripts, C or C++). Do not judge it by its number of crates: a library in many small crates can cost less than one large crate.
2. Put it once in the root `Cargo.toml` under `[workspace.dependencies]`, with `default-features = false`. Switch on only the features you use.
3. One version of each crate. `cargo deny check` does not let two versions in, because each copy gets compiled by itself.
4. No C or C++ that needs cmake. Small C code that the `cc` crate compiles is allowed, such as SQLite through the `bundled` feature of SQLx's SQLite driver. For TLS, use rustls with the `ring` backend. `hs` comes as one executable; it does not download or load anything at run time.
5. Keep heavy dependencies (networking, cryptography, search, WASM) inside the crate that needs them, so changes in other places do not rebuild them.

### Writing code that compiles fast

- **Small crates, shallow graph.** Cargo builds crates that do not depend on each other at the same time, and only rebuilds what changed.
- **Do not let generics spread.** Use the inner-function pattern:
  ```rust
  pub fn open(path: impl AsRef<Path>) -> Result<Db> {
      fn inner(path: &Path) -> Result<Db> { /* the real work */ }
      inner(path.as_ref())
  }
  ```
- **Use `dyn Trait` at crate boundaries.** Do not take type parameters through the whole program.
- **Box futures at boundaries** (`Box::pin`), so you do not get very large nested future types. Do not add middleware layers you do not need.
- **Few procedural macros.** Native `async fn` in traits is stable, so `async-trait` is banned. Derive `serde` traits only on types that really get serialized.
- **No code generation in `build.rs`.** Commit the generated code.
- Use `cargo llvm-lines` to find the functions that generate the most code.

### Local tips

- Run `cargo check` while you make changes. It does not do code generation.
- On Linux, use the `lld` or `mold` linker. New versions of Rust use `lld` on x86_64 Linux by default.
- Give rust-analyzer its own target directory (`"rust-analyzer.cargo.targetDir": true`), so it does not make command-line builds wait.

## Workspace layout

```
crates/
  hs              binary: wires the components together, nothing else
  hs-api          everything served over HTTP: REST, federation, DAV, MCP, Bitwarden, S3
  hs-node         assembles the components into one node
  hs-net          federation and mutual aid between servers
  hs-client       REST client and wire types, with optional OpenAPI schemas
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

Crates marked _portable_ have `portable = true` under `[package.metadata.hs]` in their `Cargo.toml`. They have to build for `wasm32-unknown-unknown`, because plugins, phone shells and the web app use them; `check` makes sure of this. Do not use threads, files, sockets or clocks in them. Get to those through interfaces that each host gives.

The documentation site is in `docs/`. It is built with [Astro Starlight](https://starlight.astro.build) (full-text search, translations). English pages are in `docs/src/content/docs/en-us/`, and Simplified Chinese pages are in `docs/src/content/docs/zh-cn/`, with the same file names. The pages are in three manuals. Each manual is a topic in the switcher at the top of the sidebar: `user/` for people who use a homeserver, `admin/` for people who run one, and `dev/` for developers. Put a new page in both languages, and add it to the `topics` list in `docs/astro.config.mjs`. Write it for the reader of its manual.

For production builds, set `DOCS_SITE_URL` to the public origin of the site. Then the build makes canonical URLs, `hreflang` links and the sitemap. The page at `/` only sends you to a language; it is not indexed and is not in the sitemap.

Dependencies only go down. `hs-core` depends on no other homeserver crate. `hs-plugin-api` has no dependencies at all. `hs` stays thin, so that it rebuilds in seconds.

## Mechanism, not policy

We give mechanisms and sensible defaults; each person makes the rules. When code says _what is to happen_ (whom to help, what to share, what to keep), make it a policy with a default that people can change. Do not make it a fixed rule in the code.

## Licensing

homeserver is under [AGPL-3.0-only](LICENSE), with the [homeserver Plugin Exception](PLUGIN-EXCEPTION.md) for plugins that use only the Plugin Interface. When you contribute, you agree that your contribution is under:

- **AGPL-3.0-only**, for everything except `crates/hs-plugin-api`
- **MIT OR Apache-2.0**, for `crates/hs-plugin-api`, so that plugins can use any license

You keep the copyright to your work. There is no Contributor License Agreement. So no one, not even the maintainers, is able to put homeserver under a different license by themselves.

Put an SPDX header at the start of every source file:

```rust
// SPDX-License-Identifier: AGPL-3.0-only
```

Dependencies have to use licenses that work with AGPL-3.0-only. The list of allowed licenses is in [`deny.toml`](deny.toml). GPL-2.0-only does not work with it.

### Developer Certificate of Origin

Every commit needs a sign-off. With it, you say that you have the right to contribute the commit under the [Developer Certificate of Origin](https://developercertificate.org/):

```sh
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line. CI does not accept commits without it.

## Before opening a pull request

CI runs every job in Nix, with the same `devenv.nix`. So what passes in the devenv shell passes in CI:

```sh
treefmt  # format every file (Rust, Nix, Python, TOML, Markdown, JS/TS, ...)
check    # formatting, Clippy, tests, Rustdoc, dependencies, WASM, Python, MSRV
budget   # compile-time budget
```

The workspace gives its MSRV (the oldest Rust version it supports) in `Cargo.toml`. Nix gives that compiler for a locked check of all native targets.

To measure how requests are handled with access logging on, run `cargo bench -p hs-api --bench request_path`. Set `HS_BENCH_REQUESTS` to change the sample size. The benchmark includes putting access events in the queue and flushing them. It also gives the p99 latency for blob range reads and resource writes at the same time, over loopback HTTP.

The formatters and their settings are only in `devenv.nix` (through treefmt-nix). Going into the shell does not change files; run `treefmt` yourself.

Without devenv, run the scripts yourself. `check` needs `treefmt` on `PATH`, so without Nix, CI does the formatting check:

```sh
uv run scripts/check.py
uv run scripts/budget.py
```

## How changes get in

`main` is the only long-lived branch. Nobody, not even a maintainer, pushes to it directly: every change comes in through a pull request.

1. Make a branch from `main` in your fork (or in this repository, if you are a maintainer).
2. Make your commits, each with a sign-off. If users will see the change, make a change file (see below).
3. Open a pull request into `main`. GitHub asks the code owners for a review.
4. A pull request may be merged when the `ci` check is green and every comment thread is resolved. It is merged with "Squash and merge": `main` gets one commit for each pull request, with the title of the pull request as its first line. The branch is deleted after the merge.

The `ci` check is green only when every job in [`.github/workflows/ci.yml`](.github/workflows/ci.yml) is green: `check`, `budget`, `artifacts` and, for pull requests, `dco`. A pull request that changes `docs/` also makes the documentation site as a test. After a merge into `main`, the site goes to <https://hs.nmnm.cc/>.

Dependabot opens pull requests for dependency updates: every week for GitHub Actions, and every month for Cargo, npm and uv. They go through the same checks. A major update is checked like a new dependency (see "Dependency policy").

## Changes and releases

We keep the changelog with [Knope](https://knope.tech). Knope is in the devenv shell; without Nix, get it with `cargo install --locked knope`.

### Change files

Every pull request with a change that users will see also gives a change file in `.changeset/`. Make one with:

```sh
knope document-change
```

Knope asks for the type of change and a short summary:

- `major`: a breaking change, for example a command line option that is no longer there, or a change to a data format. Before 1.0, this gives a new minor version (0.1.0 to 0.2.0).
- `minor`: a new feature.
- `patch`: a fix.

The summary is the heading of the change. Under it, you may say more in a few short paragraphs, for users, not for developers. Write it in Basic English. Commit the file with the rest of the pull request. Changes that users will not see, such as tests, refactoring or CI, do not need a change file.

Do not make changes to `CHANGELOG.md` by hand. Knope puts the change files into it at release time.

### Making a release

A maintainer makes a release from an up-to-date `main` with no uncommitted changes:

```sh
knope prepare-release
```

Knope works out the new version from the change files. It puts that version in `Cargo.toml` (also for `hs-client`), puts the change files into `CHANGELOG.md` under the new version, and deletes them. Then [`scripts/release_pr.py`](scripts/release_pr.py) makes the branch `release/v<version>` with one commit, and opens a pull request.

When that pull request is merged, the [Release workflow](.github/workflows/release.yml) sees a version in `Cargo.toml` that has no tag. It makes the release files (`hs-<version>-linux-x86_64.tar.gz`, the documentation, and `SHA256SUMS`) and runs `knope release`. Knope makes the tag `v<version>` and the GitHub release, with the part of `CHANGELOG.md` for that version as the release notes.
