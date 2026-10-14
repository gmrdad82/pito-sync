---
title: Install
description: Add pito-sync to a Rust project on Linux, macOS or WSL, and run the feed example.
sidebar:
  order: 1
---

pito-sync is a library. It ships no binaries: you add it to your
`Cargo.toml` and build it with your app.

## Add the dependency

```toml
[dependencies]
pito-sync = { git = "https://github.com/gmrdad82/pito-sync", tag = "v1.0.1" }
```

The crate re-exports `rusqlite` 0.40 with SQLite compiled in. Use
`pito_sync::rusqlite` in your code rather than adding `rusqlite` yourself, so
your app and the crate stay on the same version.

## What you need on each platform

pito-sync needs Rust 1.97 or newer and a C compiler, since SQLite is compiled
in.

### Linux

Install a C toolchain from your distribution:

```
sudo apt install build-essential    # Debian, Ubuntu
sudo pacman -S base-devel           # Arch
```

On other distributions, install `gcc`.

### macOS

Install the Xcode command line tools:

```
xcode-select --install
```

### Windows

Use WSL and follow the Linux steps inside it. Native Windows isn't supported,
because the crate sets Unix file permissions on the database.

## Quick start

Clone the repository and run the `feed` example, the clip on the
[overview](index.md):

```
git clone https://github.com/gmrdad82/pito-sync && cd pito-sync
cargo run --example feed
```

It prints the database path, then every `Changed` the UI receives, with what
the UI reads at that moment. Your home folder shows in full where this page
writes `~`:

```
database  ~/.local/state/pito-sync-feed/feed.db
feed      syncing…
feed      no key: sign in first (0 posts kept)
feed      syncing…
feed      3 posts, newest "Threads that never block the UI"
feed      syncing…
feed      6 posts, newest "One file per app"
feed      syncing…
feed      unreachable: the server did not answer (6 posts kept)
feed      syncing…
feed      9 posts, newest "The sync table"
```

The source starts with no token, so its first pass fails with `no key`. The
app then hands it a token, which starts a pass, and asks for three resyncs.
The second one is told to fail as if the server were down, and the posts from
the pass before it stay. Run it again and the first line reads
`no key: sign in first (9 posts kept)`: the file from the first run is still
there, with its posts.

## The smallest app

This is the whole idea in one file: a source that writes posts on its own
thread, and a UI loop that only reads.

```rust
use std::sync::mpsc;
use std::time::Duration;

use pito_sync::rusqlite::Connection;
use pito_sync::{Changed, Failure, Fault, Pass, Source, Spec};

const SCHEMA: &[&str] = &["CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL)"];

struct Feed;

impl Source for Feed {
    fn name(&self) -> &str {
        "feed"
    }

    fn every(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }

    fn pass(&mut self, conn: &mut Connection, _id: Option<&str>) -> Result<usize, Failure> {
        let titles = ["Hello, world"];
        let pass = Pass { source: "feed", started_at: 0, finished_at: 0, failure: None, cursor: None };
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

fn main() -> Result<(), String> {
    let spec = Spec::new("state/feed.db", 1, SCHEMA);
    let (changed, changes) = mpsc::channel::<Changed>();
    let _asks = pito_sync::spawn_checked(&spec, Box::new(Feed), changed)?;
    for Changed(name) in changes {
        let ui = pito_sync::reader(&spec)?.ok_or("not built yet")?;
        let sync = pito_sync::sync(&ui, &name).map_err(|e| e.to_string())?;
        println!("{name}: {sync:?}");
    }
    Ok(())
}
```

Next, [write a source](guides/write-a-source.md) of your own.
