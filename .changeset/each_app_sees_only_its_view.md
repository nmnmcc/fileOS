---
default: minor
---

# Each app sees only its own view

The paths an app uses are now in its own view. Each permission in its manifest is mounted at the name the manifest gives, so apps keep working. Every other path is the app's private data, in a space its key owns. `fileos app grant <app> <path> <permissions> [--as <name>]` gives an app more at run time, at `/granted/<handle>` or at a name you pick; `fileos app grants <app>` shows the view, and `fileos app take <app> <name>` takes a part away with its grant. The app never sees where its files really are.
