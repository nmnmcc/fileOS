---
default: minor
---

# .app files and `fos app pack`

An app now comes as an `.app` file: a zip with no compression in the zip itself, with `manifest.toml` as it is (so tools can read it with no unpacking) and every other file in `payload.tar.zst`. `fos app pack <folder> <file>` makes one from an `.app` folder, with no running node; the same folder always gives the same file. `fos node init --engine` and `fos app install` take an `.app` file, or an `.app` folder while you make it.
