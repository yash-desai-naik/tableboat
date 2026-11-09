//! The Claude pane: runs the user's own `claude.exe` (Claude Code), talking stream-JSON over
//! stdin/stdout. No Node.js and no ACP adapter: Rust speaks the same protocol the adapter uses.
//!
//! Each connection is its own Claude Code project: `~/.tableboat/sessions/<connection>/`. So its
//! chats, its CLAUDE.md notes and Claude Code's memory for it stay separate from other
//! connections. Without a connection, chats go to `~/.tableboat/sessions/general/`.
//!
//! The process only exists while the pane is open. Database access comes from Tableboat's
//! own MCP server (see `mcp.rs`); every other permission question is routed to the pane's
//! Allow / Deny buttons through `--permission-prompt-tool`.

pub mod mcp;

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};

/// Where the per-connection project folders live: `~/.tableboat/sessions` (the user's profile,
/// so the app works the same on any PC).
pub fn sessions_base() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    home.unwrap_or_else(|| PathBuf::from(".")).join(".tableboat").join("sessions")
}

/// A folder name from a connection name: letters, digits, '.', '_' and '-' only.
fn folder_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches(|c| c == '-' || c == '.').chars().take(60).collect();
    let out = out.trim_end_matches(['-', '.']).to_string();
    // Reserved for chats without a connection, and names Windows can't use.
    let reserved = ["general", "con", "prn", "aux", "nul", "com1", "lpt1"];
    if out.is_empty() || reserved.contains(&out.as_str()) { format!("{}-db", if out.is_empty() { "connection" } else { &out }) } else { out }
}

static PROJECTS_LOCK: Mutex<()> = Mutex::new(());

/// The project folder for a connection. The name is chosen the first time (from the connection's
/// name) and remembered in `state/claude-projects.json`, so renaming the connection keeps its chats.
fn project_folder(app: &AppHandle, connection_id: Option<&str>, connection_name: Option<&str>) -> PathBuf {
    let base = sessions_base();
    let Some(id) = connection_id.filter(|id| !id.is_empty()) else { return base.join("general") };
    let _guard = PROJECTS_LOCK.lock().unwrap();
    let map_path = crate::paths::data_dir(app).ok().map(|d| d.join("state").join("claude-projects.json"));
    let mut map: std::collections::BTreeMap<String, String> = map_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if let Some(folder) = map.get(id) {
        return base.join(folder);
    }
    let stem = folder_name(connection_name.unwrap_or("connection"));
    let used: HashSet<&String> = map.values().collect();
    let mut folder = stem.clone();
    let mut n = 2;
    while used.contains(&folder) {
        folder = format!("{stem}-{n}");
        n += 1;
    }
    map.insert(id.to_string(), folder.clone());
    if let Some(p) = map_path {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = crate::paths::write_atomic(&p, &serde_json::to_string_pretty(&map).unwrap_or_default());
    }
    base.join(folder)
}

/// A starter CLAUDE.md in a connection's folder (Claude Code reads it every chat): a place for
/// notes about this database that should carry over between chats. Never overwritten.
fn write_starter_notes(folder: &std::path::Path, context: &AiContext) {
    let path = folder.join("CLAUDE.md");
    if path.exists() || folder.file_name().map(|n| n == "general").unwrap_or(false) {
        return;
    }
    let name = context.connection_name.as_deref().unwrap_or("this connection");
    let engine = context.engine.as_deref().unwrap_or("database");
    let text = format!(
        "# {name}\n\nNotes for Claude about the Tableboat connection \"{name}\" ({engine}). Claude Code reads this \
         file at the start of every chat for this connection, so keep what should be remembered here: what the \
         databases and tables are for, naming conventions, columns to be careful with, queries that are often \
         needed.\n\n## Notes\n\n"
    );
    let _ = std::fs::write(path, text);
}

/// What Tableboat tells Claude about the window's current state.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiContext {
    pub session_id: Option<String>,
    /// The saved connection's id: picks the Claude Code project folder.
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub engine: Option<String>,
    pub database: Option<String>,
    pub open_table: Option<String>,
    pub editor_sql: Option<String>,
}

struct Proc {
    child: Child,
    /// Shared so a write never holds the window-state lock across an await.
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
}

#[derive(Default)]
struct WindowState {
    proc: Option<Proc>,
    /// Claude Code's session id, for --resume after a stop.
    claude_session: Option<String>,
    /// Tools the user allowed for the rest of this chat.
    always_allowed: HashSet<String>,
    context: AiContext,
    /// The connection's project folder, where Claude Code runs for this window.
    folder: Option<PathBuf>,
}

pub struct ClaudeHub {
    windows: Mutex<HashMap<String, WindowState>>,
    pub mcp: Mutex<Option<mcp::McpServer>>,
    /// Permission questions waiting for the user: id → answer channel.
    pub approvals: Mutex<HashMap<String, std::sync::mpsc::Sender<(bool, bool)>>>,
}

