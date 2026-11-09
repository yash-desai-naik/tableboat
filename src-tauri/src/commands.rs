//! Commands the UI calls for connections, sessions and windows.

use crate::conn::model::{Connection, ConnectionSummary, Engine, Secrets};
use crate::conn::store::ConnectionStore;
use crate::conn::{secrets, url};
use crate::db::{self, ServerInfo, Sessions};
use crate::paths::{data_dir, write_atomic};
use serde::Serialize;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, Window};

fn summary(conn: Connection) -> ConnectionSummary {
    let has = |field| secrets::get(&conn.id, field).ok().flatten().is_some();
    ConnectionSummary {
        has_password: has(secrets::PASSWORD),
        has_ssh_password: has(secrets::SSH_PASSWORD),
        has_ssh_passphrase: has(secrets::SSH_PASSPHRASE),
        connection: conn,
    }
}

fn validate(conn: &Connection) -> Result<(), String> {
    match conn.engine {
        Engine::Sqlite => {
            if conn.file.as_deref().unwrap_or("").trim().is_empty() {
                return Err("Choose a database file.".into());
            }
        }
        _ => {
            if conn.host.trim().is_empty() {
                return Err("Enter the host.".into());
            }
        }
    }
    if conn.ssh.enabled && (conn.ssh.host.trim().is_empty() || conn.ssh.user.trim().is_empty()) {
        return Err("Enter the SSH server and user, or turn SSH off.".into());
    }
    let hex = conn.color.trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("The colour must look like #1f6feb.".into());
    }
    Ok(())
}

fn known_hosts(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(data_dir(app)?.join("known_hosts"))
}

#[tauri::command]
pub fn connections_list(store: State<'_, ConnectionStore>) -> Vec<ConnectionSummary> {
    store.list().into_iter().map(summary).collect()
}

/// Save a new or edited connection. `secrets`: omitted fields stay as stored, "" removes.
#[tauri::command]
pub fn connection_save(
    store: State<'_, ConnectionStore>,
    connection: Connection,
    secrets: Secrets,
) -> Result<ConnectionSummary, String> {
    validate(&connection)?;
    let mut secrets = secrets;
    if !connection.save_password {
        // "Ask every time": never keep the database password.
        secrets.password = Some(String::new());
    }
    let saved = store.upsert(connection)?;
    secrets::apply(&saved.id, &secrets)?;
    Ok(summary(saved))
}

#[tauri::command]
pub fn connection_delete(store: State<'_, ConnectionStore>, id: String) -> Result<(), String> {
    store.delete(&id)?;
    secrets::delete_all(&id)
}

#[tauri::command]
pub fn connection_duplicate(store: State<'_, ConnectionStore>, id: String) -> Result<ConnectionSummary, String> {
    let original = store.get(&id).ok_or("That connection no longer exists.")?;
    let stored = secrets::load(&id)?;
    let mut copy = original.clone();
    copy.id = String::new();
    copy.name = format!("{} copy", original.display_name());
    let saved = store.upsert(copy)?;
    secrets::apply(&saved.id, &stored)?;
    Ok(summary(saved))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedUrl {
    connection: Connection,
    secrets: Secrets,
}

#[tauri::command]
pub fn connection_parse_url(input: String) -> Result<ParsedUrl, String> {
    let (connection, secrets) = url::parse(&input)?;
    Ok(ParsedUrl { connection, secrets })
}

#[tauri::command]
pub fn connection_url(
    store: State<'_, ConnectionStore>,
    id: String,
    include_passwords: bool,
) -> Result<String, String> {
    let conn = store.get(&id).ok_or("That connection no longer exists.")?;
    let stored = if include_passwords { secrets::load(&id)? } else { Secrets::default() };
    Ok(url::to_url(&conn, &stored, include_passwords))
}

/// Write every saved connection as one URL per line.
#[tauri::command]
pub fn connections_export(
    store: State<'_, ConnectionStore>,
    path: String,
    include_passwords: bool,
) -> Result<usize, String> {
    let mut lines = vec![
        "# Tableboat connections – one connection URL per line.".to_string(),
        if include_passwords {
            "# This file CONTAINS PASSWORDS. Keep it private.".to_string()
        } else {
            "# Passwords are not included.".to_string()
        },
    ];
    let all = store.list();
    for conn in &all {
        let stored = if include_passwords { secrets::load(&conn.id)? } else { Secrets::default() };
        lines.push(url::to_url(conn, &stored, include_passwords));
    }
    write_atomic(std::path::Path::new(&path), &(lines.join("\r\n") + "\r\n"))
        .map_err(|e| format!("Could not write {path}: {e}"))?;
    Ok(all.len())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    imported: usize,
    errors: Vec<String>,
}

/// Read a file of connection URLs (one per line, `#` comments allowed) and save each one.
#[tauri::command]
pub fn connections_import(store: State<'_, ConnectionStore>, path: String) -> Result<ImportResult, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {path}: {e}"))?;
    let mut imported = 0;
    let mut errors = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match url::parse(line) {
            Ok((conn, secrets)) => {
                let saved = store.upsert(conn)?;
                secrets::apply(&saved.id, &secrets)?;
                imported += 1;
            }
            Err(e) => errors.push(format!("Line {}: {e}", n + 1)),
        }
    }
    Ok(ImportResult { imported, errors })
}

