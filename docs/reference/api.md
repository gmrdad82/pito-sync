---
title: API
description: Every public type and function in pito-sync, in one page, with a link to the generated rustdoc.
sidebar:
  order: 1
---

Everything is at the crate root, `pito_sync::`. The generated rustdoc, with
every signature and type, is in [the API docs](../api/pito_sync/index.html).

## The database

| Item | What it does |
|---|---|
| `Spec { path, version, schema }` | an app's database: its file, its version, and the `CREATE` statements of its tables. `Spec::new(path, version, schema)` builds one |
| `Spec::user_version()` | the stamp stored in the file, `version * 1000 + SYNC_VERSION` |
| `open(&spec) -> Result<Connection, String>` | the writing connection: creates or rebuilds the file, private and in WAL; see [place the database](../guides/place-the-database.md) |
| `reader(&spec) -> Result<Option<Connection>, String>` | the UI's read-only connection, `None` until the file is built at this version |
| `memory(&spec) -> Result<Connection, String>` | the same schema in memory, for tests |
| `version(&conn) -> rusqlite::Result<i64>` | the stamp stored in an open database |
| `SYNC_VERSION`, `MAX_VERSION` | the `sync` table's version, 1, and the highest `Spec` version, 2147482 |
| `rusqlite` | the re-exported `rusqlite` 0.40, SQLite bundled |

## Sources and the runner

| Item | What it does |
|---|---|
| `trait Source: Send` | `name`, `every`, `pass`, and `secret` with a default that drops the value; see [write a source](../guides/write-a-source.md) |
| `spawn(&spec, source, changed) -> Sender<Message>` | runs a source on its own thread, opening the file there |
| `spawn_checked(&spec, source, changed) -> Result<Sender<Message>, String>` | opens the file on the calling thread first and returns its error, or starts the thread |
| `pass_once(&mut conn, &mut source, id) -> Result<usize, Failure>` | one pass on the calling thread, as the runner runs it |
| `Changed(String)` | sent with a source's name when a pass starts and when it ends |
| `Message::Resync(Resync)` | runs a pass at once; see [ask for a resync](../guides/ask-for-a-resync.md) |
| `Resync(String, Option<String>)` | a source's name and the id handed to `pass`; `.into()` makes the `Message` |
| `Message::Secret(String, Secret)` | hands a source a secret, then runs a pass; see [hand a source a secret](../guides/hand-a-source-a-secret.md) |

## Bookkeeping

| Item | What it does |
|---|---|
| `commit(&mut conn, Pass, write) -> rusqlite::Result<usize>` | a pass's rows and its `sync` row in one `BEGIN IMMEDIATE`; see [commit a pass](../guides/commit-a-pass.md) |
| `Pass { source, started_at, finished_at, failure, cursor }` | what `commit` records in the `sync` row |
| `started(&conn, source, at)` | marks a pass begun |
| `failed(&conn, source, &failure)` | records a failed pass and leaves the data alone |
| `sync(&conn, source) -> rusqlite::Result<Option<Sync>>` | reads a source's `sync` row back |
| `Sync { source, started_at, finished_at, ok, error, cursor, rows }` | that row |
| `retry_after(&conn, source) -> rusqlite::Result<Option<i64>>` | the time timed passes wait for, if any |
| `due(&conn, source, every, now) -> bool` | whether a timed pass should run at `now` |

The table's columns are in [formats](formats.md).

## Failures and secrets

| Item | What it does |
|---|---|
| `Fault` | `NoKey`, `Refused`, `Forbidden`, `Unreachable`, `BadAnswer`, `RateLimited`; `Fault::ALL` lists them |
| `Fault::word()`, `Fault::parse(&error)` | the stored word, and the stored text read back as a fault and a message |
| `Failure { fault, message, retry_after }` | a failed pass; `Failure::new(fault, message)`, `.until(Some(at))`, `.text()`; see [report a failure](../guides/report-a-failure.md) |
| `Secret` | `Secret::new(value)`, `expose()`; hidden in `Debug`, no `Clone` or `Display`, zeroed on drop |
