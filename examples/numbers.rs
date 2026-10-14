use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use pito_sync::rusqlite::{Connection, OpenFlags, TransactionBehavior};
use pito_sync::{Changed, Failure, Fault, Pass, Resync, Source, Spec};

const SCHEMA: &[&str] = &["CREATE TABLE items (id INTEGER PRIMARY KEY, title TEXT NOT NULL)"];
const ROWS: usize = 10_000;
const BUSY: Duration = Duration::from_secs(3);

fn write(tx: &pito_sync::rusqlite::Transaction, rows: usize) -> pito_sync::rusqlite::Result<usize> {
    tx.execute("DELETE FROM items", [])?;
    let mut insert = tx.prepare_cached("INSERT INTO items (id, title) VALUES (?1, ?2)")?;
    for id in 0..rows {
        insert.execute((id as i64, format!("item {id}")))?;
    }
    Ok(rows)
}

struct Items(usize);

impl Source for Items {
    fn name(&self) -> &str {
        "items"
    }

    fn every(&self) -> Option<Duration> {
        None
    }

    fn pass(&mut self, conn: &mut Connection, _id: Option<&str>) -> Result<usize, Failure> {
        let pass = Pass {
            source: "items",
            started_at: 0,
            finished_at: 0,
            failure: None,
            cursor: None,
        };
        let rows = self.0;
        pito_sync::commit(conn, pass, |tx| write(tx, rows))
            .map_err(|e| Failure::new(Fault::BadAnswer, e.to_string()))
    }
}

struct Stats(Vec<Duration>);

impl Stats {
    fn at(&self, pct: usize) -> Duration {
        let mut sorted = self.0.clone();
        sorted.sort();
        sorted[(sorted.len() * pct / 100).min(sorted.len() - 1)]
    }
}

fn us(d: Duration) -> String {
    let micros = d.as_secs_f64() * 1e6;
    if micros < 1000.0 {
        format!("{micros:.1} µs")
    } else {
        format!("{:.2} ms", micros / 1e3)
    }
}

fn time(runs: usize, mut f: impl FnMut(usize)) -> Stats {
    Stats(
        (0..runs)
            .map(|i| {
                let t = Instant::now();
                f(i);
                t.elapsed()
            })
            .collect(),
    )
}

fn ui_read(conn: &Connection) -> pito_sync::rusqlite::Result<usize> {
    let mut page = conn.prepare_cached("SELECT id, title FROM items ORDER BY id LIMIT 50")?;
    let rows = page.query_map([], |row| row.get::<_, String>(1))?.count();
    pito_sync::sync(conn, "items")?;
    Ok(rows)
}

fn under_write(writer: Connection, reader: Connection) -> (Stats, usize) {
    let stop = Arc::new(AtomicBool::new(false));
    let busy = Arc::clone(&stop);
    let handle = thread::spawn(move || {
        let mut conn = writer;
        let mut passes = 0;
        while !busy.load(Ordering::Relaxed) {
            let pass = Pass {
                source: "items",
                started_at: 0,
                finished_at: 0,
                failure: None,
                cursor: None,
            };
            pito_sync::commit(&mut conn, pass, |tx| write(tx, ROWS)).expect("pass");
            passes += 1;
        }
        passes
    });
    let end = Instant::now() + BUSY;
    let mut reads = Vec::new();
    while Instant::now() < end {
        let t = Instant::now();
        ui_read(&reader).expect("read");
        reads.push(t.elapsed());
        thread::sleep(Duration::from_millis(1));
    }
    stop.store(true, Ordering::Relaxed);
    let passes = handle.join().expect("writer");
    (Stats(reads), passes)
}

fn defaults(path: &Path) -> (Connection, Connection) {
    let mut writer = Connection::open(path).expect("open");
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("tx");
    tx.execute_batch(
        "CREATE TABLE sync (source TEXT PRIMARY KEY, started_at INTEGER, finished_at INTEGER,
           ok INTEGER, error TEXT, cursor TEXT, rows INTEGER, retry_after INTEGER)",
    )
    .expect("sync");
    tx.execute_batch(SCHEMA[0]).expect("schema");
    write(&tx, ROWS).expect("rows");
    tx.commit().expect("commit");
    let reader =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("reader");
    (writer, reader)
}

fn main() -> Result<(), String> {
    let dir: PathBuf = std::env::temp_dir().join("pito-sync-numbers");
    let _ = fs::remove_dir_all(&dir);

    let fresh = time(200, |i| {
        let spec = Spec::new(dir.join(format!("fresh/{i}/app.db")), 1, SCHEMA);
        pito_sync::open(&spec).expect("open");
    });
    println!("{:<34} {}", "open, a new database", us(fresh.at(50)));

    let spec = Spec::new(dir.join("app.db"), 1, SCHEMA);
    let reopen = time(1000, |_| {
        pito_sync::open(&spec).expect("open");
    });
    println!("{:<34} {}", "open, an existing database", us(reopen.at(50)));

    let (changed, changes) = mpsc::channel::<Changed>();
    let asks = pito_sync::spawn_checked(&spec, Box::new(Items(0)), changed)?;
    changes.recv().map_err(|e| e.to_string())?;
    changes.recv().map_err(|e| e.to_string())?;
    let resync = time(2000, |_| {
        asks.send(Resync("items".into(), None).into())
            .expect("send");
        changes.recv().expect("start");
        changes.recv().expect("end");
    });
    drop(asks);
    println!(
        "{:<34} {} (p99 {})",
        "Resync to its closing Changed",
        us(resync.at(50)),
        us(resync.at(99))
    );

    let mut conn = pito_sync::open(&spec)?;
    let mut items = Items(ROWS);
    let pass = time(20, |_| {
        pito_sync::pass_once(&mut conn, &mut items, None).expect("pass");
    });
    let rate = ROWS as f64 / pass.at(50).as_secs_f64();
    println!(
        "{:<34} {} ({:.2} M rows/s)",
        "a pass of 10,000 rows",
        us(pass.at(50)),
        rate / 1e6
    );

    let ui = pito_sync::reader(&spec)?.ok_or("not built")?;
    let idle = time(2000, |_| {
        ui_read(&ui).expect("read");
    });
    println!("{:<34} {}", "a UI read, writer idle", us(idle.at(50)));

    let (reads, passes) = under_write(conn, ui);
    println!(
        "{:<34} {} (p99 {}, max {}; {} reads, {passes} passes)",
        "a UI read, writer busy",
        us(reads.at(50)),
        us(reads.at(99)),
        us(reads.at(100)),
        reads.0.len()
    );

    let plain = dir.join("plain.db");
    let (writer, reader) = defaults(&plain);
    let (reads, passes) = under_write(writer, reader);
    println!(
        "{:<34} {} (p99 {}, max {}; {} reads, {passes} passes)",
        "  SQLite defaults, writer busy",
        us(reads.at(50)),
        us(reads.at(99)),
        us(reads.at(100)),
        reads.0.len()
    );

    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
