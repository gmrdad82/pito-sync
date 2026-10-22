use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::book::now_ms;
use crate::store::STRIDE;
use crate::*;

mod edges;

const NOTES: &[&str] = &["CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)"];

const NOTES_PINNED: &[&str] = &["CREATE TABLE notes (
  id INTEGER PRIMARY KEY,
  body TEXT NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0
)"];

const LEDGER: &[&str] = &[
    "CREATE TABLE ledger (id TEXT PRIMARY KEY, seats INTEGER NOT NULL)",
    "CREATE INDEX ledger_seats ON ledger (seats)",
];

const SYNC_COLUMNS: [&str; 8] = [
    "source",
    "started_at",
    "finished_at",
    "ok",
    "error",
    "cursor",
    "rows",
    "retry_after",
];

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tmp/tests")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut statement = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn pragma(conn: &Connection, name: &str) -> i64 {
    conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .unwrap()
}

fn mark(conn: &mut Connection, source: &str, finished_at: i64, failure: Option<Failure>) {
    commit(
        conn,
        Pass {
            source,
            started_at: finished_at - 1,
            finished_at,
            failure,
            cursor: None,
        },
        |_| Ok(0),
    )
    .unwrap();
}

#[test]
fn a_new_database_is_private_versioned_and_in_wal_mode() {
    let dir = scratch("new").join("state");
    let spec = Spec::new(dir.join("app.db"), 1, NOTES);
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
    assert_eq!(spec.user_version(), STRIDE + SYNC_VERSION);
    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    assert_eq!(pragma(&conn, "foreign_keys"), 1);
    assert_eq!(pragma(&conn, "synchronous"), 1);
    assert_eq!(pragma(&conn, "busy_timeout"), 5000);
    assert_eq!(tables(&conn), ["notes", "sync"]);
    assert_eq!(columns(&conn, "sync"), SYNC_COLUMNS);
    assert_eq!(mode(&spec.path), 0o600);
    assert_eq!(mode(&dir), 0o700);
    assert!(reader(&spec).unwrap().is_some());
}

#[test]
fn a_loose_file_and_folder_are_tightened_on_open() {
    let dir = scratch("loose").join("state");
    let spec = Spec::new(dir.join("app.db"), 1, NOTES);
    let mut conn = open(&spec).unwrap();
    mark(&mut conn, "notes", 10, None);
    for suffix in ["-wal", "-shm"] {
        let file = sibling(&spec.path, suffix);
        if file.exists() {
            assert_eq!(mode(&file), 0o600, "{suffix}");
        }
    }
    drop(conn);
    fs::set_permissions(&spec.path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let conn = open(&spec).unwrap();
    assert_eq!(mode(&spec.path), 0o600);
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(sync(&conn, "notes").unwrap().unwrap().finished_at, Some(10));
}

#[test]
fn two_apps_keep_two_separate_files() {
    let base = scratch("apps");
    let journal = Spec::new(base.join("journal/journal.db"), 2, NOTES);
    let ledger = Spec::new(base.join("ledger/ledger.db"), 1, LEDGER);
    let mut first = open(&journal).unwrap();
    let mut second = open(&ledger).unwrap();
    assert_eq!(tables(&first), ["notes", "sync"]);
    assert_eq!(tables(&second), ["ledger", "sync"]);
    first
        .execute("INSERT INTO notes (body) VALUES ('only the journal')", [])
        .unwrap();
    mark(&mut first, "inbox", 20, None);
    mark(&mut second, "licences", 30, None);
    assert_eq!(count(&first, "notes"), 1);
    assert_eq!(count(&second, "ledger"), 0);
    assert!(sync(&second, "inbox").unwrap().is_none());
    assert!(sync(&first, "licences").unwrap().is_none());
    assert_eq!(version(&first).unwrap(), journal.user_version());
    assert_eq!(version(&second).unwrap(), ledger.user_version());
    assert_ne!(journal.user_version(), ledger.user_version());
    for spec in [&journal, &ledger] {
        assert_eq!(mode(&spec.path), 0o600);
        assert_eq!(mode(spec.path.parent().unwrap()), 0o700);
    }
    drop((first, second));
    let first = open(&journal).unwrap();
    assert_eq!(count(&first, "notes"), 1);
    assert_eq!(
        sync(&first, "inbox").unwrap().unwrap().finished_at,
        Some(20)
    );
}

#[test]
fn an_app_version_bump_rebuilds() {
    let file = scratch("app-bump").join("app.db");
    let old = Spec::new(&file, 1, NOTES);
    let mut conn = open(&old).unwrap();
    conn.execute("INSERT INTO notes (body) VALUES ('old')", [])
        .unwrap();
    mark(&mut conn, "notes", 10, None);
    drop(conn);
    assert!(reader(&old).unwrap().is_some());
    let new = Spec::new(&file, 2, NOTES_PINNED);
    assert!(reader(&new).unwrap().is_none());
    let conn = open(&new).unwrap();
    assert_eq!(version(&conn).unwrap(), new.user_version());
    assert_eq!(columns(&conn, "notes"), ["id", "body", "pinned"]);
    assert_eq!(count(&conn, "notes"), 0);
    assert!(sync(&conn, "notes").unwrap().is_none());
    assert!(reader(&old).unwrap().is_none());
    assert!(reader(&new).unwrap().is_some());
}

#[test]
fn a_crate_version_bump_rebuilds() {
    for other in [SYNC_VERSION - 1, SYNC_VERSION + 1] {
        let file = scratch(&format!("crate-bump-{other}")).join("app.db");
        let spec = Spec::new(&file, 3, NOTES);
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE sync (source TEXT PRIMARY KEY, started_at INTEGER);
             CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
             INSERT INTO notes (body) VALUES ('from another crate');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 3 * STRIDE + other)
            .unwrap();
        drop(conn);
        assert!(reader(&spec).unwrap().is_none());
        let conn = open(&spec).unwrap();
        assert_eq!(version(&conn).unwrap(), spec.user_version());
        assert_eq!(columns(&conn, "sync"), SYNC_COLUMNS);
        assert_eq!(count(&conn, "notes"), 0);
    }
}

