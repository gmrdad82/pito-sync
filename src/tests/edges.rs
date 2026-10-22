use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender};

use rusqlite::ErrorCode;

use super::*;

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn has_sqlite_code(error: &rusqlite::Error, code: ErrorCode) -> bool {
    error.sqlite_error_code() == Some(code)
}

#[test]
fn open_under_a_file_names_the_folder_and_leaves_the_file_alone() {
    let base = scratch("under-file");
    let blocker = base.join("journal");
    fs::write(&blocker, "a file, not a folder").unwrap();
    let error = open(&Spec::new(blocker.join("journal.db"), 1, NOTES)).unwrap_err();
    assert!(error.contains(&shown(&blocker)), "{error}");
    assert!(!error.contains("journal.db"), "{error}");
    assert_eq!(
        fs::read_to_string(&blocker).unwrap(),
        "a file, not a folder"
    );
}

#[test]
fn a_schema_that_fails_leaves_an_unstamped_file_the_next_open_rebuilds() {
    const BROKEN: &[&str] = &[
        "CREATE TABLE notes (id INTEGER PRIMARY KEY)",
        "CREATE TABL oops",
    ];
    const CLASH: &[&str] = &["CREATE TABLE sync (source TEXT)"];
    for (case, schema) in [("broken", BROKEN), ("clash", CLASH)] {
        let file = scratch(&format!("schema-{case}")).join("journal.db");
        let bad = Spec::new(&file, 1, schema);
        let error = open(&bad).unwrap_err();
        assert!(error.contains(&shown(&file)), "{case}: {error}");
        assert!(reader(&bad).unwrap().is_none(), "{case}");
        let conn = Connection::open(&file).unwrap();
        assert_eq!(version(&conn).unwrap(), 0, "{case}");
        assert!(tables(&conn).is_empty(), "{case}");
        drop(conn);

        let good = Spec::new(&file, 1, NOTES);
        let mut conn = open(&good).unwrap();
        assert_eq!(version(&conn).unwrap(), good.user_version(), "{case}");
        assert_eq!(tables(&conn), ["notes", "sync"], "{case}");
        assert_eq!(columns(&conn, "sync"), SYNC_COLUMNS, "{case}");
        mark(&mut conn, "journal", 10, None);
        assert_eq!(mode(&file), 0o600, "{case}");
        assert!(reader(&good).unwrap().is_some(), "{case}");
    }
}

#[test]
fn a_memory_database_with_a_failing_schema_is_an_error() {
    let bad: &[&str] = &[
        "CREATE TABLE notes (id INTEGER PRIMARY KEY)",
        "CREATE TABL oops",
    ];
    assert!(memory(&Spec::new("unused.db", 1, bad)).is_err());
    let clash: &[&str] = &["CREATE TABLE sync (source TEXT)"];
    assert!(memory(&Spec::new("unused.db", 1, clash)).is_err());
}

#[test]
fn a_reader_never_changes_the_file_it_refuses() {
    let file = scratch("reader-refuses").join("journal.db");
    let old = Spec::new(&file, 1, NOTES);
    let mut conn = open(&old).unwrap();
    conn.execute("INSERT INTO notes (body) VALUES ('kept')", [])
        .unwrap();
    mark(&mut conn, "journal", 10, None);
    drop(conn);
    let before = fs::read(&file).unwrap();

    assert!(reader(&Spec::new(&file, 2, NOTES)).unwrap().is_none());
    for version in [-1, MAX_VERSION + 1] {
        let error = reader(&Spec::new(&file, version, NOTES)).unwrap_err();
        assert!(error.contains(&shown(&file)), "{version}: {error}");
    }
    assert_eq!(fs::read(&file).unwrap(), before);
    let conn = reader(&old).unwrap().unwrap();
    assert_eq!(count(&conn, "notes"), 1);
    assert_eq!(
        sync(&conn, "journal").unwrap().unwrap().finished_at,
        Some(10)
    );
}

#[test]
fn a_reader_on_junk_is_an_error_and_leaves_the_junk() {
    let file = scratch("reader-junk").join("journal.db");
    let junk = "not a database at all, just some text that is long enough";
    fs::write(&file, junk).unwrap();
    let error = reader(&Spec::new(&file, 1, NOTES)).unwrap_err();
    assert!(error.contains(&shown(&file)), "{error}");
    assert_eq!(fs::read_to_string(&file).unwrap(), junk);
}

