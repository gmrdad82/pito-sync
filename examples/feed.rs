use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pito_sync::rusqlite::Connection;
use pito_sync::{Changed, Failure, Fault, Message, Pass, Resync, Secret, Source, Spec};

const SCHEMA: &[&str] = &["CREATE TABLE posts (id INTEGER PRIMARY KEY, title TEXT NOT NULL)"];

const TITLES: [&str; 9] = [
    "Hello, world",
    "WAL in one page",
    "Threads that never block the UI",
    "Drop and rebuild",
    "Busy timeouts",
    "One file per app",
    "Secrets that wipe themselves",
    "Retry after",
    "The sync table",
];

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

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

struct Feed {
    token: Option<Secret>,
    seen: usize,
}

impl Source for Feed {
    fn name(&self) -> &str {
        "feed"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn secret(&mut self, name: &str, value: Secret) {
        if name == "feed-token" {
            self.token = Some(value);
        }
    }

    fn pass(&mut self, conn: &mut Connection, id: Option<&str>) -> Result<usize, Failure> {
        let started_at = now();
        thread::sleep(Duration::from_millis(500));
        if self.token.as_ref().is_none_or(|t| t.expose().is_empty()) {
            return Err(Failure::new(Fault::NoKey, "sign in first"));
        }
        if id == Some("offline") {
            return Err(Failure::new(
                Fault::Unreachable,
                "the server did not answer",
            ));
        }
        self.seen = (self.seen + 3).min(TITLES.len());
        let titles = &TITLES[..self.seen];
        let pass = Pass {
            source: "feed",
            started_at,
            finished_at: now(),
            failure: None,
            cursor: None,
        };
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

fn show(spec: &Spec, name: &str) -> Result<String, String> {
    let Some(ui) = pito_sync::reader(spec)? else {
        return Ok("not built yet".into());
    };
    let sync = pito_sync::sync(&ui, name)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if sync.started_at > sync.finished_at {
        return Ok("syncing…".into());
    }
    let (posts, newest): (i64, Option<String>) = ui
        .query_row(
            "SELECT count(*), (SELECT title FROM posts ORDER BY id DESC LIMIT 1) FROM posts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    match sync.error.as_deref().and_then(Fault::parse) {
        Some((fault, message)) => Ok(format!(
            "{}: {message} ({posts} posts kept)",
            fault.word().replace('_', " ")
        )),
        None => Ok(format!(
            "{posts} posts, newest \"{}\"",
            newest.unwrap_or_default()
        )),
    }
}

fn main() -> Result<(), String> {
    let spec = Spec::new(state_dir().join("pito-sync-feed/feed.db"), 1, SCHEMA);
    println!("{:<9} {}", "database", spec.path.display());
    let (changed, changes) = mpsc::channel::<Changed>();
    let feed = Feed {
        token: None,
        seen: 0,
    };
    let asks = pito_sync::spawn_checked(&spec, Box::new(feed), changed)?;
    thread::spawn(move || {
        let pause = || thread::sleep(Duration::from_millis(1200));
        pause();
        let token = String::from("from the keyring");
        let _ = asks.send(Message::Secret("feed-token".into(), Secret::new(token)));
        for id in [None, Some("offline"), None] {
            pause();
            let _ = asks.send(Resync("feed".into(), id.map(String::from)).into());
        }
        pause();
    });
    for Changed(name) in changes {
        println!("{name:<9} {}", show(&spec, &name)?);
    }
    Ok(())
}
