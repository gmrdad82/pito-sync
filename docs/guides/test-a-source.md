---
title: Test a source
description: Run a source's pass against an in-memory database with memory and pass_once, with no thread and no file.
sidebar:
  order: 9
---

A test needs no thread and no file. `memory(&spec)` builds the app's schema,
with the crate's `sync` table, in an in-memory database. `pass_once` runs one
pass on the calling thread exactly as the runner does: it marks the pass
started, calls `pass`, and records a failure.

## A test that a failed pass keeps the posts

```rust
use std::time::Duration;

use pito_sync::rusqlite::Connection;
use pito_sync::{Failure, Fault, Pass, Source, Spec};

const SCHEMA: &[&str] = &["CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL)"];

struct Feed {
    titles: Vec<String>,
    down: bool,
}

impl Source for Feed {
    fn name(&self) -> &str {
        "feed"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn pass(&mut self, conn: &mut Connection, _id: Option<&str>) -> Result<usize, Failure> {
        if self.down {
            return Err(Failure::new(Fault::Unreachable, "the server did not answer"));
        }
        let titles = &self.titles;
        let pass = Pass { source: "feed", started_at: 1, finished_at: 2, failure: None, cursor: None };
        pito_sync::commit(conn, pass, |tx| {
            tx.execute("DELETE FROM posts", [])?;
            for title in titles {
                tx.execute("INSERT INTO posts (title) VALUES (?1)", [title])?;
            }
            Ok(titles.len())
        })
        .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}

#[test]
fn a_failed_pass_keeps_the_posts() {
    let spec = Spec::new("unused.db", 1, SCHEMA);
    let mut conn = pito_sync::memory(&spec).unwrap();
    let mut feed = Feed { titles: vec!["Hello, world".into(), "WAL in one page".into()], down: false };

    assert_eq!(pito_sync::pass_once(&mut conn, &mut feed, None), Ok(2));

    feed.down = true;
    let failure = pito_sync::pass_once(&mut conn, &mut feed, None).unwrap_err();
    assert_eq!(failure.fault, Fault::Unreachable);

    let sync = pito_sync::sync(&conn, "feed").unwrap().unwrap();
    assert_eq!(sync.ok, Some(false));
    assert_eq!(sync.error.as_deref(), Some("unreachable: the server did not answer"));
    assert_eq!(sync.rows, Some(2));
    let posts: i64 = conn.query_row("SELECT count(*) FROM posts", [], |row| row.get(0)).unwrap();
    assert_eq!(posts, 2);
}
```

`memory` ignores the `Spec`'s path. After the failed pass, the source's
`sync` row reads:

```
Sync { source: "feed", started_at: Some(1792004717028), finished_at: Some(1792004717028), ok: Some(false), error: Some("unreachable: the server did not answer"), cursor: None, rows: Some(2) }
```

`ok` is false and `error` holds the failure, while `rows` and the posts are
still the first pass's two.

## Feed the source fake answers

Keep the network out of `pass` behind a field the test can set, as `down`
and `titles` do here, or behind a trait the source holds. The test then
drives every branch the UI cares about: a missing key, each `Fault`, a
`retry_after`, and a pass that works after one that failed.

## Test the timing rules

`due(&conn, source, every, now)` is the check the runner makes before a
timed pass, with `now` in milliseconds. A test can call it with any `now` to
see when the next timed pass would run, with no clock and no thread.
