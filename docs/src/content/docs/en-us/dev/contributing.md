---
title: Contributing
description: Set up a development environment and send a change.
---

Discussion is as welcome as code: open an issue to question a principle, a
design decision or anything on this page. The full rules are in
`CONTRIBUTING.md` in the repository; this page is the short version.

## Set up

We recommend [devenv](https://devenv.sh) with [direnv](https://direnv.net),
but any setup works; only CI decides whether a change is accepted.

```sh
direnv allow     # once, in the repository
check            # before pushing: formatting, lints, tests, licenses
budget           # the compile-time budget
docs             # preview this site
```

Without Nix: install Rust with rustup (`rust-toolchain.toml` picks the
version), `cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`,
and [uv](https://docs.astral.sh/uv/) for the scripts (`uv sync`). For the
documentation, Node.js 22.23.4 or later: `npm install` and `npm run dev` in
`docs/`.

## Compile time is a feature

An ordinary laptop should be enough to contribute, so CI enforces a build
budget on every pull request, measured on a 4-core laptop with 8 GB of RAM:

| Measure                            | Budget  |
| ---------------------------------- | ------- |
| Clean debug build of the workspace | ≤ 180 s |
| Rebuild after editing a leaf crate | ≤ 5 s   |
| Rebuild after editing `hs-core`    | ≤ 20 s  |

A pull request over budget is treated like one that fails the tests.

Every new dependency is reviewed. Say why it is needed and what it adds to
the clean build; judge it by build time, binary size and what it brings in
(a second crypto library or HTTP stack, heavy procedural macros, build
scripts, C or C++), not by crate count. Declare it once in the root
`Cargo.toml` with `default-features = false`. `async-trait` is banned; no C
or C++ that needs cmake.

## Shape of the code

- Small crates and a shallow graph. Dependencies flow downward only;
  `hs-core` depends on no other homeserver crate and `hs` stays thin.
- Crates marked `portable = true` must build for `wasm32-unknown-unknown`:
  no threads, files, sockets or clocks in them.
- Protocol logic is sans-IO, so the same code runs in the deterministic
  simulation (`hs-sim`).
- Mechanism, not policy: when code decides what should happen (whom to help,
  what to share, what to keep), make it a replaceable policy with a default.

The crate layout is in [Architecture](../architecture/#crates).

## Language

All English that the project puts out (documentation, help, error messages,
comments, commit messages, release notes) is in
[Basic English](https://en.wikipedia.org/wiki/Basic_English): short
sentences and common words, so that it is easy to read for everyone and goes
well through machine translation. We suggest it for your issues and pull
requests too, but it is not a rule; Chinese is also good.

## Sending a change

Nobody pushes to `main` directly. Make a branch, open a pull request, and
wait for the `ci` check to be green. Pull requests are merged with "Squash
and merge", so `main` gets one commit for each pull request.

If users will see the change, make a change file with
`knope document-change` and commit it with the pull request. Do not make
changes to `CHANGELOG.md` by hand: at release time,
[Knope](https://knope.tech) puts the change files into it and the Release
workflow makes the tag and the GitHub release. The steps are in
`CONTRIBUTING.md`.

## Licensing and sign-off

homeserver is AGPL-3.0-only; `hs-plugin-api` is MIT OR Apache-2.0 so that
plugins can use any license. Start every source file with an SPDX header.
There is no CLA: you keep your copyright.

Every commit must be signed off under the
[Developer Certificate of Origin](https://developercertificate.org/):
`git commit -s`.

## Documentation

This site is built with [Astro Starlight](https://starlight.astro.build) in
`docs/`. English pages are in `docs/src/content/docs/en-us/`, Simplified
Chinese pages in `docs/src/content/docs/zh-cn/`, with the same file names.
Pages are grouped by audience: `user/`, `admin/` and `dev/`, each a topic in
the switcher at the top of the sidebar. A new page goes in the matching
directory in both languages and in the `topics` list in
`docs/astro.config.mjs`.

Write for the reader of that manual. The user manual assumes no knowledge of
servers; the administration manual assumes a terminal; only the developer
manual talks about crates and protocols. Describe what the current release
does; designs that are not built yet belong in [Architecture](../architecture/).
