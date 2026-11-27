---
default: minor
---

# Apps share files with each other

An app can offer a file to another app with the new `fileos:app/access.share`, when it has `share.send` on the file. You pick the app with `fileos app answer <id> <app>`; it must take that kind of file (`[[receives]]` in its manifest). It gets read access to that one file only, at a path of its own. The photos app can share a photo, and the chat app takes pictures.