#[test]
fn the_user_version_saturates_at_the_extremes() {
    assert_eq!(
        Spec::new("journal.db", i64::MAX, NOTES).user_version(),
        i64::MAX
    );
    assert_eq!(
        Spec::new("journal.db", i64::MIN, NOTES).user_version(),
        i64::MIN + 1
    );
}

#[test]
fn a_rebuild_unlinks_the_sidecars_another_reader_still_holds() {
    let file = scratch("held-sidecars").join("journal.db");
    let old = Spec::new(&file, 1, NOTES);
    let mut writer = open(&old).unwrap();
    writer
        .execute("INSERT INTO notes (body) VALUES ('old')", [])
        .unwrap();
    mark(&mut writer, "journal", 10, None);
    let held = reader(&old).unwrap().unwrap();
    assert_eq!(count(&held, "notes"), 1);
    drop(writer);
    let inode = |suffix: &str| fs::metadata(sibling(&file, suffix)).unwrap().ino();
    let before = [inode(""), inode("-wal"), inode("-shm")];

    let new = Spec::new(&file, 2, NOTES_PINNED);
    let conn = open(&new).unwrap();
    assert_eq!(version(&conn).unwrap(), new.user_version());
    assert_eq!(columns(&conn, "notes"), ["id", "body", "pinned"]);
    assert_eq!(count(&conn, "notes"), 0);
    assert!(sync(&conn, "journal").unwrap().is_none());
    let after = [inode(""), inode("-wal"), inode("-shm")];
    for (i, suffix) in ["", "-wal", "-shm"].into_iter().enumerate() {
        assert_ne!(before[i], after[i], "{suffix} was reused");
    }
    drop(held);
}

#[test]
fn a_rebuild_that_cannot_remove_a_sidecar_names_it_and_keeps_it() {
    let file = scratch("sidecar").join("journal.db");
    let wal = sibling(&file, "-wal");
    fs::create_dir(&wal).unwrap();
    let spec = Spec::new(&file, 1, NOTES);
    let error = open(&spec).unwrap_err();
    assert!(error.contains(&shown(&wal)), "{error}");
    assert!(wal.is_dir());
    fs::remove_dir(&wal).unwrap();
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
}

#[test]
fn open_on_a_folder_names_it_and_leaves_the_folder_and_its_contents() {
    let file = scratch("folder").join("journal.db");
    fs::create_dir(&file).unwrap();
    let inside = file.join("keep");
    fs::write(&inside, b"mine").unwrap();
    let spec = Spec::new(&file, 1, NOTES);
    let error = open(&spec).unwrap_err();
    assert!(error.contains(&shown(&file)), "{error}");
    assert!(file.is_dir());
    assert_eq!(fs::read(&inside).unwrap(), b"mine");
    fs::remove_dir_all(&file).unwrap();
    let conn = open(&spec).unwrap();
    assert_eq!(version(&conn).unwrap(), spec.user_version());
    assert!(file.is_file());
}

#[test]
fn without_a_sync_table_reads_fail_and_due_says_run() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(NOTES[0]).unwrap();
    assert!(sync(&conn, "feed").is_err());
    assert!(retry_after(&conn, "feed").is_err());
    assert!(started(&conn, "feed", 1).is_err());
    assert!(failed(&conn, "feed", &Failure::new(Fault::Refused, "no")).is_err());
    assert!(due(&conn, "feed", Duration::from_secs(60), 0));
    assert!(due(&conn, "feed", Duration::MAX, 0));

    let error = commit(
        &mut conn,
        Pass {
            source: "feed",
            started_at: 1,
            finished_at: 2,
            failure: None,
            cursor: None,
        },
        |tx| tx.execute("INSERT INTO notes (body) VALUES ('lost')", []),
    )
    .unwrap_err();
    assert!(has_sqlite_code(&error, ErrorCode::Unknown), "{error}");
    assert!(conn.is_autocommit());
    assert_eq!(count(&conn, "notes"), 0);
}

fn pass(
    started_at: i64,
    finished_at: i64,
    failure: Option<Failure>,
    cursor: &str,
) -> Pass<'static> {
    Pass {
        source: "ledger",
        started_at,
        finished_at,
        failure,
        cursor: Some(cursor.to_owned()),
    }
}

