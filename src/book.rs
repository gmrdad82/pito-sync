use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::Failure;

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sync {
    pub source: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub ok: Option<bool>,
    pub error: Option<String>,
    pub cursor: Option<String>,
    pub rows: Option<i64>,
}

pub fn sync(conn: &Connection, source: &str) -> rusqlite::Result<Option<Sync>> {
    conn.query_row(
        "SELECT source, started_at, finished_at, ok, error, cursor, rows FROM sync WHERE source = ?1",
        [source],
        |row| {
            Ok(Sync {
                source: row.get(0)?,
                started_at: row.get(1)?,
                finished_at: row.get(2)?,
                ok: row.get(3)?,
                error: row.get(4)?,
                cursor: row.get(5)?,
                rows: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn retry_after(conn: &Connection, source: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT retry_after FROM sync WHERE source = ?1",
        [source],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

pub fn started(conn: &Connection, source: &str, at: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync (source, started_at) VALUES (?1, ?2)
         ON CONFLICT (source) DO UPDATE SET started_at = excluded.started_at",
        (source, at),
    )
    .map(|_| ())
}

pub fn failed(conn: &Connection, source: &str, failure: &Failure) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO sync (source, finished_at, ok, error, retry_after) VALUES (?1, ?2, 0, ?3, ?4)
         ON CONFLICT (source) DO UPDATE SET
           finished_at = max(excluded.finished_at, coalesce(started_at, excluded.finished_at)),
           ok = 0, error = excluded.error, retry_after = excluded.retry_after",
        (source, now_ms(), failure.text(), failure.retry_after),
    )
    .map(|_| ())
}

pub struct Pass<'a> {
    pub source: &'a str,
    pub started_at: i64,
    pub finished_at: i64,
    pub failure: Option<Failure>,
    pub cursor: Option<String>,
}

pub fn commit(
    conn: &mut Connection,
    pass: Pass,
    write: impl FnOnce(&Transaction) -> rusqlite::Result<usize>,
) -> rusqlite::Result<usize> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows = write(&tx)?;
    tx.execute(
        "INSERT INTO sync (source, started_at, finished_at, ok, error, cursor, rows, retry_after)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (source) DO UPDATE SET started_at = excluded.started_at,
           finished_at = excluded.finished_at, ok = excluded.ok, error = excluded.error,
           cursor = excluded.cursor, rows = excluded.rows, retry_after = excluded.retry_after",
        (
            pass.source,
            pass.started_at,
            pass.finished_at,
            pass.failure.is_none(),
            pass.failure.as_ref().map(Failure::text),
            pass.cursor,
            rows as i64,
            pass.failure.as_ref().and_then(|f| f.retry_after),
        ),
    )?;
    tx.commit()?;
    Ok(rows)
}

pub fn due(conn: &Connection, source: &str, every: Duration, now: i64) -> bool {
    if retry_after(conn, source)
        .ok()
        .flatten()
        .is_some_and(|at| now < at)
    {
        return false;
    }
    let finished = sync(conn, source)
        .ok()
        .flatten()
        .filter(|s| s.ok != Some(false))
        .and_then(|s| s.finished_at);
    let every = i64::try_from(every.as_millis()).unwrap_or(i64::MAX);
    finished.is_none_or(|at| now.saturating_sub(at) >= every)
}
