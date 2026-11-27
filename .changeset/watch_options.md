---
default: minor
---

# Watch options

A `[[watch]]` in an app's manifest can now say `recursive` (on when not given), `filter` (name patterns with `*` and `?`, such as `*.jpg`) and `kinds` (`made`, `changed`, `deleted`). The app is told only of the changes it asks for. The photos app now hears only of image files.