#[tauri::command]
pub async fn sqlite_create_file(path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || db::sqlite::create_file(&path)).await.map_err(|e| e.to_string())?
}

/// Connect with the given (possibly unsaved) settings, report the server, and disconnect.
#[tauri::command]
pub async fn connection_test(app: AppHandle, connection: Connection, secrets: Secrets) -> Result<ServerInfo, String> {
    validate(&connection)?;
    let merged = secrets::merge_with_stored(&connection.id, &secrets)?;
    let (backend, info, tunnel) = db::open(&connection, &merged, known_hosts(&app)?).await?;
    if let db::Backend::Mysql(b) = backend {
        b.close().await;
    }
    drop(tunnel);
    Ok(info)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    session_id: String,
    connection: Connection,
    server: ServerInfo,
}

/// Open a saved connection for this window. With "ask every time" connections the UI
/// passes the password; if it's missing, the error is exactly "PASSWORD_REQUIRED".
#[tauri::command]
pub async fn session_open(
    app: AppHandle,
    window: Window,
    store: State<'_, ConnectionStore>,
    sessions: State<'_, Sessions>,
    id: String,
    password: Option<String>,
) -> Result<SessionInfo, String> {
    let conn = store.get(&id).ok_or("That connection no longer exists.")?;
    let mut secrets = secrets::load(&id)?;
    if password.is_some() {
        secrets.password = password;
    }
    let needs_password = !conn.save_password && conn.engine != Engine::Sqlite && secrets.password.is_none();
    if needs_password {
        return Err("PASSWORD_REQUIRED".into());
    }
    let (backend, info, tunnel) = db::open(&conn, &secrets, known_hosts(&app)?).await?;
    let session_id = sessions
        .insert(window.label().to_string(), conn.clone(), info.clone(), backend, tunnel)
        .await;
    Ok(SessionInfo { session_id, connection: conn, server: info })
}

#[tauri::command]
pub async fn session_close(sessions: State<'_, Sessions>, session_id: String) -> Result<(), String> {
    sessions.close(&session_id).await;
    Ok(())
}

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Open another Tableboat window in the same process (sharing one WebView2 browser).
/// With `connect`, the new window connects to that saved connection as it opens.
#[tauri::command]
pub fn window_new(app: AppHandle, connect: Option<String>) -> Result<(), String> {
    let n = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("window-{n}");
    let page = match connect {
        // Connection ids are UUIDs; anything else is ignored.
        Some(id) if uuid::Uuid::parse_str(&id).is_ok() => format!("index.html?connect={id}"),
        _ => "index.html".to_string(),
    };
    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(page.into()))
        .title("Tableboat")
        .inner_size(1200.0, 780.0)
        .min_inner_size(720.0, 480.0)
        .decorations(false)
        .visible(false);
    // Cascade from the window that asked, so the new one doesn't sit exactly on top.
    if let Some(main) = app.webview_windows().values().next() {
        if let (Ok(pos), Ok(scale)) = (main.outer_position(), main.scale_factor()) {
            let p = pos.to_logical::<f64>(scale);
            builder = builder.position(p.x + 32.0, p.y + 32.0);
        }
    }
    builder.build().map_err(|e| format!("Could not open a new window: {e}"))?;
    Ok(())
}

// ---------- database objects (sidebar, Ctrl+P) ----------

#[tauri::command]
pub async fn meta_databases(
    sessions: State<'_, Sessions>,
    session_id: String,
    refresh: bool,
) -> Result<Vec<String>, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::databases(&session, refresh).await
}

#[tauri::command]
pub async fn meta_objects(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    refresh: bool,
) -> Result<Vec<db::meta::DbObject>, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::objects(&session, &database, refresh).await
}

#[tauri::command]
pub async fn meta_search(
    sessions: State<'_, Sessions>,
    session_id: String,
    query: String,
    database: Option<String>,
    sort: db::meta::SortBy,
    limit: usize,
) -> Result<Vec<db::meta::Hit>, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::search_session(&session, &query, database.as_deref(), sort, limit.min(5000)).await
}

