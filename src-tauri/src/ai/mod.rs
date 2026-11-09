//! The Claude pane: runs the user's own `claude.exe` (Claude Code) in
//! `D:\tableboat\sessions`, talking stream-JSON over stdin/stdout. No Node.js and no ACP
//! adapter: Rust speaks the same protocol the adapter uses.
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

pub const SESSIONS_DIR: &str = r"D:\tableboat\sessions";

/// What Tableboat tells Claude about the window's current state.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiContext {
    pub session_id: Option<String>,
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

    pub fn set_context(&self, window: &str, ctx: AiContext) {
        self.windows.lock().unwrap().entry(window.to_string()).or_default().context = ctx;
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
fn sessions_store() -> Option<PathBuf> {
    let encoded: String = SESSIONS_DIR.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
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
    std::fs::create_dir_all(SESSIONS_DIR).map_err(|e| format!("Could not create {SESSIONS_DIR}: {e}"))?;

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
    cmd.current_dir(SESSIONS_DIR)
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
        folder: SESSIONS_DIR.into(),
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
    hub.set_context(&label, context);
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
pub fn claude_set_context(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>, context: AiContext) {
    hub.set_context(window.label(), context);
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

/// Past chats started in the sessions folder, newest first.
#[tauri::command]
pub fn claude_sessions() -> Vec<SessionEntry> {
    let Some(dir) = sessions_store() else { return vec![] };
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
pub fn claude_session_messages(id: String) -> Result<Vec<Value>, String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Invalid session id.".into());
    }
    let path = sessions_store().ok_or("No profile folder.")?.join(format!("{id}.jsonl"));
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
