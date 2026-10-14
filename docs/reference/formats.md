---
title: Formats
description: What pito-sync writes to disk and promises to readers: the database file, the sync table, the user_version stamp and the stored error text.
sidebar:
  order: 2
---

pito-sync writes one SQLite database per app. Everything here is what the
crate itself writes; the app's own tables are the app's.

## The files

| File | Mode | Written by |
|---|---|---|
| the `Spec`'s path | 0600 | `open`, created on first use, corrected on every open |
| its folder | 0700 | `open`, created if missing, corrected on every open |
| `<path>-wal`, `<path>-shm` | as SQLite makes them | SQLite, while the database is open in WAL mode |

The writing connection runs with `journal_mode=WAL`, `synchronous=NORMAL`,
`busy_timeout=5000` and `foreign_keys=ON`. The UI's connection is opened
read-only.

## The `sync` table

The crate creates one table of its own next to the app's tables:

```sql
CREATE TABLE sync (
  source TEXT PRIMARY KEY,
  started_at INTEGER,
  finished_at INTEGER,
  ok INTEGER,
  error TEXT,
  cursor TEXT,
  rows INTEGER,
  retry_after INTEGER
)
```

One row per source, keyed by the source's name. Times are milliseconds since
the Unix epoch.

| Column | Meaning |
|---|---|
| `source` | the source's name |
| `started_at` | when the latest pass began; set by `started` before every pass |
| `finished_at` | when the latest pass ended, by `commit` or `failed`; a pass is running while `started_at > finished_at` |
| `ok` | 1 when the latest pass worked, 0 when it failed, NULL before any pass ended |
| `error` | the latest failure as `kind: message`, NULL after a commit with no failure |
| `cursor` | the string the latest `commit` left for the next pass |
| `rows` | how many rows the latest `commit` wrote |
| `retry_after` | a time before which timed passes don't run, NULL when nothing holds them |

A pass that returns `Err` is recorded by `failed`, which sets `ok`,
`error`, `retry_after` and `finished_at` only: `rows`, `cursor` and the app's
data stay as the last `commit` left them. `finished_at` is never set before `started_at`.

The `Sync` struct that `sync(conn, source)` returns carries every column
except `retry_after`, which `retry_after(conn, source)` reads.

## The stored error text

`error` holds `Failure::text()`: the fault's word, a colon and a space, and
the message.

```
unreachable: the server did not answer
```

The words are `no_key`, `refused`, `forbidden`, `unreachable`, `bad_answer`
and `rate_limited`. `Fault::parse` splits the text at the first `": "` and
returns the fault and the message, or `None` for text that doesn't start
with one of the words.

## The `user_version` stamp

SQLite's `PRAGMA user_version` holds `version * 1000 + SYNC_VERSION`, where
`version` is the `Spec`'s and `SYNC_VERSION` is the version of the `sync`
table, 1 today. `spec.user_version()` computes it, and `version(conn)` reads
it from a connection.

| `Spec` version | Stamp |
|---|---|
| 0 | 1 |
| 1 | 1001 |
| 2 | 2001 |

When the stamp in the file differs from the `Spec`'s, `open` deletes the
file with its `-wal` and `-shm` and builds it again, and `reader` returns
`None` until it has. A `Spec` version must be between 0 and `MAX_VERSION`,
2147482, so the stamp fits in SQLite's 32-bit `user_version`.

## Secrets

No secret is ever written to the file, a log, an error or `Debug` output by
the crate. A `Secret` prints as `Secret(hidden)`.
