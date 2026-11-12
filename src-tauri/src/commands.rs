//! Commands the UI calls for connections, sessions and windows.

use crate::conn::model::{Connection, ConnectionSummary, Engine, Secrets};
use crate::conn::store::ConnectionStore;
use crate::conn::{secrets, url};
use crate::db::{self, ServerInfo, Sessions};
use serde::Serialize;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, Window};

use crate::conn::ops::{self, known_hosts, validate};

#[tauri::command]
pub fn connections_list(store: State<'_, ConnectionStore>) -> Vec<ConnectionSummary> {
    store.list().into_iter().map(ops::summary).collect()
}

/// Save a new or edited connection. `secrets`: omitted fields stay as stored, "" removes.
#[tauri::command]
pub fn connection_save(
    store: State<'_, ConnectionStore>,
    connection: Connection,
    secrets: Secrets,
) -> Result<ConnectionSummary, String> {
    ops::save(&store, connection, secrets)
}

#[tauri::command]
pub fn connection_delete(store: State<'_, ConnectionStore>, id: String) -> Result<(), String> {
    ops::delete(&store, &id)
}

#[tauri::command]
pub fn connection_duplicate(store: State<'_, ConnectionStore>, id: String) -> Result<ConnectionSummary, String> {
    ops::duplicate(&store, &id)
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
pub fn connection_url(store: State<'_, ConnectionStore>, id: String, include_passwords: bool) -> Result<String, String> {
    ops::to_url(&store, &id, include_passwords)
}

/// Write every saved connection as one URL per line.
#[tauri::command]
pub fn connections_export(store: State<'_, ConnectionStore>, path: String, include_passwords: bool) -> Result<usize, String> {
    ops::export_file(&store, std::path::Path::new(&path), include_passwords)
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
    let (imported, errors) = ops::import_file(&store, std::path::Path::new(&path))?;
    Ok(ImportResult { imported, errors })
}

#[tauri::command]
pub async fn sqlite_create_file(path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || db::sqlite::create_file(&path)).await.map_err(|e| e.to_string())?
}

/// Connect with the given (possibly unsaved) settings, report the server, and disconnect.
#[tauri::command]
pub async fn connection_test(connection: Connection, secrets: Secrets) -> Result<ServerInfo, String> {
    validate(&connection)?;
    let merged = secrets::merge_with_stored(&connection.id, &secrets)?;
    let (backend, info, tunnel) = db::open(&connection, &merged, known_hosts()?).await?;
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
    let (backend, info, tunnel) = db::open(&conn, &secrets, known_hosts()?).await?;
    let session_id = sessions
        .insert(window.label().to_string(), conn.clone(), info.clone(), backend, tunnel)
        .await;
    // Extra windows are reopened with this connection at the next start.
    app.state::<crate::state::WindowList>().set(window.label(), Some(&conn.id));
    Ok(SessionInfo { session_id, connection: conn, server: info })
}

#[tauri::command]
pub async fn session_close(
    app: AppHandle,
    window: Window,
    sessions: State<'_, Sessions>,
    session_id: String,
) -> Result<(), String> {
    sessions.close(&session_id).await;
    app.state::<crate::state::WindowList>().set(window.label(), None);
    Ok(())
}

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Open another Tableboat window in the same process (sharing one WebView2 browser).
/// With `connect`, the new window connects to that saved connection as it opens.
///
/// Async on purpose: a synchronous command runs on the main thread, and building a WebView2
/// window there deadlocks on Windows (the app froze on "New Window").
#[tauri::command]
pub async fn window_new(app: AppHandle, connect: Option<String>) -> Result<(), String> {
    open_window(&app, connect)
}

/// Build a new window (from an async command, or during setup).
pub fn open_window(app: &AppHandle, connect: Option<String>) -> Result<(), String> {
    let n = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
    let label = format!("window-{n}");
    let page = match connect {
        // Connection ids are UUIDs; anything else is ignored.
        Some(id) if uuid::Uuid::parse_str(&id).is_ok() => format!("index.html?connect={id}"),
        _ => "index.html".to_string(),
    };
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(page.into()))
        .title("Tableboat")
        .inner_size(1200.0, 780.0)
        .min_inner_size(720.0, 480.0)
        .decorations(false)
        .visible(false)
        .build()
        .map_err(|e| format!("Could not open a new window: {e}"))?;
    // Same screen as the window that asked.
    let monitor = app.webview_windows().values().find(|w| w.label() != label).and_then(|w| w.current_monitor().ok().flatten());
    place_centered(&window, monitor);
    Ok(())
}

