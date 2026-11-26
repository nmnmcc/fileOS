---
default: minor
---

# fileOS: a file system with apps and rules in WebAssembly

A first prototype of fileOS. The core is only a file system: folders are
signed logs, and files are blobs named by their hash. A shared folder is a
folder that a number of nodes each agree to keep. Nodes talk over libp2p.

The rules of a node are an engine, and features are apps. Both are
WebAssembly components in `.app` folders, run with wasmtime. The default
engine (`Capability.app`) and a photos app (`Photos.app`) come with it.

The program is `fos`: `fos init`, `fos run`, and commands for files, folders,
sharing and apps.
