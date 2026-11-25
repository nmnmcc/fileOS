---
title: Spaces and items
description: Add, read, change and remove the things you keep on your homeserver.
---

Commands take the space first (its name or ID), then the item as `TYPE/ID`.
Use types that say what the item is: `note`, `contact`, `event`, `recipe`.
Search and the timeline understand a small number of common fields; see the
list below.

## List and read

```sh
hs ls notes                 # what is in the space "notes"
hs ls notes note            # only notes
hs ls notes --all           # everything, page by page
hs get notes note/shopping
hs get photos file/3f2a… -o beach.jpg
```

## Write

`hs put` makes a new item, or puts a new one in place of an old one. It
reads from a file, or from standard input:

```sh
echo '{"title": "Shopping", "text": "milk, eggs"}' | hs put notes note/shopping
hs put notes recipe/bread bread.md
```

The media type comes from the file name; with no file name, it is JSON. You
can set it with `--media-type`. Two flags make changes safe: `--new` fails
if the item is already there, and `--if-match ETAG` fails if someone changed
the item after you read it.

Two simpler ways to change an item:

```sh
hs edit notes note/shopping                 # opens $EDITOR; a new ID starts empty
hs patch notes note/shopping '{"done": true}'   # change some fields of a JSON item
```

`hs patch` takes a JSON merge patch. Fields you give are set; fields set to
`null` are removed; all other fields stay as they are.

## Fields that search and the timeline understand

```json
{
  "title": "Weekend",
  "text": "A walk in the park with Mum",
  "time": "2024-05-01T09:30:00Z",
  "place": { "lat": 31.23, "lon": 121.47 },
  "links": [{ "rel": "with", "href": "person/mum" }]
}
```

- `title` (or `name`) and `text` are searched.
- `time` puts the item on the timeline.
- `place` lets [`hs near`](../finding/) find it.
- `links` connect it to people, places and other items, for
  [`hs related`](../finding/).

## Files and photos

```sh
hs upload photos IMG_0001.jpg IMG_0002.jpg
```

Each file becomes a `file/…` item. Its name comes from its content, so if
you upload the same file two times, nothing changes. For photos, `hs` reads
the time, place and camera from the EXIF data. It makes a thumbnail (a small
picture) for JPEG and PNG.

## Remove

```sh
hs rm notes note/shopping
hs rm notes note/shopping --erase
```

A plain `rm` deletes the item but keeps its history, like any other change.
`--erase` also removes its older versions from the logs of every member who
follows the space. If someone has already made a copy, you cannot take it
back.