/// The "normal" (not maximized) window rectangle on a screen: 85% of its work area (the screen
/// minus the taskbar), centered, never below the minimum size. Physical pixels: (x, y, w, h).
fn fitted_rect(m: &tauri::Monitor) -> (i32, i32, i32, i32) {
    let area = m.work_area();
    let (aw, ah) = (area.size.width as f64, area.size.height as f64);
    let scale = m.scale_factor();
    let w = (aw * 0.85).max(720.0 * scale).min(aw).round();
    let h = (ah * 0.85).max(480.0 * scale).min(ah).round();
    let x = area.position.x as f64 + ((aw - w) / 2.0).round();
    let y = area.position.y as f64 + ((ah - h) / 2.0).round();
    (x as i32, y as i32, w as i32, h as i32)
}

fn monitor_of(window: &tauri::WebviewWindow, monitor: Option<tauri::Monitor>) -> Option<tauri::Monitor> {
    monitor
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
}

/// Size a (still hidden) window to fit its screen, centered. The page then maximizes and shows
/// it (main.tsx) and calls `window_fit_restore`, so "Restore Down" returns to this rectangle.
pub fn place_centered(window: &tauri::WebviewWindow, monitor: Option<tauri::Monitor>) {
    if let Some(m) = monitor_of(window, monitor) {
        let (x, y, w, h) = fitted_rect(&m);
        let _ = window.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// Make "Restore Down" return to the fitted, centered rectangle. A window maximized while still
/// hidden (to avoid a flash on start) never gets a proper restore position from Windows, so set
/// it directly: SetWindowPlacement changes only the restore rectangle, not the maximized state.
#[tauri::command]
pub fn window_fit_restore(window: tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, SetWindowPlacement, WINDOWPLACEMENT};
        let Some(m) = monitor_of(&window, None) else { return };
        let Ok(hwnd) = window.hwnd() else { return };
        let (x, y, w, h) = fitted_rect(&m);
        // The restore rectangle is in "workspace" coordinates: screen coordinates shifted by the
        // primary screen's taskbar when that sits at the top or left.
        let (ox, oy) = window
            .primary_monitor()
            .ok()
            .flatten()
            .map(|p| (p.work_area().position.x - p.position().x, p.work_area().position.y - p.position().y))
            .unwrap_or((0, 0));
        let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        unsafe {
            if GetWindowPlacement(hwnd, &mut wp).is_ok() {
                wp.rcNormalPosition.left = x - ox;
                wp.rcNormalPosition.top = y - oy;
                wp.rcNormalPosition.right = x - ox + w;
                wp.rcNormalPosition.bottom = y - oy + h;
                let _ = SetWindowPlacement(hwnd, &wp);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
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

/// MongoDB results: documents `start..start+count` of the current view (canonical Extended JSON).
#[tauri::command]
pub fn result_documents(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    start: usize,
    count: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let rs = results.get(&result_id)?;
    let rs = rs.read().map_err(|e| e.to_string())?;
    Ok(rs.documents(start, count.min(1000)))
}

/// One document as text: "shell" (mongosh syntax, also what the editor edits), "json"
/// (relaxed Extended JSON) or "new" (mongosh syntax without the _id, for a duplicate).
#[tauri::command]
pub fn result_document_text(
    results: State<'_, db::results::ResultStore>,
    result_id: String,
    index: usize,
    format: String,
) -> Result<String, String> {
    let rs = results.get(&result_id)?;
    let rs = rs.read().map_err(|e| e.to_string())?;
    let mut d = rs.document(index).ok_or("That document is no longer in the results.")?;
    Ok(match format.as_str() {
        "json" => db::mongo_doc::relaxed_json(&d, true),
        // A copy to insert as a new document: without its _id.
        "new" => {
            d.remove("_id");
            db::mongo_doc::shell_doc(&d, true)
        }
        _ => db::mongo_doc::shell_doc(&d, true),
    })
}

fn mongo(session: &db::SessionHandle) -> Result<&db::mongo::MongoBackend, String> {
    match &session.backend {
        db::Backend::Mongo(b) => Ok(b),
        _ => Err("Documents can only be edited on MongoDB connections.".into()),
    }
}

/// The document view's editor: replace one document (`id` = its `_id` as canonical Extended JSON).
#[tauri::command]
pub async fn document_replace(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    collection: String,
    id: serde_json::Value,
    text: String,
) -> Result<(), String> {
    let session = sessions.get(&session_id).await?;
    let id = db::mongo::id_from_json(&id)?;
    db::mongo::replace_document(mongo(&session)?, &database, &collection, &id, &text).await
}

#[tauri::command]
pub async fn document_delete(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    collection: String,
    id: serde_json::Value,
) -> Result<(), String> {
    let session = sessions.get(&session_id).await?;
    let id = db::mongo::id_from_json(&id)?;
    db::mongo::delete_document(mongo(&session)?, &database, &collection, &id).await
}

/// Insert a document typed as text; returns the new `_id`.
#[tauri::command]
pub async fn document_insert(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    collection: String,
    text: String,
) -> Result<String, String> {
    let session = sessions.get(&session_id).await?;
    db::mongo::insert_document(mongo(&session)?, &database, &collection, &text).await
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
