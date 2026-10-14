---
title: Place the database
description: Choose where an app's database file lives, and how pito-sync keeps it private.
sidebar:
  order: 7
---

Each app keeps its own database file. The app picks the path in its `Spec`,
and two apps never point at the same one.

## A per-user state folder

The usual place is the user's state folder:

| Platform | Folder |
|---|---|
| Linux | `$XDG_STATE_HOME/<app>/`, or `~/.local/state/<app>/` when it is unset |
| macOS | `~/Library/Application Support/<app>/` |

The `feed` example picks between them like this:

```rust
fn state_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        return home.join("Library/Application Support");
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"))
}

let spec = Spec::new(state_dir().join("pito-sync-feed/feed.db"), 1, SCHEMA);
```

## Kept private

`open` creates the folder 0700 and the file 0600, so only the user can read
them. It checks both on every open and puts them back if something loosened
them:

![A terminal: after the feed example runs, stat shows its folder drwx------ and feed.db -rw-------; chmod opens them to drwxr-xr-x and -rw-r--r--; after the example runs again, stat shows drwx------ and -rw------- once more.](../images/private.gif)

```
$ feed > /dev/null
$ stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}
drwx------  ~/.local/state/pito-sync-feed
-rw-------  ~/.local/state/pito-sync-feed/feed.db
$ chmod 755 ~/.local/state/pito-sync-feed && chmod 644 ~/.local/state/pito-sync-feed/feed.db
$ stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}
drwxr-xr-x  ~/.local/state/pito-sync-feed
-rw-r--r--  ~/.local/state/pito-sync-feed/feed.db
$ feed > /dev/null
$ stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}
drwx------  ~/.local/state/pito-sync-feed
-rw-------  ~/.local/state/pito-sync-feed/feed.db
```

`stat` prints your home folder in full where this page writes `~`. Unix
permissions are why native Windows isn't supported; on Windows, use
WSL.

## How it is opened

`open(&spec)` sets these on the writing connection:

| Setting | Value | Why |
|---|---|---|
| `journal_mode` | `WAL` | readers never wait for a writer's commit |
| `synchronous` | `NORMAL` | a commit in WAL needs no sync of its own |
| `busy_timeout` | 5000 ms | a second writer waits instead of failing at once |
| `foreign_keys` | `ON` | the app's `REFERENCES` are enforced |

Next to the file, SQLite keeps `<file>-wal` and `<file>-shm` while it is in
use. `reader(&spec)` opens the same file read-only.

## In memory, for tests

`memory(&spec)` builds the same schema in an in-memory database, with no file
at all; see [test a source](test-a-source.md).
