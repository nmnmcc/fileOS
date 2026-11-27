---
default: minor
---

# Schedules for apps

An app can list `[[schedule]]` entries in its manifest, with a `name` and how often (`every = "1d"`). At those times the kernel sends `POST /.well-known/fos/schedule/<name>` with `fos-event: schedule` to the app's own HTTP handler, so nothing new is needed in the app interface. Requests from outside never reach `/.well-known/fos/`. The photos app indexes all its photos again once a day.