#[test]
fn another_version_is_thrown_away_and_rebuilt() {
    let file = scratch("old").join("app.db");
    let spec = Spec::new(&file, 1, NOTES);
    let conn = Connection::open(&file).unwrap();
    conn.execute_batch("CREATE TABLE stale (x); PRAGMA user_version = 99;")
        .unwrap();
    drop(conn);
    assert!(reader(&spec).unwrap().is_none());
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
    assert_eq!(tables(&conn), ["notes", "sync"]);
}

#[test]
fn a_file_that_isnt_a_database_is_rebuilt() {
    let file = scratch("junk").join("app.db");
    let junk = "not a database at all, just some text that is long enough";
    for suffix in ["", "-wal", "-shm"] {
        fs::write(sibling(&file, suffix), junk).unwrap();
    }
    let spec = Spec::new(&file, 1, NOTES);
    assert!(reader(&spec).is_err());
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
    assert_eq!(tables(&conn), ["notes", "sync"]);
    assert_eq!(mode(&file), 0o600);
    for suffix in ["-wal", "-shm"] {
        if let Ok(bytes) = fs::read(sibling(&file, suffix)) {
            assert!(!bytes.starts_with(junk.as_bytes()), "{suffix}");
        }
    }
}

#[test]
fn a_missing_database_reads_as_nothing() {
    let dir = scratch("missing");
    assert!(
        reader(&Spec::new(dir.join("app.db"), 1, NOTES))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_version_out_of_range_is_refused() {
    let file = scratch("range").join("app.db");
    for version in [-1, MAX_VERSION + 1, i64::MAX] {
        let spec = Spec::new(&file, version, NOTES);
        assert!(open(&spec).is_err(), "{version}");
        assert!(reader(&spec).is_err(), "{version}");
        assert!(memory(&spec).is_err(), "{version}");
    }
    assert!(!file.exists());
    let spec = Spec::new(&file, MAX_VERSION, NOTES);
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
    assert!(reader(&spec).unwrap().is_some());
}

#[test]
fn an_error_reads_back_as_its_kind_and_message() {
    for fault in Fault::ALL {
        let text = Failure::new(fault, "why").text();
        assert_eq!(Fault::parse(&text), Some((fault, "why")));
    }
    assert_eq!(Fault::parse("odd: why"), None);
    assert_eq!(Fault::parse("no kind"), None);
}

#[test]
fn a_pass_records_its_progress_and_its_failure() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    assert_eq!(tables(&conn), ["notes", "sync"]);
    started(&conn, "inbox", 10).unwrap();
    assert_eq!(sync(&conn, "inbox").unwrap().unwrap().started_at, Some(10));
    let rows = commit(
        &mut conn,
        Pass {
            source: "inbox",
            started_at: 10,
            finished_at: 30,
            failure: Some(Failure::new(Fault::Unreachable, "GitHub didn't answer")),
            cursor: None,
        },
        |_| Ok(4),
    )
    .unwrap();
    assert_eq!(rows, 4);
    let seen = sync(&conn, "inbox").unwrap().unwrap();
    assert_eq!(seen.finished_at, Some(30));
    assert_eq!(seen.ok, Some(false));
    assert_eq!(
        seen.error.as_deref(),
        Some("unreachable: GitHub didn't answer")
    );
    assert_eq!(seen.rows, Some(4));
    assert!(sync(&conn, "feed").unwrap().is_none());
    assert!(!Path::new("unused.db").exists());
}

#[test]
fn a_retry_after_holds_a_timed_pass_until_its_time() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    let every = Duration::from_secs(60);
    assert!(due(&conn, "feed", every, 0));
    assert_eq!(retry_after(&conn, "feed").unwrap(), None);
    let limited = Failure::new(Fault::RateLimited, "slow down").until(Some(95_000));
    failed(&conn, "feed", &limited).unwrap();
    assert_eq!(retry_after(&conn, "feed").unwrap(), Some(95_000));
    assert_eq!(
        sync(&conn, "feed").unwrap().unwrap().error.as_deref(),
        Some("rate_limited: slow down")
    );
    assert!(!due(&conn, "feed", every, 50_000));
    assert!(due(&conn, "feed", every, 95_000));
    mark(&mut conn, "feed", 96_000, None);
    assert_eq!(retry_after(&conn, "feed").unwrap(), None);
    assert!(!due(&conn, "feed", every, 100_000));
    assert!(due(&conn, "feed", every, 156_000));
    mark(
        &mut conn,
        "feed",
        200_000,
        Some(limited.clone().until(Some(500_000))),
    );
    assert_eq!(retry_after(&conn, "feed").unwrap(), Some(500_000));
    assert!(!due(&conn, "feed", every, 400_000));
    assert!(due(&conn, "feed", every, 500_000));
    failed(&conn, "feed", &Failure::new(Fault::Unreachable, "gone")).unwrap();
    assert_eq!(retry_after(&conn, "feed").unwrap(), None);
    assert!(due(&conn, "feed", every, 400_000));
    assert!(due(&conn, "other", every, 0));
}

