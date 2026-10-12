use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

pub const SYNC_VERSION: i64 = 1;

pub(crate) const STRIDE: i64 = 1000;

pub const MAX_VERSION: i64 = (i32::MAX as i64 - (STRIDE - 1)) / STRIDE;

const _: () = assert!(SYNC_VERSION > 0 && SYNC_VERSION < STRIDE);

const SYNC: &str = "CREATE TABLE sync (
  source TEXT PRIMARY KEY,
  started_at INTEGER,
  finished_at INTEGER,
  ok INTEGER,
  error TEXT,
  cursor TEXT,
  rows INTEGER,
  retry_after INTEGER
)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub path: PathBuf,
    pub version: i64,
    pub schema: &'static [&'static str],
}

impl Spec {
    pub fn new(path: impl Into<PathBuf>, version: i64, schema: &'static [&'static str]) -> Self {
        Spec {
            path: path.into(),
            version,
            schema,
        }
    }

    pub fn user_version(&self) -> i64 {
        self.version
            .saturating_mul(STRIDE)
            .saturating_add(SYNC_VERSION)
    }

    fn stamp(&self) -> Result<i64, String> {
        if (0..=MAX_VERSION).contains(&self.version) {
            Ok(self.user_version())
        } else {
            Err(format!(
                "{}: version {} is outside 0..={MAX_VERSION}",
                self.path.display(),
                self.version
            ))
        }
    }
}

fn private(path: &Path, mode: u32) -> Result<(), String> {
    let meta = fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.permissions().mode() & 0o777 != mode {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("cannot protect {}: {e}", path.display()))?;
    }
    Ok(())
}

fn pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")
}

pub fn version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn remove(path: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let file = PathBuf::from(name);
        match fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", file.display())),
        }
    }
    Ok(())
}

fn build(conn: &mut Connection, schema: &[&str], stamp: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(SYNC)?;
    for statement in schema {
        tx.execute_batch(statement)?;
    }
    tx.pragma_update(None, "user_version", stamp)?;
    tx.commit()
}

fn connect(path: &Path) -> Result<Connection, String> {
    let shown = path.display();
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {shown}: {e}"))?;
    private(path, 0o600)?;
    let conn = Connection::open(path).map_err(|e| format!("cannot open {shown}: {e}"))?;
    pragmas(&conn).map_err(|e| format!("cannot prepare {shown}: {e}"))?;
    Ok(conn)
}

pub fn open(spec: &Spec) -> Result<Connection, String> {
    let stamp = spec.stamp()?;
    let path = spec.path.as_path();
    let shown = path.display();
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
        private(dir, 0o700)?;
    }
    let found = connect(path).and_then(|conn| {
        version(&conn)
            .map(|found| (conn, found))
            .map_err(|e| format!("cannot read {shown}: {e}"))
    });
    match found {
        Ok((conn, found)) if found == stamp => return Ok(conn),
        other => drop(other),
    }
    remove(path)?;
    let mut conn = connect(path)?;
    build(&mut conn, spec.schema, stamp).map_err(|e| format!("cannot build {shown}: {e}"))?;
    Ok(conn)
}

pub fn reader(spec: &Spec) -> Result<Option<Connection>, String> {
    let stamp = spec.stamp()?;
    let path = spec.path.as_path();
    if !path.exists() {
        return Ok(None);
    }
    let shown = path.display();
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("cannot open {shown}: {e}"))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| format!("cannot read {shown}: {e}"))?;
    match version(&conn) {
        Ok(found) if found == stamp => Ok(Some(conn)),
        Ok(_) => Ok(None),
        Err(e) => Err(format!("cannot read {shown}: {e}")),
    }
}

pub fn memory(spec: &Spec) -> Result<Connection, String> {
    let stamp = spec.stamp()?;
    let mut conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    pragmas(&conn).map_err(|e| e.to_string())?;
    build(&mut conn, spec.schema, stamp).map_err(|e| e.to_string())?;
    Ok(conn)
}
