//! The Claude pane: runs the user's own Claude Code (`claude`), talking stream-JSON over
//! stdin/stdout. No Node.js and no ACP adapter: Rust speaks the same protocol the adapter uses.
//!
//! Each connection is its own Claude Code project (see `tableboat_core::claude`).
//!
//! The process only exists while the pane is open. Database access comes from Tableboat's
//! own MCP server (`tableboat_core::mcp`, served for each window by `host.rs`); every other
//! permission question is routed to the pane's Allow / Deny buttons through
//! `--permission-prompt-tool`.

pub mod host;

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};

pub use tableboat_core::claude::{AiContext, sessions_base};
use tableboat_core::claude;

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
    /// Other saved connections the user @-mentioned in this chat: Claude may read them (schema,
    /// read-only queries) through the MCP tools' `connection` argument. Ids.
    linked: HashSet<String>,
    /// Sessions Tableboat opened for linked connections: connection id → session id.
    linked_sessions: HashMap<String, String>,
}

impl WindowState {
    /// Forget the linked connections (new chat or another project); returns their session ids to close.
    fn unlink_all(&mut self) -> Vec<String> {
        self.linked.clear();
        self.linked_sessions.drain().map(|(_, sid)| sid).collect()
    }
}

/// Close sessions opened for linked connections (in the background).
fn close_sessions(app: &AppHandle, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let sessions = app.state::<crate::db::Sessions>();
        for id in ids {
            sessions.close(&id).await;
        }
    });
}

pub struct ClaudeHub {
    windows: Mutex<HashMap<String, WindowState>>,
    pub mcp: Mutex<Option<tableboat_core::mcp::McpServer>>,
    /// Permission questions waiting for the user: id → answer channel.
    pub approvals: Mutex<HashMap<String, std::sync::mpsc::Sender<(bool, bool)>>>,
    /// Claude's UI actions waiting for the window to do them: id → result channel.
    pub ui_calls: Mutex<HashMap<String, std::sync::mpsc::Sender<Result<serde_json::Value, String>>>>,
}

