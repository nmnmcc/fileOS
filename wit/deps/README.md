<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

# WASI 0.2 interfaces

These are the WIT files of WASI 0.2.12 (`wasi:cli`, `wasi:clocks`,
`wasi:filesystem`, `wasi:http`, `wasi:io`, `wasi:random`, `wasi:sockets`), as
they come with `wasmtime-wasi-http` 51.0.0. We do not change them. They are
from the [WASI project](https://github.com/WebAssembly/WASI), under
Apache-2.0 WITH LLVM-exception.

`wit/fileos.wit` uses `wasi:http/incoming-handler`, so that apps are HTTP
servers. The other packages are here because `wasi:http` names them.
