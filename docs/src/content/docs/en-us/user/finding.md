---
title: Finding things
description: Search, the timeline, places, links and live changes.
---

These commands look across every space you can read.

## Search

```sh
hs search park
hs search 公园
```

Search covers titles and text, ranked by relevance. Chinese, Japanese and
Korean text is found by two-character pieces, so `公园` finds
`人民公园`.

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

Latitude first, then longitude. Photos with GPS data and items with a
`place` are found.

## Links

```sh
hs related family person/mum
```

Lists what links to an item and what it links to: everything with a person,
everything at a place, everything in a project.

## Changes as they happen

```sh
hs watch
```

Prints one line per change in any space you can read, until you press
Ctrl-C. Useful to see a friend's new photos arrive, or in a script.
