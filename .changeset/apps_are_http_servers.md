---
default: minor
---

# Apps are HTTP servers

Every app is now an HTTP server for its own origin, `http://<app-id>.localhost:7341/`, with the standard `wasi:http` (WASI 0.2). `fileos app install` prints the address. Each request gets a new instance of the app, with limits on fuel, memory and time. The kernel takes `cookie`, `authorization` and `fileos-user` away from requests and `set-cookie` away from responses. Apps have no files, sockets or outgoing HTTP of their own. The SDK has `fileos_app::http::serve`, and the photos app gives a page with the photos it found.