impl Default for ClaudeHub {
    fn default() -> Self {
        Self { windows: Mutex::new(HashMap::new()), mcp: Mutex::new(None), approvals: Mutex::new(HashMap::new()) }
    }
}

impl ClaudeHub {
    pub fn context(&self, window: &str) -> AiContext {
        self.windows.lock().unwrap().get(window).map(|w| w.context.clone()).unwrap_or_default()
    }

    /// Update what the window shows. Returns true when the window moved to another connection:
    /// its Claude Code process and chat belong to the old connection's project and are dropped.
    pub fn set_context(&self, app: &AppHandle, window: &str, ctx: AiContext) -> bool {
        let unchanged = {
            let windows = self.windows.lock().unwrap();
            windows.get(window).is_some_and(|w| w.folder.is_some() && w.context.connection_id == ctx.connection_id)
        };
        if unchanged {
            self.windows.lock().unwrap().entry(window.to_string()).or_default().context = ctx;
            return false;
        }
        let folder = project_folder(app, ctx.connection_id.as_deref(), ctx.connection_name.as_deref());
        let mut windows = self.windows.lock().unwrap();
        let w = windows.entry(window.to_string()).or_default();
        let moved = w.folder.as_ref().is_some_and(|f| *f != folder);
        if moved {
            // Dropping the process kills it (kill_on_drop); the next message starts a new chat.
            w.proc = None;
            w.claude_session = None;
            w.always_allowed.clear();
        }
        w.folder = Some(folder);
        w.context = ctx;
        moved
    }

    /// The window's project folder (from its current connection).
    pub fn folder(&self, app: &AppHandle, window: &str) -> PathBuf {
        let (known, ctx) = {
            let windows = self.windows.lock().unwrap();
            let w = windows.get(window);
            (w.and_then(|w| w.folder.clone()), w.map(|w| w.context.clone()).unwrap_or_default())
        };
        known.unwrap_or_else(|| project_folder(app, ctx.connection_id.as_deref(), ctx.connection_name.as_deref()))
    }

    pub fn is_always_allowed(&self, window: &str, tool: &str) -> bool {
        self.windows.lock().unwrap().get(window).map(|w| w.always_allowed.contains(tool)).unwrap_or(false)
    }

    pub fn allow_always(&self, window: &str, tool: &str) {
        self.windows.lock().unwrap().entry(window.to_string()).or_default().always_allowed.insert(tool.to_string());
    }
}

/// `claude.exe` on PATH or in the default install folder.
pub fn find_claude() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in ["claude.exe", "claude.cmd"] {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let fallback = PathBuf::from(std::env::var_os("USERPROFILE")?).join(r".local\bin\claude.exe");
    fallback.is_file().then_some(fallback)
}

/// Claude Code keeps a project's sessions in ~/.claude/projects/<path with non-alphanumerics as '-'>.
fn sessions_store(folder: &std::path::Path) -> Option<PathBuf> {
    let encoded: String =
        folder.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(PathBuf::from(std::env::var_os("USERPROFILE")?).join(".claude").join("projects").join(encoded))
}

const SYSTEM_PROMPT: &str = "You are running inside Tableboat, a desktop database client, in a side pane next to the user's \
SQL editor. Use the tableboat MCP tools to look at the user's database: get_context (connection, database, open table, \
current editor SQL), get_schema, and run_read_query (read-only, enforced by the database). execute_sql changes data and \
always asks the user first; prefer giving the SQL in a ```sql code block so the user can insert or run it from the pane. \
Keep answers short and practical.";

