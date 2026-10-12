<p align="center"><img src="docs/images/feed.gif" width="800" alt="A terminal: the feed example prints the database path, then the feed source's passes as the UI sees them: syncing…, no key: sign in first, then after a secret arrives 3 posts, 6 posts, an unreachable pass that keeps its 6 posts, and 9 posts."></p>

# pito-sync

[![CI](https://github.com/gmrdad82/pito-sync/actions/workflows/ci.yml/badge.svg)](https://github.com/gmrdad82/pito-sync/actions/workflows/ci.yml) [![Version](https://img.shields.io/github/v/tag/gmrdad82/pito-sync)](https://github.com/gmrdad82/pito-sync/tags)

pito-sync is a small Rust crate for apps whose UI reads a local SQLite file
while background sources keep it up to date, part of [PITO](https://pitomd.com).
The UI never waits on the network: it reads its own database, and each source
syncs data into it on its own thread and says when something changed. Each app
keeps its own database file and its own tables; the crate never shares a file
or a row between apps.

Works on Linux and macOS (Windows through WSL).

## What it is

- **One database per app,** opened private (the file 0600, its folder 0700)
  with WAL, and treated as a cache: when the app's schema version changes, the
  file is dropped and rebuilt, never migrated.
- **Sources on their own threads.** A source is a trait with one `pass`. The
  runner gives it its own connection, runs the first pass at once, then timed
  passes or a pass whenever the app asks with a `Resync`.
- **One transaction per pass.** A pass writes its rows and its bookkeeping row
  in one `BEGIN IMMEDIATE`, so the UI sees both or neither.
- **The UI only reads,** through a read-only connection, and re-reads when a
  `Changed` arrives. A pass sends one when it starts and one when it ends, so
  the UI can show "syncing…" with no extra wiring.
- **Failures the UI can read back** by kind (`no_key`, `refused`, `forbidden`,
  `unreachable`, `bad_answer`, `rate_limited`), with a `retry_after` a timed
  pass respects.
- **Secrets that stay secret:** a `Secret` hides in `Debug`, has no `Clone` or
  `Display`, and zeroes its buffer when it drops; the crate never writes one
  anywhere.

The crate re-exports `rusqlite` (0.40, with SQLite bundled), so an app uses
`pito_sync::rusqlite` and stays on the same version.

## Install

Add it to your `Cargo.toml` as a git dependency:

```toml
pito-sync = { git = "https://github.com/gmrdad82/pito-sync", tag = "v0.1.5" }
```

It needs Rust 1.97 or newer and a C compiler, since SQLite is compiled in:

- **Linux:** `build-essential` on Debian and Ubuntu, `base-devel` on Arch, or
  `gcc` from your distribution.
- **macOS:** the Xcode command line tools, `xcode-select --install`.
- **Windows:** inside WSL, as on Linux. Native Windows isn't supported: the
  crate sets Unix file permissions on the database.

## Use

[`examples/feed.rs`](examples/feed.rs) is the clip at the top: a source that
needs a token, gets it after start, syncs three posts per pass, fails once
while keeping its rows, and a UI loop that only reads.

```
cargo run --example feed
```

The pieces, in short:

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

### Where the database goes

The app picks the path. A per-user state folder is the usual place:
`$XDG_STATE_HOME/<app>/` on Linux (`~/.local/state/<app>/` when it is unset)
and `~/Library/Application Support/<app>/` on macOS; the example's
`state_dir()` picks between them. Two apps never point at the same path.

### The contract

- **One database per app.** An app describes its file with a `Spec`: its path,
  its `version` and its `schema`, the `CREATE` statements of its own tables.
  The crate adds one table of its own, `sync`.
- **The UI never writes.** It reads through `reader(&spec)`, a read-only
  connection that is `None` until a writer has built the file and while the
  stored version differs, and re-reads when a `Changed` arrives. Writes come
  only from sources, through `commit`, one `BEGIN IMMEDIATE` per pass.
- **Opening.** `open(&spec)` sets WAL, `synchronous=NORMAL`,
  `busy_timeout=5000` and `foreign_keys=ON`. It creates the file 0600 and its
  folder 0700, and corrects both on every open.
- **Drop and rebuild.** The file is a cache, never migrated. The stored
  `user_version` is `spec.version * 1000 + SYNC_VERSION`
  (`spec.user_version()`), so a change on either side, the app's tables or the
  crate's `sync` table, rebuilds: when it differs, or the file isn't a
  database, `open` deletes the file with its `-wal` and `-shm` and builds it
  fresh. A version runs from 0 to `MAX_VERSION` (2147482), so the stamp fits
  SQLite's 32-bit `user_version`; anything else is refused.
- **Tests and fixtures** build the same database in memory with
  `memory(&spec)`.

### Sources and the runner

```rust
pub trait Source: Send {
    fn name(&self) -> &str;
    fn every(&self) -> Option<Duration>;
    fn pass(&mut self, conn: &mut Connection, id: Option<&str>) -> Result<usize, Failure>;
    fn secret(&mut self, name: &str, value: Secret) {}
}

pub fn spawn(spec: &Spec, source: Box<dyn Source>, changed: Sender<Changed>) -> Sender<Message>;
pub fn spawn_checked(spec: &Spec, source: Box<dyn Source>, changed: Sender<Changed>) -> Result<Sender<Message>, String>;
```

`spawn` runs one source on its own thread with its own connection and returns
the source's inbox. A name is any string the app builds; it keys the source's
`sync` row.

- **The first pass runs at once.** As soon as the thread has opened the file
  it runs a pass, so an app needs no start-up `Resync`. Only a `retry_after`
  an earlier run left that still holds keeps it back; the timed passes then
  pick it up.
- **Timed passes.** With `every` set, the thread then wakes every `every`, or
  every minute when that's longer, and runs a pass when it's `due`: no
  `retry_after` holds it, and the last pass failed or finished at least
  `every` ago. A failed pass is tried again at the next wake; a source that
  wants to wait longer says so with `retry_after`. With `every` at `None` it
  runs at start and then only when asked.
- **`Message::Resync(Resync(name, id))`** runs a pass at once, `retry_after`
  or not, and hands `id` to `pass` (a source might take `"full"` for a full
  resync). `Resync(..).into()` makes the message. An explicit resync always
  runs, even right after a pass: a user's refresh key, or a resync after a
  write, is never dropped.
- **`Message::Secret(name, Secret)`** hands a running source a secret it
  didn't have when it started, such as a key unlocked later or a token after
  sign-in. The thread calls `source.secret(name, value)`, whose default drops
  it, then runs a pass at once, never skipped.
- **`Changed(name)`** goes out twice per pass, ok or failed: when it starts,
  right after the thread marks it `started`, and when it ends. A UI re-reads
  on each and shows "syncing…" while `started_at > finished_at`, with no
  source cloning the sender. A pass ends by committing its rows, which stamps
  `finished_at`, or by returning `Err(failure)`, which the thread records with
  `failed`. If the file can't be opened the thread sends one `Changed` and
  stops. Dropping the inbox's `Sender` or the `Changed` receiver ends the
  thread.
- **`pass_once(conn, source, id) -> Result<usize, Failure>`** is that one pass
  on its own, on the calling thread: mark `started`, run `pass`, and record a
  failure with `failed`. The runner uses exactly this, adding only its two
  `Changed` sends. An app that needs a pass without a thread (a headless dump,
  a test) calls it instead of copying the sequence.

### A failed open

`spawn` opens the file on its thread, so a file that can't be opened shows
only as one `Changed` and a source that never passes. `spawn_checked` opens it
on the calling thread instead: it returns `Err` with `open`'s reason (`cannot
write <folder>: …`, `cannot open <file>: …`) and starts nothing, or moves the
open connection into the thread, which then runs exactly as `spawn`'s does.

### Secrets

`Secret::new(value)` takes the value, `expose()` is the only way back to it,
its `Debug` prints `Secret(hidden)`, and its buffer, spare capacity included,
is zeroed when it drops. It has no `Clone` and no `Display`. The crate writes
no secret to the database, a log, an error or `Debug` output, and a source
keeps it that way, never putting one in a `Failure`, a cursor or a row. Hand it
an owned `String` so no other copy is left behind.

### Bookkeeping

The `sync` table holds one row per source: `source`, `started_at`,
`finished_at`, `ok`, `error`, `cursor`, `rows` and `retry_after`, times in
milliseconds since the epoch.

- `started(conn, source, at)` marks a pass begun; the runner calls it before
  every pass.
- `commit(conn, Pass { source, started_at, finished_at, failure, cursor }, |tx| ...)`
  writes the pass's rows and its `sync` row in one `BEGIN IMMEDIATE`. A
  failure sets `ok = 0`, its `error` and its `retry_after`; a success clears
  them.
- `failed(conn, source, &failure)` records a failed pass: `ok = 0`, its
  `error` and its `retry_after`, and `finished_at` at the time of the failure
  (never before `started_at`), so the pass no longer reads as running. It
  leaves the data alone, and `rows` and `cursor` stay as the last successful
  pass left them.
- `sync(conn, source)` reads the row back, `retry_after(conn, source)` its
  hold, and `due(conn, source, every, now)` says whether a timed pass should
  run.

### Errors

`Fault` is the kind: `no_key`, `refused`, `forbidden`, `unreachable`,
`bad_answer` or `rate_limited`. `Failure { fault, message, retry_after }` is
stored as `kind: message` (`text()`), and `Fault::parse(&error)` reads it back
so the UI can say each kind its own way. `Failure::until(Some(at))` sets
`retry_after`: a timed pass waits until `at`, a resync doesn't.

## Contributing

Issues are welcome at
[github.com/gmrdad82/pito-sync/issues](https://github.com/gmrdad82/pito-sync/issues):
a bug with a small program or test that shows it, or an idea the crate should
cover. Pull requests aren't taken. Please read the
[code of conduct](CODE_OF_CONDUCT.md) first, and report a security issue
privately as [SECURITY.md](SECURITY.md) says.

## Build and test

```
bin/gate
```

`bin/gate` runs `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, the tests with [cargo-nextest](https://nexte.st) and the doctests;
the tests write only under `tmp/`. CI runs it on Linux and macOS on every push
and nightly, and runs the example. The clip at the top is a terminal recording
of the example ([NOTICE](NOTICE)).

## Licence

The code is MIT licensed ([LICENSE](LICENSE)), © Catalin Ilinca. The pito-sync
and PITO names and their logos are © Catalin Ilinca, all rights reserved, and
are not covered by the MIT licence ([TRADEMARKS.md](TRADEMARKS.md)).