struct Counter {
    name: String,
    every: Option<Duration>,
    passes: usize,
}

impl Counter {
    fn new(name: &str, every: Option<Duration>) -> Box<Self> {
        Box::new(Counter {
            name: name.to_owned(),
            every,
            passes: 0,
        })
    }
}

impl Source for Counter {
    fn name(&self) -> &str {
        &self.name
    }

    fn every(&self) -> Option<Duration> {
        self.every
    }

    fn pass(&mut self, conn: &mut Connection, _: Option<&str>) -> Result<usize, Failure> {
        self.passes += 1;
        let n = self.passes;
        commit(
            conn,
            Pass {
                source: &self.name,
                started_at: 1,
                finished_at: 2,
                failure: None,
                cursor: Some(n.to_string()),
            },
            |_| Ok(n),
        )
        .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}

const WAIT: Duration = Duration::from_secs(30);

const SECOND: Duration = Duration::from_secs(1);

fn passed(seen: &Receiver<Changed>, name: &str) {
    for _ in 0..2 {
        assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed(name.into()));
    }
}

fn syncing(sync: &Sync) -> bool {
    sync.started_at > sync.finished_at
}

#[test]
fn a_resync_runs_a_pass_and_says_changed() {
    let spec = Spec::new(scratch("thread").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, Counter::new("count:app", None), changed);
    passed(&seen, "count:app");
    asks.send(Resync("count:app".into(), None).into()).unwrap();
    passed(&seen, "count:app");
    let conn = reader(&spec).unwrap().unwrap();
    assert_eq!(
        sync(&conn, "count:app").unwrap().unwrap().cursor.as_deref(),
        Some("2")
    );
}