impl Default for ClaudeHub {
    fn default() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            mcp: Mutex::new(None),
            approvals: Mutex::new(HashMap::new()),
            ui_calls: Mutex::new(HashMap::new()),
        }
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
        let folder = claude::project_folder(ctx.connection_id.as_deref(), ctx.connection_name.as_deref());
        let mut windows = self.windows.lock().unwrap();
        let w = windows.entry(window.to_string()).or_default();
        let moved = w.folder.as_ref().is_some_and(|f| *f != folder);
        let mut to_close = Vec::new();
        if moved {
            // Dropping the process kills it (kill_on_drop); the next message starts a new chat.
            w.proc = None;
            w.claude_session = None;
            w.always_allowed.clear();
            to_close = w.unlink_all();
        }
        w.folder = Some(folder);
        w.context = ctx;
        drop(windows);
        close_sessions(app, to_close);
        moved
    }

    /// Let Claude read these saved connections in this window's chat (the user @-mentioned them).
    pub fn link(&self, window: &str, ids: &[String]) {
        let mut windows = self.windows.lock().unwrap();
        let w = windows.entry(window.to_string()).or_default();
        w.linked.extend(ids.iter().filter(|id| !id.is_empty()).cloned());
    }

    /// Connections the user added to this window's chat.
    pub fn linked(&self, window: &str) -> Vec<String> {
        self.windows.lock().unwrap().get(window).map(|w| w.linked.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn linked_session(&self, window: &str, connection_id: &str) -> Option<String> {
        self.windows.lock().unwrap().get(window)?.linked_sessions.get(connection_id).cloned()
    }

    pub fn set_linked_session(&self, window: &str, connection_id: &str, session_id: &str) {
        let mut windows = self.windows.lock().unwrap();
        let w = windows.entry(window.to_string()).or_default();
        w.linked_sessions.insert(connection_id.to_string(), session_id.to_string());
    }

    /// The window's project folder (from its current connection).
    pub fn folder(&self, window: &str) -> PathBuf {
        let (known, ctx) = {
            let windows = self.windows.lock().unwrap();
            let w = windows.get(window);
            (w.and_then(|w| w.folder.clone()), w.map(|w| w.context.clone()).unwrap_or_default())
        };
        known.unwrap_or_else(|| claude::project_folder(ctx.connection_id.as_deref(), ctx.connection_name.as_deref()))
    }

    pub fn is_always_allowed(&self, window: &str, tool: &str) -> bool {
        self.windows.lock().unwrap().get(window).map(|w| w.always_allowed.contains(tool)).unwrap_or(false)
    }

    pub fn allow_always(&self, window: &str, tool: &str) {
        self.windows.lock().unwrap().entry(window.to_string()).or_default().always_allowed.insert(tool.to_string());
    }
}

async fn spawn(app: &AppHandle, window: &str, resume: Option<String>) -> Result<Proc, String> {
    let claude = claude::find_claude().ok_or(claude::NOT_INSTALLED)?;
    let hub = app.state::<ClaudeHub>();
    let folder = hub.folder(window);
    let ctx = hub.context(window);
    claude::prepare_folder(&folder, ctx.connection_name.as_deref(), ctx.engine.as_deref())?;

    // Tableboat's MCP server, started on first use.
    let (port, token) = {
        let hub = app.state::<ClaudeHub>();
        let mut guard = hub.mcp.lock().unwrap();
        if guard.is_none() {
            let resolver = host::resolver(app.clone(), tokio::runtime::Handle::current());
            *guard = Some(tableboat_core::mcp::McpServer::start(resolver)?);
        }
        let s = guard.as_ref().unwrap();
        (s.port, s.token.clone())
    };
    let config_path = crate::paths::data_dir()?.join(format!("mcp-{window}.json"));
    claude::write_mcp_config(&config_path, port, &token, window)?;

    let model = app.state::<crate::settings::SettingsStore>().current().claude_model;
    let mut cmd = Command::new(&claude);
    cmd.current_dir(&folder)
        .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
        .arg("--include-partial-messages")
        .args(claude::base_args(&config_path, &claude::pane_prompt(), &model))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(id) = resume {
        cmd.args(["--resume", &id]);
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

#[tauri::command]
pub fn claude_status(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>) -> ClaudeStatus {
    let path = claude::find_claude();
    let windows = hub.windows.lock().unwrap();
    let w = windows.get(window.label());
    ClaudeStatus {
        installed: path.is_some(),
        path: path.map(|p| p.display().to_string()),
        running: w.map(|w| w.proc.is_some()).unwrap_or(false),
        session: w.and_then(|w| w.claude_session.clone()),
        folder: w.and_then(|w| w.folder.clone()).unwrap_or_else(|| sessions_base().join("general")).display().to_string(),
        default_model: claude::default_model(),
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
    images: Option<Vec<ImageInput>>,
    connections: Option<Vec<String>>,
) -> Result<(), String> {
    let label = window.label().to_string();
    if hub.set_context(&app, &label, context) {
        let _ = app.emit_to(&label, "claude:reset", ());
    }
    hub.link(&label, &connections.unwrap_or_default());
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
    let mut content = vec![json!({ "type": "text", "text": text })];
    for img in images.unwrap_or_default().into_iter().take(20) {
        if !IMAGE_TYPES.contains(&img.media_type.as_str()) {
            return Err(format!("Images must be PNG, JPEG, GIF or WebP (got {}).", img.media_type));
        }
        content.push(json!({ "type": "image", "source": { "type": "base64", "media_type": img.media_type, "data": img.data } }));
    }
    let line = json!({ "type": "user", "message": { "role": "user", "content": content } });
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
pub async fn claude_new(
    app: AppHandle,
    window: tauri::Window,
    hub: tauri::State<'_, ClaudeHub>,
    resume: Option<String>,
) -> Result<(), String> {
    stop_window(&hub, window.label()).await;
    let to_close = {
        let mut windows = hub.windows.lock().unwrap();
        let w = windows.entry(window.label().to_string()).or_default();
        w.claude_session = resume;
        w.always_allowed.clear();
        w.unlink_all()
    };
    close_sessions(&app, to_close);
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

/// The window's answer to one of Claude's UI actions (see `host::GuiHost::ui`).
#[tauri::command]
pub fn ui_reply(hub: tauri::State<'_, ClaudeHub>, id: String, ok: bool, value: Option<serde_json::Value>, error: Option<String>) {
    if let Some(tx) = hub.ui_calls.lock().unwrap().remove(&id) {
        let _ = tx.send(if ok { Ok(value.unwrap_or(serde_json::Value::Null)) } else { Err(error.unwrap_or_else(|| "The window couldn't do that.".into())) });
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
pub fn claude_sessions(window: tauri::Window, hub: tauri::State<'_, ClaudeHub>) -> Vec<SessionEntry> {
    let Some(dir) = claude::sessions_store(&hub.folder(window.label())) else { return vec![] };
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
    window: tauri::Window,
    hub: tauri::State<'_, ClaudeHub>,
    id: String,
) -> Result<Vec<Value>, String> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Invalid session id.".into());
    }
    let folder = hub.folder(window.label());
    let path = claude::sessions_store(&folder).ok_or("No profile folder.")?.join(format!("{id}.jsonl"));
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

// ---------- attachments and voice ----------

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;

/// An image for Claude (base64), from a file, the clipboard or a drop.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInput {
    media_type: String,
    data: String,
}

#[derive(Serialize)]
// Variant names ("text", "image") and the fields inside them ("mediaType") both camelCase.
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum Attachment {
    Text { name: String, path: String, text: String, truncated: bool, size: u64 },
    Image { name: String, path: String, media_type: String, data: String, size: u64 },
}

fn image_type(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Read a file the user picked to attach to a message: images as base64 (up to 5 MB), text up
/// to 256 KB (longer files are cut, and say so). Binary files other than images are refused.
#[tauri::command]
pub fn read_attachment(path: String) -> Result<Attachment, String> {
    use base64::Engine as _;
    let p = std::path::Path::new(&path);
    let meta = std::fs::metadata(p).map_err(|e| format!("Could not read {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file."));
    }
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    if let Some(media_type) = image_type(p) {
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(format!("{name} is larger than 5 MB."));
        }
        let bytes = std::fs::read(p).map_err(|e| format!("Could not read {name}: {e}"))?;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Ok(Attachment::Image { name, path, media_type: media_type.into(), data, size: meta.len() });
    }
    let mut bytes = Vec::new();
    use std::io::Read;
    std::fs::File::open(p)
        .and_then(|f| f.take(MAX_TEXT_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Could not read {name}: {e}"))?;
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    bytes.truncate(MAX_TEXT_BYTES);
    if bytes.contains(&0) {
        return Err(format!("{name} isn't a text file (only text files and PNG/JPEG/GIF/WebP images can be attached)."));
    }
    // Cut at a character boundary if the 256 KB limit split one.
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let bytes = e.into_bytes();
            if !truncated || bytes.len() - valid > 3 {
                return Err(format!("{name} isn't UTF-8 text."));
            }
            String::from_utf8_lossy(&bytes[..valid]).into_owned()
        }
    };
    Ok(Attachment::Text { name, path, text, truncated, size: meta.len() })
}

/// Open Windows voice typing (Win+H) for the focused field: speech goes straight into the chat
/// input. Built into Windows 10/11; no extra memory in Tableboat.
#[tauri::command]
pub fn voice_typing() -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
            VK_H, VK_LWIN,
        };
        let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        let inputs = [
            key(VK_LWIN, KEYBD_EVENT_FLAGS(0)),
            key(VK_H, KEYBD_EVENT_FLAGS(0)),
            key(VK_H, KEYEVENTF_KEYUP),
            key(VK_LWIN, KEYEVENTF_KEYUP),
        ];
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        if sent as usize != inputs.len() {
            return Err("Windows didn't accept the voice typing shortcut. Press Win+H yourself.".into());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Err("Use macOS dictation instead: turn it on in System Settings > Keyboard > Dictation, then press the microphone key twice.".into())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("Voice typing isn't available on this system.".into())
    }
}