async fn spawn(app: &AppHandle, window: &str, resume: Option<String>) -> Result<Proc, String> {
    let claude = find_claude().ok_or(
        "Claude Code (claude.exe) wasn't found. Install it from https://claude.com/claude-code and sign in once in a terminal.",
    )?;
    let hub = app.state::<ClaudeHub>();
    let folder = hub.folder(app, window);
    std::fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    write_starter_notes(&folder, &hub.context(window));

    // Tableboat's MCP server, started on first use.
    let (port, token) = {
        let hub = app.state::<ClaudeHub>();
        let mut guard = hub.mcp.lock().unwrap();
        if guard.is_none() {
            *guard = Some(mcp::McpServer::start(app.clone())?);
        }
        let s = guard.as_ref().unwrap();
        (s.port, s.token.clone())
    };
    let config = json!({
        "mcpServers": {
            "tableboat": {
                "type": "http",
                "url": format!("http://127.0.0.1:{port}/mcp/{window}"),
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    });
    let config_path = crate::paths::data_dir(app)?.join(format!("mcp-{window}.json"));
    std::fs::write(&config_path, config.to_string()).map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&claude);
    cmd.current_dir(&folder)
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
        .arg("--include-partial-messages")
        .arg("--mcp-config")
        .arg(&config_path)
        // Only Tableboat's MCP server: the user's other servers would cost memory and start-up time.
        .arg("--strict-mcp-config")
        .args(["--allowedTools", "mcp__tableboat__get_context mcp__tableboat__get_schema mcp__tableboat__run_read_query"])
        .args(["--permission-prompt-tool", "mcp__tableboat__approve"])
        // Ask for everything not pre-allowed, regardless of the user's own Claude Code default
        // mode (e.g. auto or bypass): the questions come to the pane's Allow / Deny buttons.
        .args(["--permission-mode", "manual"])
        .args(["--append-system-prompt", SYSTEM_PROMPT])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(id) = resume {
        cmd.args(["--resume", &id]);
    }
    let model = app.state::<crate::settings::SettingsStore>().current().claude_model;
    if !model.trim().is_empty() && model.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c)) {
        cmd.args(["--model", model.trim()]);
    }
    #[cfg(windows)]
    {
        // No console window.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start Claude Code: {e}"))?;
    let stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;

    // stdout: one JSON message per line, forwarded to the window as-is.
    let app2 = app.clone();
    let win = window.to_string();
    tauri::async_runtime::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if let Some(id) = value.get("session_id").and_then(Value::as_str) {
                if let Some(w) = app2.state::<ClaudeHub>().windows.lock().unwrap().get_mut(&win) {
                    w.claude_session = Some(id.to_string());
                }
            }
            let _ = app2.emit_to(&win, "claude:event", value);
        }
    });
    // stderr: keep the tail for error messages.
    let app3 = app.clone();
    let win3 = window.to_string();
    tauri::async_runtime::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            tail.push(line);
            if tail.len() > 20 {
                tail.remove(0);
            }
        }
        let _ = app3.emit_to(&win3, "claude:exit", json!({ "stderr": tail.join("\n") }));
    });

    Ok(Proc { child, stdin: Arc::new(tokio::sync::Mutex::new(stdin)) })
}

// ---------- commands ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    installed: bool,
    path: Option<String>,
    running: bool,
    session: Option<String>,
    folder: String,
    /// The model Claude Code uses when Tableboat doesn't pick one (from ~/.claude/settings.json).
    default_model: Option<String>,
}

/// The "model" entry of Claude Code's user settings, if any.
fn claude_default_model() -> Option<String> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let text = std::fs::read_to_string(std::path::Path::new(&home).join(".claude").join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let model = value.get("model")?.as_str()?.trim();
    (!model.is_empty()).then(|| model.to_string())
}

#[tauri::command]
pub fn claude_status(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>) -> ClaudeStatus {
    let path = find_claude();
    let windows = hub.windows.lock().unwrap();
    let w = windows.get(window.label());
    ClaudeStatus {
        installed: path.is_some(),
        path: path.map(|p| p.display().to_string()),
        running: w.map(|w| w.proc.is_some()).unwrap_or(false),
        session: w.and_then(|w| w.claude_session.clone()),
        folder: w.and_then(|w| w.folder.clone()).unwrap_or_else(|| sessions_base().join("general")).display().to_string(),
        default_model: claude_default_model(),
    }
}

/// Send a message, starting Claude Code (resuming this window's chat) if it isn't running.
#[tauri::command]
pub async fn claude_send(
    app: AppHandle,
    window: tauri::Window,
    hub: tauri::State<'_, ClaudeHub>,
    text: String,
    context: AiContext,
) -> Result<(), String> {
    let label = window.label().to_string();
    if hub.set_context(&app, &label, context) {
        let _ = app.emit_to(&label, "claude:reset", ());
    }
    let needs_start = {
        let mut windows = hub.windows.lock().unwrap();
        let w = windows.entry(label.clone()).or_default();
        // A process that exited (crash, sign-out) is replaced.
        if let Some(p) = w.proc.as_mut() {
            if p.child.try_wait().map(|s| s.is_some()).unwrap_or(true) {
                w.proc = None;
            }
        }
        if w.proc.is_none() { Some(w.claude_session.clone()) } else { None }
    };
    if let Some(resume) = needs_start {
        let proc = spawn(&app, &label, resume).await?;
        hub.windows.lock().unwrap().entry(label.clone()).or_default().proc = Some(proc);
    }
    let line = json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": text }] } });
    let stdin = {
        let windows = hub.windows.lock().unwrap();
        windows.get(&label).and_then(|w| w.proc.as_ref()).map(|p| p.stdin.clone()).ok_or("Claude Code stopped.")?
    };
    let mut stdin = stdin.lock().await;
    stdin.write_all(format!("{line}\n").as_bytes()).await.map_err(|e| format!("Could not send to Claude Code: {e}"))?;
    stdin.flush().await.map_err(|e| format!("Could not send to Claude Code: {e}"))
}

