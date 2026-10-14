---
title: Report a failure
description: Return a Failure with its kind, keep the last good rows, hold timed passes back, and let the UI say each kind its own way.
sidebar:
  order: 6
---

A pass that can't sync returns `Err(Failure)`. The thread records it in the
source's `sync` row, and the rows the last good pass wrote stay where they
are, so the UI still shows what it has.

## Return a `Failure`

```rust
return Err(Failure::new(Fault::Unreachable, "the server did not answer"));
```

`Fault` is the kind of failure:

| `Fault` | Stored as | Typical cause |
|---|---|---|
| `NoKey` | `no_key` | no token or key yet: ask the user to sign in |
| `Refused` | `refused` | the server turned the credentials down |
| `Forbidden` | `forbidden` | signed in, but not allowed to read this |
| `Unreachable` | `unreachable` | no network, or the server didn't answer |
| `BadAnswer` | `bad_answer` | the answer couldn't be read, or writing it failed |
| `RateLimited` | `rate_limited` | the server asked to slow down |

The message is free text for the user. Never put a secret in it.

## Hold the timed passes back

`until(Some(at))` sets `retry_after`, a time in milliseconds since the epoch:

```rust
return Err(Failure::new(Fault::RateLimited, "try again in a minute").until(Some(retry_at)));
```

Timed passes wait until then. A `Resync` the user asks for still runs, and
so does the pass after a `Secret`. A failure without `until` is tried again
at the thread's next wake.

## What the thread records

`failed(conn, source, &failure)` is what the thread calls, and what
`pass_once` calls for you:

- `ok = 0`, `error` set to the failure's text, `kind: message`, and
  `retry_after` set or cleared;
- `finished_at` set to the time of the failure, never before `started_at`, so
  the pass no longer reads as running;
- the data, `rows` and `cursor` left as the last successful pass left them.

## Say it in the UI

`Fault::parse` reads the stored text back into its kind and message:

```rust
match sync.error.as_deref().and_then(Fault::parse) {
    Some((Fault::NoKey, _)) => "sign in to see your feed".into(),
    Some((fault, message)) => format!("{}: {message} ({posts} posts kept)", fault.word()),
    None => format!("{posts} posts"),
}
```

The `feed` example fails one pass on purpose. Its six posts stay, and the
next pass brings nine:

```
feed      syncing…
feed      unreachable: the server did not answer (6 posts kept)
feed      syncing…
feed      9 posts, newest "The sync table"
```