#[test]
fn a_due_source_runs_on_its_own() {
    let spec = Spec::new(scratch("timed").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let _asks = spawn(
        &spec,
        Counter::new("timed", Some(Duration::from_millis(20))),
        changed,
    );
    for _ in 0..4 {
        assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("timed".into()));
    }
}

#[test]
fn a_database_that_cannot_open_still_says_changed() {
    let base = scratch("blocked");
    fs::write(base.join("file"), "a file, not a folder").unwrap();
    let spec = Spec::new(base.join("file/app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let _asks = spawn(&spec, Counter::new("blocked", None), changed);
    assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("blocked".into()));
    assert!(seen.recv_timeout(WAIT).is_err());
}

#[test]
fn a_checked_spawn_returns_the_open_error() {
    let base = scratch("checked-blocked");
    fs::write(base.join("file"), "a file, not a folder").unwrap();
    let spec = Spec::new(base.join("file/app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let error = spawn_checked(&spec, Counter::new("blocked", None), changed).unwrap_err();
    assert!(error.starts_with("cannot write "), "{error}");
    assert!(
        error.contains(&base.join("file").display().to_string()),
        "{error}"
    );
    assert!(seen.recv_timeout(SECOND).is_err());
}

#[test]
fn a_checked_spawn_that_opens_runs_like_spawn() {
    let spec = Spec::new(scratch("checked").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let asks = spawn_checked(&spec, Counter::new("count:app", None), changed).unwrap();
    passed(&seen, "count:app");
    asks.send(Resync("count:app".into(), None).into()).unwrap();
    passed(&seen, "count:app");
    let conn = reader(&spec).unwrap().unwrap();
    assert_eq!(
        sync(&conn, "count:app").unwrap().unwrap().cursor.as_deref(),
        Some("2")
    );
}

const SECRET: &str = "s3cr3t-value-0042";

struct Keyed {
    held: Option<(String, Secret)>,
}

impl Source for Keyed {
    fn name(&self) -> &str {
        "keyed"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn secret(&mut self, name: &str, value: Secret) {
        self.held = Some((name.to_owned(), value));
    }

    fn pass(&mut self, conn: &mut Connection, _: Option<&str>) -> Result<usize, Failure> {
        let cursor = match &self.held {
            Some((name, value)) => format!("{name}:{}", value.expose() == SECRET),
            None => "none".to_owned(),
        };
        let failure = self
            .held
            .is_none()
            .then(|| Failure::new(Fault::NoKey, "no key yet"));
        commit(
            conn,
            Pass {
                source: "keyed",
                started_at: 1,
                finished_at: 2,
                failure,
                cursor: Some(cursor),
            },
            |_| Ok(1),
        )
        .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}

#[test]
fn a_secret_hides_in_debug() {
    let secret = Secret::new(SECRET);
    assert_eq!(format!("{secret:?}"), "Secret(hidden)");
    assert_eq!(format!("{secret:#?}"), "Secret(hidden)");
    let message = Message::Secret("feed".into(), secret);
    let shown = format!("{message:?}");
    assert_eq!(shown, "Secret(\"feed\", Secret(hidden))");
    assert!(!format!("{message:#?}").contains(SECRET));
}

#[test]
fn a_secret_reaches_a_source_through_spawn_then_a_pass_runs() {
    let spec = Spec::new(scratch("secret").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, Box::new(Keyed { held: None }), changed);
    passed(&seen, "keyed");
    let conn = reader(&spec).unwrap().unwrap();
    let before = sync(&conn, "keyed").unwrap().unwrap();
    assert_eq!(before.cursor.as_deref(), Some("none"));
    assert_eq!(before.error.as_deref(), Some("no_key: no key yet"));
    asks.send(Message::Secret("feed".into(), Secret::new(SECRET)))
        .unwrap();
    passed(&seen, "keyed");
    let after = sync(&conn, "keyed").unwrap().unwrap();
    assert_eq!(after.cursor.as_deref(), Some("feed:true"));
    assert_eq!(after.ok, Some(true));
    assert_eq!(after.error, None);
    drop(asks);
    for suffix in ["", "-wal"] {
        if let Ok(bytes) = fs::read(sibling(&spec.path, suffix)) {
            assert!(
                !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
                "{suffix}"
            );
        }
    }
}

#[test]
fn a_source_without_a_secret_hook_drops_it_and_still_passes() {
    let spec = Spec::new(scratch("unkeyed").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, Counter::new("plain", None), changed);
    asks.send(Message::Secret("feed".into(), Secret::new(SECRET)))
        .unwrap();
    passed(&seen, "plain");
    passed(&seen, "plain");
    let conn = reader(&spec).unwrap().unwrap();
    assert_eq!(
        sync(&conn, "plain").unwrap().unwrap().cursor.as_deref(),
        Some("2")
    );
}

#[test]
fn a_failure_records_its_finish_and_keeps_the_last_good_rows_and_cursor() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    commit(
        &mut conn,
        Pass {
            source: "feed",
            started_at: 10,
            finished_at: 20,
            failure: None,
            cursor: Some("page-7".into()),
        },
        |_| Ok(7),
    )
    .unwrap();
    let begun = now_ms();
    started(&conn, "feed", begun).unwrap();
    assert!(syncing(&sync(&conn, "feed").unwrap().unwrap()));
    failed(&conn, "feed", &Failure::new(Fault::Unreachable, "gone")).unwrap();
    let seen = sync(&conn, "feed").unwrap().unwrap();
    assert_eq!(seen.started_at, Some(begun));
    assert!(seen.started_at <= seen.finished_at);
    assert!(!syncing(&seen));
    assert!(seen.finished_at.unwrap() <= now_ms());
    assert_eq!(seen.ok, Some(false));
    assert_eq!(seen.error.as_deref(), Some("unreachable: gone"));
    assert_eq!(seen.rows, Some(7));
    assert_eq!(seen.cursor.as_deref(), Some("page-7"));
    assert_eq!(retry_after(&conn, "feed").unwrap(), None);
    assert!(due(&conn, "feed", Duration::from_secs(60), now_ms()));

    let before = now_ms();
    failed(&conn, "new", &Failure::new(Fault::NoKey, "sign in first")).unwrap();
    let first = sync(&conn, "new").unwrap().unwrap();
    assert!(first.finished_at.unwrap() >= before);
    assert_eq!(
        (first.started_at, first.rows, first.cursor),
        (None, None, None)
    );

    let ahead = now_ms() + 3_600_000;
    started(&conn, "ahead", ahead).unwrap();
    failed(&conn, "ahead", &Failure::new(Fault::Refused, "no")).unwrap();
    let skewed = sync(&conn, "ahead").unwrap().unwrap();
    assert_eq!(skewed.finished_at, Some(ahead));
    assert!(!syncing(&skewed));
}

struct Gated {
    go: Receiver<()>,
    passes: usize,
}

impl Source for Gated {
    fn name(&self) -> &str {
        "gated"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn pass(&mut self, conn: &mut Connection, _: Option<&str>) -> Result<usize, Failure> {
        self.go
            .recv_timeout(WAIT)
            .map_err(|e| Failure::new(Fault::Unreachable, e.to_string()))?;
        self.passes += 1;
        let n = self.passes;
        commit(
            conn,
            Pass {
                source: "gated",
                started_at: 1,
                finished_at: 2,
                failure: None,
                cursor: None,
            },
            |_| Ok(n),
        )
        .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}

#[test]
fn changed_goes_out_when_a_pass_starts_and_again_when_it_ends() {
    let spec = Spec::new(scratch("announced").join("app.db"), 1, NOTES);
    let (go, gate) = mpsc::channel();
    let (changed, seen) = mpsc::channel();
    let asks = spawn(
        &spec,
        Box::new(Gated {
            go: gate,
            passes: 0,
        }),
        changed,
    );
    for round in 1..=2 {
        assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("gated".into()));
        let conn = reader(&spec).unwrap().unwrap();
        assert!(syncing(&sync(&conn, "gated").unwrap().unwrap()), "{round}");
        assert!(seen.try_recv().is_err(), "{round}");
        go.send(()).unwrap();
        assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("gated".into()));
        let done = sync(&conn, "gated").unwrap().unwrap();
        assert!(!syncing(&done), "{round}");
        assert_eq!(done.rows, Some(round), "{round}");
        if round == 1 {
            asks.send(Resync("gated".into(), Some("full".into())).into())
                .unwrap();
        }
    }
}

#[test]
fn the_first_pass_runs_at_once_without_a_resync() {
    let spec = Spec::new(scratch("at-once").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let begun = Instant::now();
    let _asks = spawn(
        &spec,
        Counter::new("hourly", Some(Duration::from_secs(3600))),
        changed,
    );
    assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("hourly".into()));
    let waited = begun.elapsed();
    assert!(waited < SECOND, "{waited:?}");
    assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("hourly".into()));
    let conn = reader(&spec).unwrap().unwrap();
    assert_eq!(
        sync(&conn, "hourly").unwrap().unwrap().cursor.as_deref(),
        Some("1")
    );
}

#[test]
fn an_explicit_resync_always_runs_even_right_after_a_pass() {
    let spec = Spec::new(scratch("explicit").join("app.db"), 1, NOTES);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, Counter::new("asked", None), changed);
    passed(&seen, "asked");
    asks.send(Resync("asked".into(), None).into()).unwrap();
    passed(&seen, "asked");
    asks.send(Resync("asked".into(), None).into()).unwrap();
    passed(&seen, "asked");
    let conn = reader(&spec).unwrap().unwrap();
    assert_eq!(
        sync(&conn, "asked").unwrap().unwrap().cursor.as_deref(),
        Some("3")
    );
}

#[test]
fn a_held_retry_after_keeps_the_first_pass_back() {
    let spec = Spec::new(scratch("held").join("app.db"), 1, NOTES);
    let conn = open(&spec).unwrap();
    let hold = Failure::new(Fault::RateLimited, "slow down").until(Some(now_ms() + 3_600_000));
    failed(&conn, "held", &hold).unwrap();
    drop(conn);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(
        &spec,
        Counter::new("held", Some(Duration::from_millis(20))),
        changed,
    );
    assert!(seen.recv_timeout(SECOND).is_err());
    asks.send(Resync("held".into(), None).into()).unwrap();
    passed(&seen, "held");
    let conn = reader(&spec).unwrap().unwrap();
    let ran = sync(&conn, "held").unwrap().unwrap();
    assert_eq!(ran.cursor.as_deref(), Some("1"));
    assert_eq!(ran.ok, Some(true));
}

struct Refused;

impl Source for Refused {
    fn name(&self) -> &str {
        "refused:app"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn pass(&mut self, _: &mut Connection, _: Option<&str>) -> Result<usize, Failure> {
        Err(Failure::new(Fault::BadAnswer, "nope").until(Some(9)))
    }
}

#[test]
fn pass_once_runs_the_runners_sequence_without_a_thread() {
    let spec = Spec::new(scratch("once").join("app.db"), 1, NOTES);
    let mut conn = open(&spec).unwrap();
    let mut counter = Counter::new("count:app", None);
    assert_eq!(pass_once(&mut conn, counter.as_mut(), None), Ok(1));
    assert_eq!(pass_once(&mut conn, counter.as_mut(), Some("x")), Ok(2));
    let done = sync(&conn, "count:app").unwrap().unwrap();
    assert_eq!(done.ok, Some(true));
    assert_eq!(done.cursor.as_deref(), Some("2"));
    let before = now_ms();
    let failure = pass_once(&mut conn, &mut Refused, None).unwrap_err();
    assert_eq!(failure.fault, Fault::BadAnswer);
    let after = sync(&conn, "refused:app").unwrap().unwrap();
    assert_eq!(after.ok, Some(false));
    assert_eq!(after.error.as_deref(), Some(failure.text().as_str()));
    assert!(after.started_at.unwrap() >= before);
    assert!(after.finished_at >= after.started_at);
    assert!(!syncing(&after));
    assert_eq!(retry_after(&conn, "refused:app").unwrap(), Some(9));
}
