---
title: Commit a pass
description: Write a pass's rows and its sync row in one transaction, so the UI sees all of a sync or none of it.
sidebar:
  order: 3
---

A pass writes its rows and its bookkeeping row together, in one
`BEGIN IMMEDIATE` through `commit`. The UI sees both or neither, never half a
sync. Writing 10,000 rows this way took 3.61 ms in the measured run, 2.77
million rows a second.

```rust
let pass = Pass { source: "feed", started_at, finished_at: now(), failure: None, cursor: None };
pito_sync::commit(conn, pass, |tx| {
    tx.execute("DELETE FROM posts", [])?;
    let mut insert = tx.prepare_cached("INSERT INTO posts (title) VALUES (?1)")?;
    for title in &titles {
        insert.execute([title])?;
    }
    Ok(titles.len())
})
```

`commit(conn, pass, write)` opens the transaction, calls `write` with it,
writes the source's `sync` row and commits. If `write` returns an error,
nothing is committed and `commit` returns that error.

## The `Pass` fields

| Field | What it holds |
|---|---|
| `source` | the source's name, the key of its `sync` row |
| `started_at` | when the pass began, milliseconds since the epoch |
| `finished_at` | when it ended, the same unit |
| `failure` | `None` for a pass that worked; a `Failure` to record a partial pass with its fault |
| `cursor` | a string the source wants back next time, such as a page token or a last-seen id, or `None` |

`write` returns the number of rows the pass wrote, stored as the row's
`rows`. A success sets `ok = 1` and clears `error` and `retry_after`.

## Read the cursor back

The cursor is stored in the `sync` row. A source that syncs incrementally
reads it at the start of its next pass:

```rust
let since = pito_sync::sync(conn, "feed")
    .ok()
    .flatten()
    .and_then(|s| s.cursor);
```

Never put a secret in a cursor, an error or a row; see
[hand a source a secret](hand-a-source-a-secret.md).

## Writes come only from passes

The UI's connection is read-only, and a source's thread holds the only
writing connection for its file. Keep every write inside a pass, so each one
lands with its bookkeeping and the UI hears about it through `Changed`. For
data the user creates, which must never be dropped, use a separate store:
pito-sync's file is a cache that is rebuilt when its version changes, as
[change the schema](change-the-schema.md) explains.
