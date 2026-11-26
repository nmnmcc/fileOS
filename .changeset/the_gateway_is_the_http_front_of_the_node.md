---
default: minor
---

# The gateway is the HTTP front of the node, and `fos curl`

The gateway now listens on port 7341 (`fos node run --port` changes it) on `127.0.0.1` and `::1` only. Requests go by host name: `fos.localhost` is the kernel API, and `<app-id>.localhost` is an app. Other host names get `421 Misdirected Request`, so DNS rebinding does not work. The kernel API takes no cross-site requests from browsers, every answer has safe default headers, and apps can set no cookies. `fos curl` runs curl with the node's token, and sends every connection to the gateway, so the token never leaves the machine: `fos curl /v0/status`. The node also cleans up when it gets SIGTERM.
