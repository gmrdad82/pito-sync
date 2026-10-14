---
title: Read from the UI
description: Open a read-only connection, re-read on each Changed, and show syncing… while a pass runs.
sidebar:
  order: 2
---

The UI never writes. It reads through its own read-only connection and reads
again whenever a source says something changed. Every database is opened in
WAL mode with `synchronous=NORMAL`, so a read never queues behind a source's
commit.

![Paired bars, lower is better: UI read times while a source writes 10,000-row passes back to back. Median 0.047 ms with pito-sync against 0.073 ms on SQLite's defaults, 1.6× faster; p99 0.194 ms against 1233.670 ms, 6362× faster; slowest 2.840 ms against 1233.670 ms, 434× faster.](../images/busy-writer-paired.png)

In the chart a source commits 10,000-row passes back to back for three
seconds while the UI reads a page of 50 rows and the sync row every
millisecond. With pito-sync the UI got 2,305 reads, the slowest 2.84 ms. The
same code on a plain connection with SQLite's defaults (rollback journal,
`synchronous=FULL`) got 88 reads, the slowest stuck behind the writer for
1.23 s.

## Open a reader

```rust
let Some(ui) = pito_sync::reader(&spec)? else {
    return Ok("not built yet".into());
};
```

`reader(&spec)` returns `Ok(None)` until a source has built the file, and
while the version stored in the file differs from the `Spec`'s, which happens
for a moment after an upgrade, before the source's thread rebuilds it. It
returns `Err` only when the file exists and can't be read.

## Re-read on each `Changed`

`spawn` and `spawn_checked` take the sending half of a channel. The UI owns
the receiving half and reads again on every `Changed(name)`:

```rust
for Changed(name) in changes {
    println!("{name:<9} {}", show(&spec, &name)?);
}
```

A desktop or terminal app does the same in its event loop, with `try_recv`
on the channel so the loop never blocks.

## Show "syncing…"

A pass sends `Changed` when it starts and again when it ends. While it runs,
the source's `sync` row has `started_at` later than `finished_at`:

```rust
let sync = pito_sync::sync(&ui, name)
    .map_err(|e| e.to_string())?
    .unwrap_or_default();
if sync.started_at > sync.finished_at {
    return Ok("syncing…".into());
}
```

`sync(&conn, source)` returns `None` for a source that has never run;
`unwrap_or_default()` turns that into an empty row.

## Read your own tables

The rest is plain SQL on your own tables through the same connection:

```rust
let (posts, newest): (i64, Option<String>) = ui
    .query_row(
        "SELECT count(*), (SELECT title FROM posts ORDER BY id DESC LIMIT 1) FROM posts",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|e| e.to_string())?;
```

With the [failure check](report-a-failure.md) added, this is the `show`
function of the `feed` example, and these are the lines it prints:

```
feed      syncing…
feed      6 posts, newest "One file per app"
feed      syncing…
feed      unreachable: the server did not answer (6 posts kept)
```
