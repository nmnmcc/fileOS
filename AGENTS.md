# Notes for coding agents

This file is for AI coding agents working on fileOS. `CLAUDE.md` is a link to it. People should read [CONTRIBUTING.md](CONTRIBUTING.md). Everything here except "Worktrees" comes from it, and CONTRIBUTING.md is right when the two are different.

## Language

- All English that the project puts out is in **Basic English**: docs, README, CONTRIBUTING, CLI help, error messages, code comments, commit messages, pull requests, change files and release notes. Use short sentences and common words. Names of things (Rust, SQLite, OpenID Connect) stay as they are.
- Docs are in English and Simplified Chinese with the same file names: `docs/src/content/docs/en-us/` and `docs/src/content/docs/zh-cn/`. A change to one is also made to the other. The same is true of `README.md` / `README.zh-CN.md`, `MANIFESTO.md` / `MANIFESTO.zh-CN.md`, `CONTRIBUTING.md` / `CONTRIBUTING.zh-CN.md` and `SECURITY.md` / `SECURITY.zh-CN.md`.

## Commands

The devenv shell (`direnv allow`, or `devenv shell -- <command>`) has every tool. Without it, use `uv run scripts/<name>.py`.

| Command                 | What it does                                                        |
| ----------------------- | ------------------------------------------------------------------- |
| `treefmt`               | Formats every file. Run it before every commit.                     |
| `check`                 | Formatting, Clippy, tests, Rustdoc, cargo-deny, wasm, Python, MSRV. |
| `budget`                | The compile-time budget. Run it after changes to dependencies.      |
| `apps`                  | Builds the engine and apps in `apps/` into `target/apps/`.          |
| `docs`                  | Shows the documentation site.                                       |
| `knope document-change` | Makes a change file in `.changeset/`.                               |
| `cargo test -p <crate>` | Tests for one crate, while working.                                 |

The tests of `fos-runtime` and `fos-node` use the `.app` folders in `target/apps/`, so run `apps` before `cargo test`. The interfaces between the core and WebAssembly components are in `wit/fos.wit`.

## Worktrees

The main folder (the repository root) always stays on `main`. Never switch its branch, and do not make commits there. Someone else may be working in it.

For every piece of work, make a worktree in `.worktrees/` from an up-to-date `main`, and do all the work, the commits and the push there:

```sh
git fetch origin
git worktree add -b <branch> .worktrees/<name> origin/main
```

The `<name>` of the folder says in full what the work is, for example `.worktrees/remove-built-in-acme-certificates`, not `.worktrees/fix`. `.worktrees/` is in `.gitignore`.

When the work is done (the pull request is merged or closed, or the work is dropped), take the worktree away with `git worktree remove .worktrees/<name>`, then delete the local branch.

## How changes get in

- Never push to `main`; the branch rules do not let anyone do it. Make a branch, commit, push, and open a pull request into `main`. Pull requests are squashed when merged, so the title of the pull request becomes the commit message.
- Every commit has a sign-off: `git commit -s`. The `dco` check fails without it.
- A pull request may be merged only when the `ci` check is green.
- For a change that users will see, add a change file in `.changeset/` (`knope document-change` makes one; the type is `major`, `minor` or `patch`). Never make changes to `CHANGELOG.md` or the version in `Cargo.toml` by hand: `knope prepare-release` does that at release time.

## Rules for the code

- Compile time is a feature. A pull request over the budget is not merged. Keep generics from spreading (inner-function pattern), use `dyn Trait` at crate edges, use few procedural macros. No `async-trait`. No code generation in `build.rs`.
- Every new external dependency needs a reason in the pull request and its cost to a clean build. Put it once in the root `Cargo.toml` under `[workspace.dependencies]` with `default-features = false`. One version of every crate. No C or C++ that needs cmake. TLS is rustls with `ring`.
- Dependencies between crates go down only. `fos-core` uses no other fileOS crate; apps and engines depend only on `fos-app`; `fos` stays thin. The layout is in CONTRIBUTING.md under "Workspace layout".
- Crates with `portable = true` in `[package.metadata.fos]` must build for `wasm32-unknown-unknown`: no threads, files, sockets or clocks in them.
- Protocol logic is sans-IO, so it runs in the simulation in `fos-sim`.
- Mechanism, not policy: when code decides what should happen (whom to help, what to share, what to keep), make it a policy that can be replaced, with a default.
- Every source file starts with an SPDX header: `// SPDX-License-Identifier: AGPL-3.0-only`, or `MIT OR Apache-2.0` in `apps/fos-app` and `wit/`.
