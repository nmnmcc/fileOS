---
title: Finding things
description: Search, the timeline, places, links and changes as they happen.
---

These commands look in every space you can read.

## Search

```sh
hs search park
hs search 公园
```

Search looks at titles and text, and puts the best matches first. Chinese,
Japanese and Korean text is found by pieces of two characters, so `公园`
finds `人民公园`.

## Timeline

```sh
hs timeline
hs timeline --before 2024-06-01 --limit 20
```

Items with a `time` (photos with EXIF data, notes, events) are listed newest
first. `--before` takes a day in UTC.

## Places

```sh
hs near 31.23 121.47            # within 1 km
hs near 31.23 121.47 --km 5
```

Give the latitude first, then the longitude. `hs near` finds photos with GPS
data and items with a `place`.

## Links

```sh
hs related family person/mum
```

This lists what links to an item and what it links to: everything with a
person, everything at a place, everything in a project.

## Changes as they happen

```sh
hs watch
```

This prints one line for each change in any space you can read, until you
press Ctrl-C. Use it to see a friend's new photos come in, or in a script.
