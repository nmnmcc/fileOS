---
title: Contributing
description: Set up a development environment and send a change.
---

Talk is as welcome as code. Open an issue to question a principle, a design
decision or anything on this page. The full rules are in `CONTRIBUTING.md` in
the repository; this page is the short form.

## Set up

We suggest [devenv](https://devenv.sh) with [direnv](https://direnv.net),
but any setup works. Only CI decides if a change is accepted.

```sh
direnv allow     # once, in the repository
check            # before pushing: formatting, lints, tests, licenses
budget           # the compile-time budget
docs             # preview this site
```

Without Nix: install Rust with rustup (`rust-toolchain.toml` picks the
version). Then run
`cargo install --locked cargo-deny cargo-machete cargo-llvm-lines`, and
install [uv](https://docs.astral.sh/uv/) for the scripts (`uv sync`). For the
documentation you need Node.js 22.23.4 or later: run `npm install` and
`npm run dev` in `docs/`.

## Compile time is a feature

An ordinary laptop should be enough to take part. So CI checks a build
budget on every pull request, measured on a 4-core laptop with 8 GB of RAM:

| Measure                            | Budget  |
| ---------------------------------- | ------- |
| Clean debug build of the workspace | ≤ 360 s |
| Rebuild after editing a leaf crate | ≤ 5 s   |
| Rebuild after editing `fos-core`   | ≤ 20 s  |

We treat a pull request over budget like one that fails the tests.

We review every new dependency. Say why it is needed and what it adds to the
clean build. Judge it by build time, binary size and what it brings in (a
second crypto library or HTTP stack, heavy procedural macros, build scripts,
C or C++), not by the number of crates. Declare it once in the root
`Cargo.toml` with `default-features = false`. `async-trait` is not allowed,
and neither is C or C++ that needs cmake.

## Shape of the code

- Small crates and a shallow graph. Dependencies only go down. `fos-core`
  depends on no other fileOS crate, and `fos` stays thin.
- Crates marked `portable = true` must build for `wasm32-unknown-unknown`, so
  they have no threads, files, sockets or clocks.
- Protocol logic is sans-IO (it does no input or output itself). So the same
  code runs in the deterministic simulation (`fos-sim`).
- Mechanism, not policy: when code decides what should happen (whom to help,
  what to share, what to keep), make it a policy with a default that can be
  changed for another.

The crate layout is in [Architecture](../fileos/#crate-layout).

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

fileOS is AGPL-3.0-only. The app SDK (`apps/fos-app`) and the interfaces
in `wit/` are MIT OR Apache-2.0, so that apps and engines can use any license. Start every source file with an SPDX header.
There is no CLA: you keep your copyright.

Every commit must be signed off under the
[Developer Certificate of Origin](https://developercertificate.org/):
`git commit -s`.

## Documentation

This site is built with [Astro Starlight](https://starlight.astro.build) in
`docs/`. English pages are in `docs/src/content/docs/en-us/`, and Simplified
Chinese pages are in `docs/src/content/docs/zh-cn/`, with the same file
names. Pages are grouped by reader: `user/`, `admin/` and `dev/`. Each group
is a topic in the switcher at the top of the sidebar. Put a new page in the
matching directory in both languages, and in the `topics` list in
`docs/astro.config.mjs`.

Write for the reader of that manual. The user manual takes it that the
reader knows nothing about servers. The administration manual takes it that
the reader can use a terminal. Only the developer manual talks about crates
and protocols. Say what the current release does; designs that are not built
yet go in [Architecture](../fileos/).
