---
default: minor
---

# Apps ask, and you pick

An app can now ask you for access at run time, with the new `fileos:app/access.ask` import, within the `[[asks]]` of its manifest. `fileos app requests` lists what apps ask for, `fileos app answer <id> <path>` gives it, and `fileos app refuse <id>` says no. The app sees what you gave at `/granted/<handle>`, never where it really is. The photos app uses this to show the photos in a folder you pick.