#[test]
fn a_failing_write_rolls_back_the_rows_and_the_sync_row() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    commit(&mut conn, pass(10, 20, None, "a"), |tx| {
        tx.execute("INSERT INTO notes (body) VALUES ('kept')", [])
    })
    .unwrap();
    let before = sync(&conn, "ledger").unwrap().unwrap();

    let error = commit(&mut conn, pass(30, 40, None, "b"), |tx| {
        tx.execute("INSERT INTO notes (body) VALUES ('lost')", [])?;
        tx.execute("INSERT INTO notes (body) VALUES (NULL)", [])
    })
    .unwrap_err();
    assert!(
        has_sqlite_code(&error, ErrorCode::ConstraintViolation),
        "{error}"
    );
    assert!(conn.is_autocommit());
    assert_eq!(count(&conn, "notes"), 1);
    assert_eq!(sync(&conn, "ledger").unwrap().unwrap(), before);

    commit(&mut conn, pass(50, 60, None, "c"), |tx| {
        tx.execute("INSERT INTO notes (body) VALUES ('next')", [])
    })
    .unwrap();
    assert_eq!(count(&conn, "notes"), 2);
    assert_eq!(
        sync(&conn, "ledger").unwrap().unwrap().cursor.as_deref(),
        Some("c")
    );
}

#[test]
fn a_failed_commit_stores_its_error_and_hold_and_a_good_one_clears_them() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    let limited = Failure::new(Fault::RateLimited, "slow down").until(Some(77_000));
    let rows = commit(&mut conn, pass(10, 20, Some(limited.clone()), "p3"), |_| {
        Ok(3)
    })
    .unwrap();
    assert_eq!(rows, 3);
    let seen = sync(&conn, "ledger").unwrap().unwrap();
    assert_eq!(
        (seen.started_at, seen.finished_at, seen.ok, seen.rows),
        (Some(10), Some(20), Some(false), Some(3))
    );
    assert_eq!(seen.error, Some(limited.text()));
    assert_eq!(seen.cursor.as_deref(), Some("p3"));
    assert_eq!(retry_after(&conn, "ledger").unwrap(), Some(77_000));

    commit(&mut conn, pass(30, 40, None, "p4"), |_| Ok(0)).unwrap();
    let seen = sync(&conn, "ledger").unwrap().unwrap();
    assert_eq!(
        (seen.ok, seen.error, seen.rows),
        (Some(true), None, Some(0))
    );
    assert_eq!(retry_after(&conn, "ledger").unwrap(), None);
}

#[test]
fn due_holds_until_exactly_every_after_a_good_pass() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    let every = Duration::from_secs(60);
    mark(&mut conn, "feed", 1_000, None);
    assert!(!due(&conn, "feed", every, 60_999));
    assert!(due(&conn, "feed", every, 61_000));
    assert!(due(&conn, "feed", Duration::ZERO, 1_000));
    assert!(!due(&conn, "feed", Duration::from_millis(1), 1_000));
}

#[test]
fn a_huge_every_never_comes_due_after_the_first_pass() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    assert!(due(&conn, "feed", Duration::MAX, 0));
    mark(&mut conn, "feed", 1_000, None);
    for every in [
        Duration::MAX,
        Duration::from_secs(u64::MAX),
        Duration::from_millis(i64::MAX as u64),
    ] {
        assert!(!due(&conn, "feed", every, 2_000), "{every:?}");
        assert!(!due(&conn, "feed", every, i64::MAX), "{every:?}");
    }
}

#[test]
fn due_does_not_overflow_on_a_finish_far_in_the_past() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    commit(&mut conn, pass(i64::MIN, i64::MIN, None, "x"), |_| Ok(0)).unwrap();
    assert!(due(&conn, "ledger", Duration::from_secs(60), i64::MAX));
    assert!(due(
        &conn,
        "ledger",
        Duration::from_millis(i64::MAX as u64),
        i64::MAX
    ));
}

#[test]
fn a_past_hold_after_a_failed_pass_is_due_at_once() {
    let mut conn = memory(&Spec::new("unused.db", 1, NOTES)).unwrap();
    let every = Duration::from_secs(60);
    mark(&mut conn, "feed", 1_000, None);
    assert!(!due(&conn, "feed", every, 2_000));
    let limited = Failure::new(Fault::RateLimited, "slow down").until(Some(4_000));
    mark(&mut conn, "feed", 2_000, Some(limited));
    assert!(!due(&conn, "feed", every, 3_999));
    assert!(due(&conn, "feed", every, 4_000));
    assert!(due(&conn, "feed", every, 5_000));
    failed(
        &conn,
        "feed",
        &Failure::new(Fault::Unreachable, "gone").until(Some(1)),
    )
    .unwrap();
    assert_eq!(retry_after(&conn, "feed").unwrap(), Some(1));
    assert!(due(&conn, "feed", every, 2_000));
}

