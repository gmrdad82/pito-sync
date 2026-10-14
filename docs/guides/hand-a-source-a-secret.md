---
title: Hand a source a secret
description: Give a running source a token or key it didn't have at start, wrapped in a Secret that never shows and is wiped on drop.
sidebar:
  order: 5
---

Some sources need a secret the app only gets later: a token after the user
signs in, or a key unlocked from the keyring. Send it to the running source
with `Message::Secret`, wrapped in a `Secret`.

## Send it

```rust
use pito_sync::{Message, Secret};

let token = String::from("from the keyring");
asks.send(Message::Secret("feed-token".into(), Secret::new(token)))?;
```

The thread calls the source's `secret(name, value)` and then runs a pass at
once, never skipped, so the data shows up as soon as the secret arrives.

## Keep it in the source

`Source::secret` drops the value by default. A source that wants it
overrides the method and keeps it:

```rust
struct Feed {
    token: Option<Secret>,
}

impl Source for Feed {
    fn secret(&mut self, name: &str, value: Secret) {
        if name == "feed-token" {
            self.token = Some(value);
        }
    }

    fn pass(&mut self, conn: &mut Connection, _id: Option<&str>) -> Result<usize, Failure> {
        let Some(token) = self.token.as_ref().filter(|t| !t.expose().is_empty()) else {
            return Err(Failure::new(Fault::NoKey, "sign in first"));
        };
        let posts = fetch(token.expose())?;
        write(conn, posts)
    }
}
```

`name` and `every` are left out here; `fetch` and `write` stand for the
source's own code. Until the token arrives, each pass fails with `no_key` and the UI can ask the
user to sign in. In the `feed` example the first pass runs without a token,
the app sends one, and the next pass writes:

```
feed      syncing…
feed      no key: sign in first (0 posts kept)
feed      syncing…
feed      3 posts, newest "Threads that never block the UI"
```

## What `Secret` guarantees

```rust
let token = pito_sync::Secret::new(String::from("from the keyring"));
println!("{token:?}");
```

```
Secret(hidden)
```

- `Debug` prints `Secret(hidden)`, so a secret in a struct that derives
  `Debug` never reaches a log.
- It has no `Clone` and no `Display`.
- `expose()` is the only way back to the value, as a `&str`.
- When it drops, its buffer is zeroed, spare capacity included.

Hand `Secret::new` an owned `String`, so no other copy of the value is left
behind. The crate writes no secret to the database, a log, an error or
`Debug` output. Keep it that way in your source: never put a secret in a
`Failure`, a cursor or a row.
