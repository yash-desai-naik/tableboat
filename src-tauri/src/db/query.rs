//! Running SQL. Statements arrive already split by the editor. Rows stay in Rust
//! (`results::ResultSet`); the UI only ever fetches the rows it is drawing.
//!
//! MySQL runs on the session's editor connection (session state carries over between runs)
//! or, for table views, on a pooled connection. Cancel = `KILL QUERY` from another
//! connection. SQLite runs on its single connection; cancel = `sqlite3_interrupt`.

use super::meta::quote_mysql;
use super::results::{Cell, Column, ResultSet};
use super::{Backend, SessionHandle};
use futures_util::StreamExt;
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::*;
use mysql_async::{Row, Value};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;

/// Outcome of one statement.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementOutcome {
    pub index: usize,
    pub sql: String,
    pub elapsed_ms: f64,
    /// Rows, when the statement returned a result set. Kept in Rust; the UI gets `result_info`.
    #[serde(skip)]
    pub result: Option<ResultSet>,
    pub result_info: Option<super::results::ResultInfo>,
    pub affected_rows: Option<u64>,
    pub last_insert_id: Option<u64>,
    pub warnings: u16,
    pub info: Option<String>,
    pub error: Option<String>,
    pub cancelled: bool,
}

impl StatementOutcome {
    pub fn new(index: usize, sql: &str) -> Self {
        Self {
            index,
            sql: sql.to_string(),
            elapsed_ms: 0.0,
            result: None,
            result_info: None,
            affected_rows: None,
            last_insert_id: None,
            warnings: 0,
            info: None,
            error: None,
            cancelled: false,
        }
    }
}

/// How to stop a running query.
pub enum CancelHandle {
    Mysql { pool: mysql_async::Pool, connection_id: Arc<std::sync::Mutex<Option<u32>>> },
    Sqlite(Arc<super::sqlite::SqliteBackend>),
    /// Wakes the running operation, which then drops it (and its server cursor).
    Mongo(Arc<tokio::sync::Notify>),
}

struct RunEntry {
    cancelled: Arc<AtomicBool>,
    handle: CancelHandle,
}

/// Queries currently running, by run id.
#[derive(Default)]
pub struct Runs {
    map: Mutex<HashMap<String, RunEntry>>,
}

impl Runs {
    pub async fn cancel(&self, run_id: &str) -> bool {
        let entry = self.map.lock().await.remove(run_id);
        let Some(entry) = entry else { return false };
        entry.cancelled.store(true, Ordering::SeqCst);
        match entry.handle {
            CancelHandle::Mysql { pool, connection_id } => {
                let id = *connection_id.lock().unwrap();
                if let Some(id) = id {
                    kill_query(&pool, id).await;
                }
            }
            CancelHandle::Sqlite(b) => b.interrupt.interrupt(),
            CancelHandle::Mongo(n) => n.notify_one(),
        }
        true
    }
}

async fn kill_query(pool: &mysql_async::Pool, connection_id: u32) {
    if let Ok(mut c) = pool.get_conn().await {
        let _ = c.query_drop(format!("KILL QUERY {connection_id}")).await;
    }
}

pub struct RunOptions {
    pub database: Option<String>,
    pub max_rows: usize,
    /// Run on a pooled connection instead of the editor's own (table views).
    pub use_pool: bool,
    /// Stop at the first failing statement.
    pub stop_on_error: bool,
}

pub async fn run(
    session: &SessionHandle,
    runs: &Runs,
    run_id: &str,
    statements: Vec<String>,
    opts: RunOptions,
) -> Result<Vec<StatementOutcome>, String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcome = match &session.backend {
        Backend::Mysql(b) => {
            let connection_id = Arc::new(std::sync::Mutex::new(None));
            runs.map.lock().await.insert(
                run_id.to_string(),
                RunEntry {
                    cancelled: cancelled.clone(),
                    handle: CancelHandle::Mysql { pool: b.pool.clone(), connection_id: connection_id.clone() },
                },
            );
            Ok(run_mysql(b, statements, &opts, cancelled.clone(), connection_id).await)
        }
        Backend::Mongo(b) => {
            let notify = Arc::new(tokio::sync::Notify::new());
            runs.map.lock().await.insert(
                run_id.to_string(),
                RunEntry { cancelled: cancelled.clone(), handle: CancelHandle::Mongo(notify.clone()) },
            );
            Ok(super::mongo::run(b, statements, opts.database.clone(), opts.max_rows, opts.stop_on_error, cancelled.clone(), notify).await)
        }
        Backend::Sqlite(b) => {
            runs.map.lock().await.insert(
                run_id.to_string(),
                RunEntry { cancelled: cancelled.clone(), handle: CancelHandle::Sqlite(b.clone()) },
            );
            let b = b.clone();
            let flag = cancelled.clone();
            let max_rows = opts.max_rows;
            let stop = opts.stop_on_error;
            tokio::task::spawn_blocking(move || run_sqlite(&b, statements, max_rows, stop, flag))
                .await
                .map_err(|e| e.to_string())
        }
    };
    runs.map.lock().await.remove(run_id);
    outcome
}

