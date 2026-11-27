---
default: minor
---

# Apps ask, and you pick

An app can now ask you for access at run time, with the new `fos:app/access.ask` import, within the `[[asks]]` of its manifest. `fos app requests` lists what apps ask for, `fos app answer <id> <path>` gives it, and `fos app refuse <id>` says no. The app sees what you gave at `/granted/<handle>`, never where it really is. The photos app uses this to show the photos in a folder you pick.
