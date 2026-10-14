---
title: Write a source
description: Implement the Source trait and run it on its own thread with spawn or spawn_checked.
sidebar:
  order: 1
---

A source is the part of your app that fetches data, from a server, a folder
or another program, and writes it into the database. pito-sync runs each
source on its own thread with its own connection, so the UI never waits on it.

## Implement `Source`

```rust
pub trait Source: Send {
    fn name(&self) -> &str;
    fn every(&self) -> Option<Duration>;
    fn pass(&mut self, conn: &mut Connection, id: Option<&str>) -> Result<usize, Failure>;
    fn secret(&mut self, name: &str, value: Secret) {}
}
```

- `name` keys the source's row in the `sync` table. It is any string the app
  builds; two sources in one app need two names.
- `every` is how often a timed pass runs, or `None` to run only at start and
  when asked.
- `pass` does one sync: fetch, then write the rows in one transaction with
  [`commit`](commit-a-pass.md). It returns how many rows it wrote, or a
  [`Failure`](report-a-failure.md).
- `secret` receives a secret the app hands in later. Its default drops it;
  see [hand a source a secret](hand-a-source-a-secret.md).

A source that rewrites a table of posts on every pass:

```rust
use std::time::Duration;

use pito_sync::rusqlite::Connection;
use pito_sync::{Failure, Fault, Pass, Source};

struct Feed;

impl Source for Feed {
    fn name(&self) -> &str {
        "feed"
    }

    fn every(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }

    fn pass(&mut self, conn: &mut Connection, _id: Option<&str>) -> Result<usize, Failure> {
        let titles = fetch_titles()?;
        let pass = Pass { source: "feed", started_at: now(), finished_at: now(), failure: None, cursor: None };
        pito_sync::commit(conn, pass, |tx| {
            tx.execute("DELETE FROM posts", [])?;
            for title in &titles {
                tx.execute("INSERT INTO posts (title) VALUES (?1)", [title])?;
            }
            Ok(titles.len())
        })
        .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}
```

`fetch_titles` and `now` stand for your own code: the network call, and the
time in milliseconds since the epoch.

## Start it

```rust
let spec = Spec::new(path, 1, SCHEMA);
let (changed, changes) = std::sync::mpsc::channel::<Changed>();
let asks = pito_sync::spawn_checked(&spec, Box::new(Feed), changed)?;
```

`spawn_checked` opens the database on the calling thread. If the file can't
be opened it returns `Err` with the reason (`cannot write <folder>: …`,
`cannot open <file>: …`) and starts nothing; otherwise it moves the open
connection into a new thread. `spawn` does the open on the new thread
instead and returns at once, so a file that can't be opened shows only as one
`Changed` and a source that never passes. Prefer `spawn_checked` when the app
can report the error.

Both return `asks`, the source's inbox, where the app sends a
[`Resync`](ask-for-a-resync.md) or a [`Secret`](hand-a-source-a-secret.md).
Keep it: dropping it, or dropping the `Changed` receiver, ends the thread.

## What the thread does

- **The first pass runs at once**, as soon as the file is open. An app needs
  no start-up `Resync`. Only a `retry_after` left by an earlier run that still
  holds keeps it back.
- **Each pass sends `Changed(name)` twice**: when it starts and when it ends,
  whether it worked or failed. The UI shows "syncing…" in between with no
  extra wiring; see [read from the UI](read-from-the-ui.md).
- **Timed passes.** With `every` set, the thread wakes every `every`, or
  every minute when `every` is longer, and runs a pass when it's due: no
  `retry_after` holds it, and the last pass failed or finished at least
  `every` ago. A failed pass is tried again at the next wake.
- **On request.** A `Resync` or a `Secret` runs a pass at once.

The `feed` example in the repository starts a source this way and prints
each `Changed` as the UI sees it:

```
feed      syncing…
feed      no key: sign in first (0 posts kept)
feed      syncing…
feed      3 posts, newest "Threads that never block the UI"
```

## Run one pass without a thread

`pass_once(&mut conn, &mut source, id)` runs exactly what the thread runs for
one pass, on the calling thread: mark the pass started, call `pass`, and
record a failure. Use it for a headless dump or a test; see
[test a source](test-a-source.md).
