---
default: patch
---

# Apps are workers

Each change event now gets a new instance of the app, as each HTTP request already did. Nothing stays in memory between events: an app keeps what it needs in files or in its cache. A broken app is still found when it is installed.