#[tauri::command]
pub async fn meta_ddl(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    kind: db::meta::ObjectKind,
    name: String,
) -> Result<String, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::ddl(&session, &database, kind, &name).await
}

#[tauri::command]
pub async fn meta_refresh(sessions: State<'_, Sessions>, session_id: String) -> Result<(), String> {
    let session = sessions.get(&session_id).await?;
    db::meta::clear(&session).await;
    Ok(())
}

#[tauri::command]
pub async fn meta_columns(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
) -> Result<std::collections::HashMap<String, Vec<db::meta::ColumnInfo>>, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::columns(&session, &database).await
}

// ---------- running queries ----------

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn query_run(
    window: Window,
    sessions: State<'_, Sessions>,
    runs: State<'_, db::query::Runs>,
    results: State<'_, db::results::ResultStore>,
    session_id: String,
    run_id: String,
    statements: Vec<String>,
    database: Option<String>,
    max_rows: usize,
    use_pool: bool,
    stop_on_error: bool,
) -> Result<Vec<db::query::StatementOutcome>, String> {
    let session = sessions.get(&session_id).await?;
    let opts = db::query::RunOptions { database, max_rows: max_rows.clamp(1, 5_000_000), use_pool, stop_on_error };
    let mut outcomes = db::query::run(&session, &runs, &run_id, statements, opts).await?;
    for o in outcomes.iter_mut() {
        if let Some(rs) = o.result.take() {
            o.result_info = Some(results.insert(window.label(), rs));
        }
    }
    Ok(outcomes)
}

#[tauri::command]
pub async fn query_cancel(runs: State<'_, db::query::Runs>, run_id: String) -> Result<bool, String> {
    Ok(runs.cancel(&run_id).await)
}

#[tauri::command]
pub fn result_rows(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    start: usize,
    count: usize,
) -> Result<Vec<Vec<Option<String>>>, String> {
    let rs = results.get(&result_id)?;
    let rs = rs.read().map_err(|e| e.to_string())?;
    Ok(rs
        .rows(start, count.min(5000))
        .into_iter()
        .map(|row| row.into_iter().map(|c| c.map(str::to_string)).collect())
        .collect())
}

/// Sort by a column (`None` = original order). Returns the number of visible rows.
#[tauri::command]
pub fn result_sort(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    column: Option<usize>,
    descending: bool,
) -> Result<usize, String> {
    let rs = results.get(&result_id)?;
    let mut rs = rs.write().map_err(|e| e.to_string())?;
    rs.set_sort(column.map(|c| (c, descending)));
    Ok(rs.visible_len())
}

/// Quick search: keep rows where any value contains `text`. Returns the visible row count.
#[tauri::command]
pub fn result_filter(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    text: Option<String>,
) -> Result<usize, String> {
    let rs = results.get(&result_id)?;
    let mut rs = rs.write().map_err(|e| e.to_string())?;
    Ok(rs.set_filter(text))
}

#[tauri::command]
pub fn result_copy(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    ranges: Option<Vec<[u32; 2]>>,
    columns: Option<Vec<u32>>,
    format: db::results::Format,
    dialect: db::results::Dialect,
    table: Option<String>,
) -> Result<String, String> {
    let rs = results.get(&result_id)?;
    let rs = rs.read().map_err(|e| e.to_string())?;
    let sel = rs.select(ranges.as_deref(), columns.as_deref());
    db::results::to_text(&rs, &sel, format, dialect, table.as_deref())
}

#[tauri::command]
pub async fn result_export(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    path: String,
    format: db::results::Format,
    dialect: db::results::Dialect,
    table: Option<String>,
) -> Result<usize, String> {
    let rs = results.get(&result_id)?;
    tokio::task::spawn_blocking(move || {
        let rs = rs.read().map_err(|e| e.to_string())?;
        db::results::export_file(&rs, std::path::Path::new(&path), format, dialect, table.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn result_free(results: State<'_, db::results::ResultStore>, result_ids: Vec<String>) {
    results.free(&result_ids);
}

// ---------- editing ----------

#[tauri::command]
pub async fn table_columns(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    table: String,
) -> Result<Vec<db::meta::TableColumn>, String> {
    let session = sessions.get(&session_id).await?;
    db::meta::table_columns(&session, &database, &table).await
}

#[tauri::command]
pub async fn table_preview(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    table: String,
    changes: db::edit::Changes,
) -> Result<Vec<String>, String> {
    let session = sessions.get(&session_id).await?;
    db::edit::preview(&session, &database, &table, &changes).await
}

#[tauri::command]
pub async fn table_save(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    table: String,
    changes: db::edit::Changes,
) -> Result<usize, String> {
    let session = sessions.get(&session_id).await?;
    db::edit::save(&session, &database, &table, &changes).await
}