// ---------- MySQL ----------

async fn run_mysql(
    b: &super::mysql::MysqlBackend,
    statements: Vec<String>,
    opts: &RunOptions,
    cancelled: Arc<AtomicBool>,
    connection_id: Arc<std::sync::Mutex<Option<u32>>>,
) -> Vec<StatementOutcome> {
    // The editor connection is locked for the whole run (two editor runs queue up). It's
    // taken out of its slot while in use and put back afterwards unless it broke.
    let mut editor = if opts.use_pool { None } else { Some(b.editor.clone().lock_owned().await) };
    let mut conn = match editor.as_mut().and_then(|slot| slot.take()) {
        Some(c) => c,
        None => match b.pool.get_conn().await {
            Ok(c) => c,
            Err(e) => return vec![failed(0, statements.first(), mysql_message(&e))],
        },
    };
    *connection_id.lock().unwrap() = Some(conn.id());
    let mut lost = false;

    if let Some(db) = opts.database.as_deref().filter(|d| !d.is_empty()) {
        if let Err(e) = conn.query_drop(format!("USE {}", quote_mysql(db))).await {
            if !is_connection_error(&e) {
                if let Some(slot) = editor.as_mut() {
                    **slot = Some(conn);
                }
            }
            return vec![failed(0, statements.first(), mysql_message(&e))];
        }
    }

    let mut out = Vec::with_capacity(statements.len());
    for (index, sql) in statements.iter().enumerate() {
        if cancelled.load(Ordering::SeqCst) {
            let mut o = StatementOutcome::new(index, sql);
            o.cancelled = true;
            o.error = Some("Cancelled before it ran.".into());
            out.push(o);
            continue;
        }
        let started = Instant::now();
        let mut o = StatementOutcome::new(index, sql);
        let res = run_one_mysql(&mut conn, sql, opts.max_rows, &b.pool, &mut o).await;
        o.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        // KILL QUERY can end a statement without an error (e.g. SLEEP returns 1): still mark it.
        if res.is_ok() && cancelled.load(Ordering::SeqCst) {
            o.cancelled = true;
            o.info = Some("Cancelled.".into());
        }
        if let Err(e) = res {
            if cancelled.load(Ordering::SeqCst) {
                o.cancelled = true;
                o.error = Some("Cancelled.".into());
            } else {
                o.error = Some(mysql_message(&e));
            }
            lost = is_connection_error(&e);
            out.push(o);
            if lost {
                break;
            }
            if opts.stop_on_error || cancelled.load(Ordering::SeqCst) {
                break;
            }
            continue;
        }
        out.push(o);
    }

    // Return the editor connection to its slot (a pooled one goes back to the pool on drop).
    if let Some(slot) = editor.as_mut() {
        if !lost {
            **slot = Some(conn);
        }
    }
    out
}

async fn run_one_mysql(
    conn: &mut mysql_async::Conn,
    sql: &str,
    max_rows: usize,
    pool: &mysql_async::Pool,
    o: &mut StatementOutcome,
) -> Result<(), mysql_async::Error> {
    let connection_id = conn.id();
    let mut result = conn.query_iter(sql).await?;
    let mut truncated_early = false;

    loop {
        let Some(mut stream) = result.stream::<Row>().await? else { break };
        let columns = stream.columns();
        if columns.is_empty() {
            o.affected_rows = Some(o.affected_rows.unwrap_or(0) + stream.affected_rows());
            o.last_insert_id = stream.last_insert_id().or(o.last_insert_id);
            o.warnings = stream.get_warnings();
            let info = stream.info();
            if !info.is_empty() {
                o.info = Some(info.into_owned());
            }
            // Drain the (empty) stream.
            while stream.next().await.is_some() {}
            continue;
        }

        let cols: Vec<Column> = columns.iter().map(describe_mysql_column).collect();
        let binary: Vec<bool> = cols.iter().map(|c| c.binary).collect();
        let mut rows: Vec<Box<[Cell]>> = Vec::new();
        let mut truncated = false;
        while let Some(row) = stream.next().await {
            let row = row?;
            if rows.len() >= max_rows {
                truncated = true;
                break;
            }
            rows.push(
                row.unwrap_raw()
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| mysql_cell(v, binary.get(i).copied().unwrap_or(false)))
                    .collect(),
            );
        }
        o.warnings = stream.get_warnings();
        // Keep the first result set (a procedure may return several; later ones are counted).
        if o.result.is_none() {
            o.result = Some(ResultSet::new(cols, rows, truncated));
        }
        if truncated {
            truncated_early = true;
            break;
        }
    }

    if truncated_early {
        // Stop the server sending the rest instead of downloading it all.
        kill_query(pool, connection_id).await;
        let _ = result.drop_result().await;
        // KILL QUERY can leave an error behind on the connection; clear it.
        let _ = conn.query_drop("DO 0").await;
    }
    Ok(())
}

