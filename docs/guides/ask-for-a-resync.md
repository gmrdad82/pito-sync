---
title: Ask for a resync
description: Send a source a Resync to run a pass at once, for a refresh key or after the app writes elsewhere.
sidebar:
  order: 4
---

The app asks a source for a pass at once by sending a `Resync` to the inbox
that `spawn` or `spawn_checked` returned:

```rust
use pito_sync::Resync;

asks.send(Resync("feed".into(), None).into())?;
```

`Resync(name, id)` becomes a `Message::Resync` with `.into()`. The thread
runs a pass and hands `id` to `pass` as its second argument. A source decides
what an id means: `"full"` for a full resync instead of an incremental one,
for example, or the id of the one item to fetch again.

```rust
fn pass(&mut self, conn: &mut Connection, id: Option<&str>) -> Result<usize, Failure> {
    let since = match id {
        Some("full") => None,
        _ => pito_sync::sync(conn, "feed").ok().flatten().and_then(|s| s.cursor),
    };
    let items = self.fetch(since.as_deref())?;
    self.write(conn, items)
}
```

`fetch` and `write` stand for the source's own code.

## It always runs

An explicit resync is never dropped. It runs right after another pass, and
it runs while a `retry_after` holds back the timed passes, so a user's
refresh key always does something. A resync took 33 µs from the send to its
closing `Changed` in the measured run.

## What the UI sees

Like every pass, a resync sends `Changed(name)` when it starts and when it
ends. The `feed` example sends three resyncs, the second one with the id
`"offline"`, which its source treats as a server that doesn't answer:

```rust
for id in [None, Some("offline"), None] {
    pause();
    let _ = asks.send(Resync("feed".into(), id.map(String::from)).into());
}
```

```
feed      syncing…
feed      6 posts, newest "One file per app"
feed      syncing…
feed      unreachable: the server did not answer (6 posts kept)
feed      syncing…
feed      9 posts, newest "The sync table"
```

When the source's thread has ended, because the file couldn't be opened or
the `Changed` receiver was dropped, `send` returns `Err`.
