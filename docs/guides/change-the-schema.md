---
title: Change the schema
description: Bump the Spec's version when the app's tables change; the file is a cache, dropped and rebuilt, never migrated.
sidebar:
  order: 8
---

The database is a cache of what the sources fetched, never the only copy of
anything. So it is never migrated: when the tables change, the app bumps the
version in its `Spec`, and the next `open` deletes the file and builds it
fresh. The sources fill it again on their first pass.

## Bump the version

```rust
const SCHEMA: &[&str] = &[
    "CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL, author TEXT)",
];

let spec = Spec::new(state_dir().join("feed/feed.db"), 2, SCHEMA);
```

Version 1 had no `author`; version 2 adds it. Nothing else changes in the app.

## What happens on the next start

1. `reader(&spec)` returns `Ok(None)` while the file still carries the old
   version, so the UI shows its "not built yet" state rather than reading
   tables that have changed.
2. The source's thread calls `open(&spec)`, which sees the old version,
   deletes the file with its `-wal` and `-shm`, and builds the new schema.
   That took 144 ms in the measured run, once per install or version.
3. The first pass runs at once and fills the new tables, and the UI reads
   again on its `Changed`.

`open` rebuilds the same way when the file isn't a database at all, for
example after it was cut short.

## The stored stamp

The version check reads SQLite's `user_version`. pito-sync stores
`spec.version * 1000 + SYNC_VERSION` there, which `spec.user_version()`
returns. `SYNC_VERSION` is the version of the crate's own `sync` table, so a
change on either side rebuilds the file: the app's tables or the crate's.
Version 2 with `SYNC_VERSION` 1 is stored as 2001.

A version runs from 0 to `MAX_VERSION`, 2147482, so the stamp fits SQLite's
32-bit `user_version`. Anything else is refused:

```
tmp/feed.db: version -1 is outside 0..=2147482
```

## Data that must survive

Anything the user made, and anything that can't be fetched again, belongs in
a store that migrates, not in this file. See
[formats](../reference/formats.md) for exactly what pito-sync writes.
