---
title: "Data in apps: good patterns"
description: How to put complex data into files so that it syncs and merges well, for people who write fileOS apps.
---

:::note[Status: design]
These patterns go with the new data design (see [Data](../data/)). Some of
the building blocks named here (CRDT files, cache files, short messages) are not
built yet.
:::

In fileOS everything is a file, and every key merges by itself. Writes to
the same key at the same time become versions side by side. So one rule is
behind all the patterns on this page:

**Make the unit of merging the same as the unit that people change.**
Changes that have nothing to do with each other go to different keys.
Changes that must merge go into a CRDT.

This is an old idea that works: Maildir keeps one mail in one file, and it
has been one of the safest mail formats to sync for decades.

## Ten patterns

1. **One record, one file, named by a fixed ID.** Use
   `/contacts/01J9X….cbor`, not one large `contacts.json`. Then a conflict
   touches only one record, change events name the record, access works by
   prefix, and sync sends only what changed.
2. **IDs are random, or random and ordered by time** (UUIDv7, ULID), or the
   hash of the content. Never a counter: two offline devices give out the
   same number. Names for people are fields, not paths, so a rename changes a
   field and moves nothing.
3. **Fields that people change one at a time:** keep the record as a small
   CRDT document (a Loro map), or put each field in its own key
   (`/contacts/<id>/name`). A record that one person changes as a whole can be
   a plain file.
4. **Events, feeds, chat and counts are directories of record files.** Each
   record is one small file, named by a time-sorted ID and its author, so
   names never meet and sort by time. Records never conflict. The state now is
   worked out from all the records (event sourcing). A counter is a set of
   "+1" files, never `x = x + 1`, which loses updates when two devices do it at
   the same time.
5. **A set is a directory of keys.** An album's photos are
   `/albums/<album>/items/<photo-id>`: the key is there or it is not. Adding
   and taking away are separate keys, so they merge. Not an array of IDs in
   one file.
6. **An ordered list** uses fractional indexing in the key
   (`/playlist/<position>-<id>`), where a new item can go between any two, or a
   Loro movable list when items are moved a lot.
7. **A tree whose nodes move** is not put into paths. Each node has a "parent
   ID" field. When moves are frequent (an outline), use a Loro movable tree,
   which deals with concurrent moves that would make a cycle.
8. **Large binary data is its own file.** A record keeps its path or hash.
   Large files are cut into chunks that versions share. Never put images or
   video inside records.
9. **Queries and indexes go into cache files, and SQLite is good there.** The
   files are the truth: they sync, merge, keep versions and have access. An
   index is made from them, on each node only. An app can keep a SQLite
   database in its cache, update it on change events, and use SQL for
   queries. If it is lost, it is made again from the files. It never syncs, so
   it never conflicts. You get SQL without the problem that SQL databases do
   not merge.
10. **Formats are open, describe themselves, and have a version.** Where there
    is a standard format, use it: vCard for contacts, iCalendar for events,
    Markdown with front matter for notes, XMP sidecar files for photo data.
    The data then lives longer than the app, other apps can read it, and
    CalDAV and CardDAV adapters can use it. Your own formats (CBOR or JSON)
    have a `schema` version and only ever get new fields. When reading, skip
    fields you do not know; when writing, keep them as they are, so an older
    app does not delete what a newer one wrote.

## Things not to do

- **Do not depend on rules that merging cannot keep.** Design them away: an ID
  in place of a unique name, a set in place of a limited count. When you
  must, check them in the merge hook, or use the authority path, but seldom.
- **Do not put data with different sharing in one file.** Access goes by path
  prefix, and half a file cannot be shared. Private notes and notes shared
  with the family go under different prefixes, or different spaces.
- **Do not keep short-lived state in files that are kept.** Cursor positions,
  "is typing" and "is online" are not data: they go as **short messages**
  between the nodes that are online, which are never stored (this channel
  comes later).
  Otherwise they are data that grows for nothing.
- **Do not write the same key very often**, such as on every key press. Put
  frequent changes into a CRDT document (which packs operations together), or
  keep them in memory for a short time and then write.

## Which structure for which data

| Shape of the data                        | Structure                                 | Examples                   |
| ---------------------------------------- | ----------------------------------------- | -------------------------- |
| Records that do not depend on each other | One file each, named by ID                | Contacts, bookmarks, tasks |
| Records that several people change       | A small CRDT document each                | Shared task cards          |
| Events that are only added               | A directory of record files               | Chat, feeds, logs, counts  |
| A set                                    | Keys in a directory                       | Album members, tags        |
| An ordered list                          | Fractional index keys, or a movable list  | Playlists, board columns   |
| A tree that changes shape                | Parent fields, or a movable tree          | Outlines                   |
| Rich text                                | A text CRDT, with history                 | Documents, note bodies     |
| Large binary data                        | Its own chunked file, plus a small record | Photos, video              |
| Data to search                           | A SQLite index in cache files             | Search, filters, totals    |
| Short-lived state                        | Short messages, not stored                | Cursors, presence          |

## Examples

- **Contacts:** `/contacts/<id>.vcf`, one vCard file for each person; groups as
  keys under `/contacts/groups/<group>/<id>`; search with SQLite in cache files.
- **Notes:** `/notes/<id>.md` in Markdown. For notes that several people edit
  at the same time, the body is Loro text, written out as Markdown for export.
  Nesting uses a parent field.
- **Tasks:** one file for each task; the order uses fractional indexes; "how
  many are done" is not stored, but worked out from the files.
- **Chat:** one directory for each conversation, one file for each message; where each person has read up to
  is a small file of their own; "is typing" is a short message.

## Help from the SDK

The `fileos-app` SDK is to give these, so that each app does not make them
again: records by ID (with new IDs, a schema version, and unknown fields
kept), fractional indexes, Loro documents, and a helper that keeps a SQLite
index in cache files up to date with files.
