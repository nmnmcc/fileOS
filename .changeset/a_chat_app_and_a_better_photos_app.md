---
default: minor
---

# A chat app, and a better photos app

fileOS now comes with a chat app. Its rooms are streams in `/chat`: share the folder with your friends' nodes, and each of you talks through the chat app at its own origin on your own node. The photos app now shows the photos in a gallery at its own origin and takes uploads into `/photos`. For tests and for nodes that send requests on, the runtime can give an app a request made of bytes and get back all of its answer.