/// Stop the running answer (and the process). The next message resumes the same chat.
#[tauri::command]
pub async fn claude_stop(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>) -> Result<(), String> {
    stop_window(&hub, window.label()).await;
    Ok(())
}

pub async fn stop_window(hub: &ClaudeHub, label: &str) {
    let proc = hub.windows.lock().unwrap().get_mut(label).and_then(|w| w.proc.take());
    if let Some(mut p) = proc {
        let _ = p.child.kill().await;
    }
    // Unblock any permission question still waiting.
    hub.approvals.lock().unwrap().clear();
}

/// Start a fresh chat (or switch to a past one with `resume`).
#[tauri::command]
pub async fn claude_new(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>, resume: Option<String>) -> Result<(), String> {
    stop_window(&hub, window.label()).await;
    let mut windows = hub.windows.lock().unwrap();
    let w = windows.entry(window.label().to_string()).or_default();
    w.claude_session = resume;
    w.always_allowed.clear();
    Ok(())
}

/// The window's current connection, database, open table and editor SQL (for get_context).
#[tauri::command]
pub fn claude_set_context(app: AppHandle, window: tauri::Window, hub: tauri::State<'_, ClaudeHub>, context: AiContext) {
    // Another connection in this window: its own project, so the pane starts a fresh chat.
    if hub.set_context(&app, window.label(), context) {
        let _ = app.emit_to(window.label(), "claude:reset", ());
    }
}

#[tauri::command]
pub fn claude_permission_reply(hub: tauri::State<'_, ClaudeHub>, id: String, allow: bool, always: bool) {
    if let Some(tx) = hub.approvals.lock().unwrap().remove(&id) {
        let _ = tx.send((allow, always));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    id: String,
    title: String,
    modified: u64,
}

fn first_user_text(v: &Value) -> Option<String> {
    if v.get("type")?.as_str()? != "user" {
        return None;
    }
    let content = v.get("message")?.get("content")?;
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().find_map(|p| {
            (p.get("type")?.as_str()? == "text").then(|| p.get("text")?.as_str().map(str::to_string)).flatten()
        })?,
        _ => return None,
    };
    let t = text.trim();
    (!t.is_empty()).then(|| t.chars().take(90).collect())
}

/// Past chats of the window's connection (its project folder), newest first.
#[tauri::command]
pub fn claude_sessions(app: AppHandle, window: tauri::Window, hub: tauri::State<'_, ClaudeHub>) -> Vec<SessionEntry> {
    let Some(dir) = sessions_store(&hub.folder(&app, window.label())) else { return vec![] };
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<SessionEntry> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().map(|x| x == "jsonl").unwrap_or(false))
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            let file = std::fs::File::open(&path).ok()?;
            let title = BufReader::new(file)
                .lines()
                .take(200)
                .filter_map(Result::ok)
                .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
                .find_map(|v| first_user_text(&v))?;
            Some(SessionEntry { id: path.file_stem()?.to_string_lossy().into_owned(), title, modified })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(50);
    out
}

/// A past chat's messages (same shape as live events), for showing it before resuming.
#[tauri::command]
pub fn claude_session_messages(
    app: AppHandle,
    window: tauri::Window,
    hub: tauri::State<'_, ClaudeHub>,
    id: String,
) -> Result<Vec<Value>, String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Invalid session id.".into());
    }
    let folder = hub.folder(&app, window.label());
    let path = sessions_store(&folder).ok_or("No profile folder.")?.join(format!("{id}.jsonl"));
    let file = std::fs::File::open(&path).map_err(|e| format!("Could not open the chat: {e}"))?;
    let mut out: Vec<Value> = BufReader::new(file)
        .lines()
        .filter_map(Result::ok)
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter(|v| matches!(v.get("type").and_then(Value::as_str), Some("user") | Some("assistant")))
        .filter(|v| !v.get("isMeta").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    if out.len() > 400 {
        out.drain(..out.len() - 400);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::folder_name;

    #[test]
    fn folder_names_are_safe_and_readable() {
        assert_eq!(folder_name("logs-db (local Mongo)"), "logs-db-local-mongo");
        assert_eq!(folder_name("  Shop prod / EU  "), "shop-prod-eu");
        assert_eq!(folder_name("local"), "local");
        assert_eq!(folder_name("..\\..\\Windows"), "windows");
        assert_eq!(folder_name("général"), "g-n-ral");
        assert_eq!(folder_name("???"), "connection-db");
        assert_eq!(folder_name("General"), "general-db");
        assert_eq!(folder_name("CON"), "con-db");
        assert!(folder_name(&"x".repeat(200)).len() <= 60);
    }
}