fn failed(index: usize, sql: Option<&String>, message: String) -> StatementOutcome {
    let mut o = StatementOutcome::new(index, sql.map(String::as_str).unwrap_or(""));
    o.error = Some(message);
    o
}

fn is_connection_error(e: &mysql_async::Error) -> bool {
    matches!(e, mysql_async::Error::Io(_)) || e.to_string().to_lowercase().contains("connection")
        && !matches!(e, mysql_async::Error::Server(_))
}

pub fn mysql_message(e: &mysql_async::Error) -> String {
    match e {
        mysql_async::Error::Server(s) => format!("{} (MySQL error {}, SQLSTATE {})", s.message, s.code, s.state),
        other => other.to_string(),
    }
}

fn mysql_cell(v: Option<Value>, binary: bool) -> Cell {
    match v {
        None | Some(Value::NULL) => None,
        Some(Value::Bytes(bytes)) => Some(if binary { hex(&bytes) } else { String::from_utf8_lossy(&bytes).into_owned() }.into_boxed_str()),
        Some(Value::Int(i)) => Some(i.to_string().into_boxed_str()),
        Some(Value::UInt(u)) => Some(u.to_string().into_boxed_str()),
        Some(Value::Float(f)) => Some(f.to_string().into_boxed_str()),
        Some(Value::Double(d)) => Some(d.to_string().into_boxed_str()),
        Some(other) => Some(other.as_sql(false).trim_matches('\'').to_string().into_boxed_str()),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

fn describe_mysql_column(c: &mysql_async::Column) -> Column {
    use ColumnType::*;
    let ty = c.column_type();
    let flags = c.flags();
    let binary_charset = c.character_set() == 63;
    let unsigned = flags.contains(ColumnFlags::UNSIGNED_FLAG);
    let (name, numeric) = match ty {
        MYSQL_TYPE_TINY => ("tinyint", true),
        MYSQL_TYPE_SHORT => ("smallint", true),
        MYSQL_TYPE_INT24 => ("mediumint", true),
        MYSQL_TYPE_LONG => ("int", true),
        MYSQL_TYPE_LONGLONG => ("bigint", true),
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => ("decimal", true),
        MYSQL_TYPE_FLOAT => ("float", true),
        MYSQL_TYPE_DOUBLE => ("double", true),
        MYSQL_TYPE_YEAR => ("year", true),
        MYSQL_TYPE_BIT => ("bit", false),
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => ("date", false),
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => ("time", false),
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => ("datetime", false),
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => ("timestamp", false),
        MYSQL_TYPE_JSON => ("json", false),
        MYSQL_TYPE_ENUM => ("enum", false),
        MYSQL_TYPE_SET => ("set", false),
        MYSQL_TYPE_GEOMETRY => ("geometry", false),
        MYSQL_TYPE_VECTOR => ("vector", false),
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => {
            (if binary_charset { "blob" } else { "text" }, false)
        }
        MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_VARCHAR => (if binary_charset { "varbinary" } else { "varchar" }, false),
        MYSQL_TYPE_STRING => {
            if flags.contains(ColumnFlags::ENUM_FLAG) {
                ("enum", false)
            } else if flags.contains(ColumnFlags::SET_FLAG) {
                ("set", false)
            } else {
                (if binary_charset { "binary" } else { "char" }, false)
            }
        }
        MYSQL_TYPE_NULL => ("null", false),
        _ => ("unknown", false),
    };
    let binary = binary_charset
        && matches!(
            ty,
            MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB
                | MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_VARCHAR | MYSQL_TYPE_STRING | MYSQL_TYPE_GEOMETRY
        )
        || ty == MYSQL_TYPE_BIT;
    Column {
        name: c.name_str().into_owned(),
        type_name: if unsigned && numeric { format!("{name} unsigned") } else { name.to_string() },
        table: Some(c.org_table_str().into_owned()).filter(|t| !t.is_empty()),
        numeric,
        binary,
        primary_key: flags.contains(ColumnFlags::PRI_KEY_FLAG),
    }
}

// ---------- SQLite ----------

fn run_sqlite(
    b: &super::sqlite::SqliteBackend,
    statements: Vec<String>,
    max_rows: usize,
    stop_on_error: bool,
    cancelled: Arc<AtomicBool>,
) -> Vec<StatementOutcome> {
    let conn = match b.conn.lock() {
        Ok(c) => c,
        Err(e) => return vec![failed(0, statements.first(), e.to_string())],
    };
    let mut out = Vec::with_capacity(statements.len());
    for (index, sql) in statements.iter().enumerate() {
        let mut o = StatementOutcome::new(index, sql);
        if cancelled.load(Ordering::SeqCst) {
            o.cancelled = true;
            o.error = Some("Cancelled before it ran.".into());
            out.push(o);
            continue;
        }
        let started = Instant::now();
        let res = run_one_sqlite(&conn, sql, max_rows, &mut o);
        o.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if let Err(e) = res {
            if cancelled.load(Ordering::SeqCst) || matches!(e, rusqlite::Error::SqliteFailure(ref f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted) {
                o.cancelled = true;
                o.error = Some("Cancelled.".into());
            } else {
                o.error = Some(e.to_string());
            }
            out.push(o);
            if stop_on_error || cancelled.load(Ordering::SeqCst) {
                break;
            }
            continue;
        }
        out.push(o);
    }
    out
}

fn run_one_sqlite(conn: &rusqlite::Connection, sql: &str, max_rows: usize, o: &mut StatementOutcome) -> rusqlite::Result<()> {
    use rusqlite::types::ValueRef;
    let mut stmt = conn.prepare(sql)?;
    if stmt.column_count() == 0 {
        stmt.raw_execute()?;
        o.affected_rows = Some(conn.changes());
        let id = conn.last_insert_rowid();
        if id > 0 && sql.trim_start().to_ascii_lowercase().starts_with("insert") {
            o.last_insert_id = Some(id as u64);
        }
        return Ok(());
    }

    let declared: Vec<(String, Option<String>)> = stmt
        .columns()
        .map(|c| (c.name().to_string(), c.decl_type().map(str::to_string)))
        .collect();
    let n = declared.len();
    // Columns are numeric when every non-NULL value is an integer or real.
    let mut numeric = vec![true; n];
    let mut seen = vec![false; n];
    let mut blob = vec![false; n];
    let mut rows: Vec<Box<[Cell]>> = Vec::new();
    let mut truncated = false;

    let mut q = stmt.raw_query();
    while let Some(row) = q.next()? {
        if rows.len() >= max_rows {
            truncated = true;
            break;
        }
        let mut cells = Vec::with_capacity(n);
        for i in 0..n {
            let v = row.get_ref(i)?;
            let cell = match v {
                ValueRef::Null => None,
                ValueRef::Integer(x) => {
                    seen[i] = true;
                    Some(x.to_string())
                }
                ValueRef::Real(x) => {
                    seen[i] = true;
                    Some(format_real(x))
                }
                ValueRef::Text(t) => {
                    seen[i] = true;
                    numeric[i] = false;
                    Some(String::from_utf8_lossy(t).into_owned())
                }
                ValueRef::Blob(bytes) => {
                    seen[i] = true;
                    numeric[i] = false;
                    blob[i] = true;
                    Some(hex(bytes))
                }
            };
            cells.push(cell.map(String::into_boxed_str));
        }
        rows.push(cells.into_boxed_slice());
    }
    drop(q);

    let cols = declared
        .into_iter()
        .enumerate()
        .map(|(i, (name, decl))| {
            let decl_upper = decl.clone().unwrap_or_default().to_ascii_uppercase();
            let declared_numeric = ["INT", "REAL", "FLOA", "DOUB", "NUM", "DEC"].iter().any(|k| decl_upper.contains(k));
            Column {
                name,
                type_name: decl.map(|d| d.to_lowercase()).unwrap_or_default(),
                table: None,
                numeric: if seen[i] { numeric[i] } else { declared_numeric },
                binary: blob[i],
                primary_key: false,
            }
        })
        .collect();
    o.result = Some(ResultSet::new(cols, rows, truncated));
    Ok(())
}

/// Reals without a trailing ".0" noise but never in exponent form for ordinary values.
fn format_real(x: f64) -> String {
    let s = x.to_string();
    if s.contains('e') || s.contains('E') { format!("{x:?}") } else { s }
}