struct Watched {
    gate: Option<Receiver<()>>,
    passes: Arc<AtomicUsize>,
    gone: Sender<()>,
}

impl Watched {
    fn new(gate: Option<Receiver<()>>) -> (Box<Self>, Arc<AtomicUsize>, Receiver<()>) {
        let passes = Arc::new(AtomicUsize::new(0));
        let (gone, ended) = mpsc::channel();
        let source = Box::new(Watched {
            gate,
            passes: Arc::clone(&passes),
            gone,
        });
        (source, passes, ended)
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let _ = self.gone.send(());
    }
}

impl Source for Watched {
    fn name(&self) -> &str {
        "watched"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn pass(&mut self, conn: &mut Connection, _: Option<&str>) -> Result<usize, Failure> {
        if let Some(gate) = &self.gate {
            gate.recv_timeout(WAIT)
                .map_err(|e| Failure::new(Fault::Unreachable, e.to_string()))?;
        }
        let n = self.passes.fetch_add(1, Ordering::SeqCst) + 1;
        commit(
            conn,
            Pass {
                source: "watched",
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

#[test]
fn a_runner_whose_listener_is_gone_before_the_first_pass_never_passes() {
    for checked in [false, true] {
        let spec = Spec::new(
            scratch(&format!("deaf-{checked}")).join("feed.db"),
            1,
            NOTES,
        );
        let (source, passes, ended) = Watched::new(None);
        let (changed, seen) = mpsc::channel();
        drop(seen);
        let begun = now_ms();
        let _asks = if checked {
            spawn_checked(&spec, source, changed).unwrap()
        } else {
            spawn(&spec, source, changed)
        };
        ended.recv_timeout(WAIT).unwrap();
        assert_eq!(passes.load(Ordering::SeqCst), 0, "{checked}");
        let conn = reader(&spec).unwrap().unwrap();
        let row = sync(&conn, "watched").unwrap().unwrap();
        assert!(row.started_at.unwrap() >= begun, "{checked}");
        assert_eq!(
            (row.finished_at, row.ok, row.error, row.cursor, row.rows),
            (None, None, None, None, None),
            "{checked}"
        );
    }
}

#[test]
fn a_runner_whose_listener_goes_mid_pass_finishes_it_then_stops() {
    let spec = Spec::new(scratch("deaf-mid").join("feed.db"), 1, NOTES);
    let (go, gate) = mpsc::channel();
    let (source, passes, ended) = Watched::new(Some(gate));
    let (changed, seen) = mpsc::channel();
    let _asks = spawn(&spec, source, changed);
    assert_eq!(seen.recv_timeout(WAIT).unwrap(), Changed("watched".into()));
    drop(seen);
    go.send(()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert_eq!(passes.load(Ordering::SeqCst), 1);
    let conn = reader(&spec).unwrap().unwrap();
    let row = sync(&conn, "watched").unwrap().unwrap();
    assert_eq!((row.ok, row.rows), (Some(true), Some(1)));
    assert_eq!(row.cursor.as_deref(), Some("1"));
}

#[test]
fn a_runner_whose_listener_goes_after_a_pass_stops_at_the_next_ask() {
    let spec = Spec::new(scratch("deaf-after").join("feed.db"), 1, NOTES);
    let (source, passes, ended) = Watched::new(None);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, source, changed);
    passed(&seen, "watched");
    drop(seen);
    asks.send(Resync("watched".into(), None).into()).unwrap();
    ended.recv_timeout(WAIT).unwrap();
    assert_eq!(passes.load(Ordering::SeqCst), 1);
    let conn = reader(&spec).unwrap().unwrap();
    let row = sync(&conn, "watched").unwrap().unwrap();
    assert_eq!(row.cursor.as_deref(), Some("1"));
    assert!(syncing(&row));
}

#[test]
fn dropping_the_asks_ends_the_runner() {
    let spec = Spec::new(scratch("hung-up").join("feed.db"), 1, NOTES);
    let (source, passes, ended) = Watched::new(None);
    let (changed, seen) = mpsc::channel();
    let asks = spawn(&spec, source, changed);
    passed(&seen, "watched");
    drop(asks);
    ended.recv_timeout(WAIT).unwrap();
    assert_eq!(seen.recv_timeout(WAIT), Err(RecvTimeoutError::Disconnected));
    assert_eq!(passes.load(Ordering::SeqCst), 1);
}
