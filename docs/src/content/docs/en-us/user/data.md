---
title: Spaces and items
description: Add, read, change and remove what you keep on your homeserver.
---

Commands take the space first (its name or ID), then the item as `TYPE/ID`.
Pick types that describe what the item is: `note`, `contact`, `event`,
`recipe`. Search and the timeline understand a few common fields, listed
below.

## List and read

```sh
hs ls notes                 # what is in the space "notes"
hs ls notes note            # only notes
hs ls notes --all           # everything, page by page
hs get notes note/shopping
hs get photos file/3f2a… -o beach.jpg
```

## Write

`hs put` creates or replaces an item from a file, or from standard input:

```sh
echo '{"title": "Shopping", "text": "milk, eggs"}' | hs put notes note/shopping
hs put notes recipe/bread bread.md
```

The media type comes from the file name, or JSON when there is none; set it
with `--media-type`. To change an item safely, `--new` fails if it already
exists, and `--if-match ETAG` fails if someone changed it since you read it.

Two easier ways to change an item:

```sh
hs edit notes note/shopping                 # opens $EDITOR; a new ID starts empty
hs patch notes note/shopping '{"done": true}'   # change some fields of a JSON item
```

`hs patch` takes a JSON merge patch: fields you give are set, fields set to
`null` are removed, the rest stay as they are.

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
- `time` places the item on the timeline.
- `place` lets [`hs near`](../finding/) find it.
- `links` connect it to people, places and other items, for
  [`hs related`](../finding/).

## Files and photos

```sh
hs upload photos IMG_0001.jpg IMG_0002.jpg
```

Each file becomes a `file/…` item named by its content, so uploading the
same file twice changes nothing. For photos, the time, place and camera are
read from the EXIF data, and a thumbnail is made for JPEG and PNG.

## Remove

```sh
hs rm notes note/shopping
hs rm notes note/shopping --erase
```

A plain `rm` deletes the item but keeps its history, like any other change.
`--erase` also removes its earlier versions from the logs of every member
who follows the space. What someone already copied cannot be taken back.
