//! SQLite files through rusqlite (SQLite compiled in, no DLL to install).

use super::ServerInfo;
use crate::conn::model::{Connection, Engine};
use crate::paths::expand_home;
use rusqlite::OpenFlags;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

pub struct SqliteBackend {
    pub path: PathBuf,
    pub conn: Mutex<rusqlite::Connection>,
    pub read_only: bool,
    /// Stops the statement currently running on `conn` (used by Cancel).
    pub interrupt: rusqlite::InterruptHandle,
}

/// Files under \\wsl$ are reached over a network file system without reliable locking,
/// which corrupts or locks SQLite databases; open those read-only.
fn is_wsl_or_network(path: &Path) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase();
    s.starts_with(r"\\wsl$") || s.starts_with(r"\\wsl.localhost") || s.starts_with(r"\\")
}

impl SqliteBackend {
    pub fn open(conn: &Connection) -> Result<(Self, ServerInfo), String> {
        let file = conn.file.as_deref().filter(|f| !f.trim().is_empty()).ok_or("Choose a database file.")?;
        let path = expand_home(file.trim());
        if !path.exists() {
            return Err(format!("The file {} doesn't exist.", path.display()));
        }
        let read_only = is_wsl_or_network(&path);
        let flags = if read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
        };
        let db = rusqlite::Connection::open_with_flags(&path, flags)
            .map_err(|e| format!("Could not open {}: {e}", path.display()))?;
        db.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;

        // Reading the schema proves this is really a SQLite database.
        let check: Result<i64, _> = db.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0));
        if let Err(e) = check {
            return Err(format!("{} is not a SQLite database ({e}).", path.display()));
        }
        let version: String = db
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;

        let info = ServerInfo {
            engine: Some(Engine::Sqlite),
            product: "SQLite".into(),
            full_version: version.clone(),
            version,
            database: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            tls: false,
            ssh: false,
        };
        let interrupt = db.get_interrupt_handle();
        Ok((Self { path, conn: Mutex::new(db), read_only, interrupt }, info))
    }
}

/// Create a new, empty SQLite database file.
pub fn create_file(path: &str) -> Result<(), String> {
    let path = expand_home(path);
    if path.exists() {
        return Err(format!("{} already exists.", path.display()));
    }
    let db = rusqlite::Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    // An empty database file is only written once something is stored.
    db.execute_batch("PRAGMA user_version = 0; VACUUM;")
        .map_err(|e| format!("Could not initialise {}: {e}", path.display()))?;
    Ok(())
}
