//! `tb ask`: Claude Code with Tableboat's database tools, in a terminal.
//!
//! The same MCP tool server as the app's Claude pane (`tableboat_core::mcp`), with a terminal
//! host: read-only tools run freely, anything else asks "Allow? [y/N/a]" on the console. Runs in
//! the connection's Claude Code project folder, so chats and CLAUDE.md notes are shared with the
//! app's pane for that connection.

use crate::cli::AskArgs;
use crate::cmds::{self, Ctx, OWNER};
use crate::term;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use tableboat_core::claude::{self, AiContext};
use tableboat_core::conn::store::ConnectionStore;
use tableboat_core::db::{self, Sessions};
use tableboat_core::mcp::{Host, McpServer, Resolver};
use tableboat_core::paths;
use tokio::io::AsyncBufReadExt;

struct CliHost {
    context: AiContext,
    sessions: Sessions,
    runs: db::query::Runs,
    store: ConnectionStore,
    runtime: tokio::runtime::Handle,
    linked: Vec<String>,
    linked_sessions: Mutex<HashMap<String, String>>,
    always: Mutex<HashSet<String>>,
    no_input: bool,
    /// One question on the console at a time.
    asking: Mutex<()>,
}

fn tool_title(name: &str) -> String {
    match name.strip_prefix("mcp__tableboat__") {
        Some("get_context") => "Context".into(),
        Some("get_schema") => "Schema".into(),
        Some("run_read_query") => "Read query".into(),
        Some("execute_sql") => "Execute SQL".into(),
        Some("list_connections") => "Connections".into(),
        Some("get_settings") => "Settings".into(),
        Some("update_settings") => "Change settings".into(),
        Some("save_connection") => "Save connection".into(),
        Some("test_connection") => "Test connection".into(),
        Some("delete_connection") => "Delete connection".into(),
        Some("export_connections") => "Export connections".into(),
        Some("import_connections") => "Import connections".into(),
        Some("export_data") => "Export data".into(),
        Some("dump_database") => "Export database".into(),
        Some("open_connection") => "Open connection".into(),
        Some(other) => other.into(),
        None => name.into(),
    }
}

fn input_summary(input: &Value, max: usize) -> String {
    let keys = ["sql", "query", "table", "command", "file_path", "file", "pattern", "name", "connection", "database"];
    let text = keys
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str).map(str::to_string))
        .or_else(|| input.get("changes").map(Value::to_string))
        .unwrap_or_else(|| {
            // Never print passwords (they may be in fields or inside a connection URL).
            let mut shown = input.clone();
            if let Some(o) = shown.as_object_mut() {
                for k in ["password", "ssh_password", "url"] {
                    if o.contains_key(k) {
                        o.insert(k.into(), Value::String("…".into()));
                    }
                }
                if o.is_empty() {
                    return String::new();
                }
            }
            shown.to_string()
        });
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { format!("{}…", flat.chars().take(max).collect::<String>()) } else { flat }
}

impl Host for CliHost {
    fn context(&self) -> AiContext {
        self.context.clone()
    }
    fn sessions(&self) -> &Sessions {
        &self.sessions
    }
    fn runs(&self) -> &db::query::Runs {
        &self.runs
    }
    fn connections(&self) -> &ConnectionStore {
        &self.store
    }
    fn runtime(&self) -> &tokio::runtime::Handle {
        &self.runtime
    }
    fn owner(&self) -> String {
        OWNER.into()
    }
    fn linked(&self) -> Vec<String> {
        self.linked.clone()
    }
    fn linked_session(&self, connection_id: &str) -> Option<String> {
        self.linked_sessions.lock().unwrap().get(connection_id).cloned()
    }
    fn set_linked_session(&self, connection_id: &str, session_id: &str) {
        self.linked_sessions.lock().unwrap().insert(connection_id.into(), session_id.into());
    }
    fn how_to_link(&self) -> &'static str {
        "running tb ask again with --with \"<connection>\""
    }

    fn open_in_app(&self, connection_id: &str) -> Result<String, String> {
        cmds::launch_app(Some(connection_id))?;
        Ok("Opened it in Tableboat (a new window if the app is already running).".into())
    }
    fn files_dir(&self) -> std::path::PathBuf {
        std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir())
    }

    fn ask(&self, tool: &str, input: Value) -> Result<bool, String> {
        if self.always.lock().unwrap().contains(tool) {
            return Ok(true);
        }
        let _one = self.asking.lock().unwrap();
        // App tools send their own question and a readable summary.
        let what = match input.get("question").and_then(Value::as_str) {
            Some(q) => q.to_string(),
            None if tool.ends_with("execute_sql") => "Claude wants to run this SQL (it may change data):".to_string(),
            None => format!("Claude wants to use {}:", tool_title(tool)),
        };
        eprintln!();
        eprintln!("{} {what}", term::clay("?"));
        let detail = match input.get("summary").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => input_summary(&input, 2000),
        };
        for line in detail.lines().take(30) {
            eprintln!("    {line}");
        }
        if self.no_input || !term::has_console() {
            eprintln!("  {}", term::dim("Denied (--no-input or no console to answer in)."));
            return Ok(false);
        }
        let answer = term::ask("  Allow? [y]es / [N]o / [a]lways in this chat: ").unwrap_or_default().to_ascii_lowercase();
        match answer.as_str() {
            "y" | "yes" => Ok(true),
            "a" | "always" => {
                self.always.lock().unwrap().insert(tool.to_string());
                Ok(true)
            }
            _ => {
                eprintln!("  {}", term::dim("Denied."));
                Ok(false)
            }
        }
    }
}

fn question(a: &AskArgs) -> Result<String, String> {
    if !a.question.is_empty() {
        return Ok(a.question.join(" "));
    }
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Err("Ask a question: tb ask -c <connection> \"…\" (or pipe it in).".into());
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
    let s = s.trim().to_string();
    if s.is_empty() { Err("The question is empty.".into()) } else { Ok(s) }
}

pub async fn ask(ctx: &Ctx, a: AskArgs) -> Result<ExitCode, String> {
    let claude_exe = claude::find_claude().ok_or(claude::NOT_INSTALLED)?;
    let text = question(&a)?;
    let conn = a.connection.as_deref().map(|c| ctx.find(c)).transpose()?;
    let linked = a.with.iter().map(|w| ctx.find(w).map(|c| c.id)).collect::<Result<Vec<_>, _>>()?;

    // The connection Claude works on, opened in the host's own session list.
    let sessions = Sessions::default();
    let mut context = AiContext::default();
    if let Some(c) = &conn {
        let (id, _) = cmds::open_in(&sessions, c).await?;
        context = AiContext {
            session_id: Some(id),
            connection_id: Some(c.id.clone()),
            connection_name: Some(c.display_name()),
            engine: Some(c.engine.label().to_string()),
            database: a
                .database
                .clone()
                .or_else(|| Some(c.database.clone()).filter(|d| !d.is_empty()))
                .or_else(|| (c.engine == tableboat_core::conn::model::Engine::Sqlite).then(|| "main".into())),
            open_table: None,
            editor_sql: None,
        };
    }

    let host = Arc::new(CliHost {
        context,
        sessions,
        runs: db::query::Runs::default(),
        store: ConnectionStore::load(&paths::data_dir()?)?,
        runtime: tokio::runtime::Handle::current(),
        linked,
        linked_sessions: Mutex::new(HashMap::new()),
        always: Mutex::new(HashSet::new()),
        no_input: a.no_input,
        asking: Mutex::new(()),
    });
    let h = host.clone();
    let resolver: Resolver = Arc::new(move |route: &str| (route == "tb").then(|| h.clone() as Arc<dyn Host>));
    let server = McpServer::start(resolver)?;

    let folder = claude::project_folder(conn.as_ref().map(|c| c.id.as_str()), conn.as_ref().map(|c| c.display_name()).as_deref());
    claude::prepare_folder(&folder, conn.as_ref().map(|c| c.display_name()).as_deref(), conn.as_ref().map(|c| c.engine.label()))?;
    let config = paths::data_dir()?.join(format!("mcp-tb-{}.json", std::process::id()));
    claude::write_mcp_config(&config, server.port, &server.token, "tb")?;

    let model = a.model.clone().unwrap_or_else(|| ctx.settings.claude_model.clone());
    let mut cmd = tokio::process::Command::new(&claude_exe);
    cmd.current_dir(&folder)
        .args(["-p", &text, "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
        .args(claude::base_args(&config, &claude::cli_prompt(), &model))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if a.resume {
        cmd.arg("--continue");
    }
    if let Some(c) = &conn {
        eprintln!("{}", term::dim(&format!("Claude · {} · {}", c.display_name(), folder.display())));
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start Claude Code: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let stderr = child.stderr.take().ok_or("no stderr")?;
    let err_tail = tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        let mut tail = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            tail.push(l);
            if tail.len() > 15 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut out = std::io::stdout();
    let mut at_line_start = true;
    let mut code = ExitCode::SUCCESS;
    let mut finished = false;
    loop {
        let line = tokio::select! {
            l = lines.next_line() => match l { Ok(Some(l)) => l, _ => break },
            _ = tokio::signal::ctrl_c() => {
                let _ = child.kill().await;
                eprintln!("\n{}", term::dim("Stopped."));
                code = ExitCode::from(130);
                break;
            }
        };
        let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
        match ev.get("type").and_then(Value::as_str) {
            Some("stream_event") => {
                let e = &ev["event"];
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta" {
                    if let Some(t) = e["delta"]["text"].as_str() {
                        let _ = out.write_all(t.as_bytes());
                        let _ = out.flush();
                        at_line_start = t.ends_with('\n');
                    }
                } else if e["type"] == "content_block_stop" && !at_line_start {
                    let _ = writeln!(out);
                    at_line_start = true;
                }
            }
            Some("assistant") => {
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_use" {
                        let name = b["name"].as_str().unwrap_or("");
                        if name == "ToolSearch" {
                            continue;
                        }
                        if !at_line_start {
                            let _ = writeln!(out);
                            at_line_start = true;
                        }
                        let summary = input_summary(&b["input"], 110);
                        eprintln!("{}", term::dim(&format!("● {}{}", tool_title(name), if summary.is_empty() { String::new() } else { format!(": {summary}") })));
                    }
                }
            }
            Some("user") => {
                for b in ev["message"]["content"].as_array().into_iter().flatten() {
                    if b["type"] == "tool_result" && b["is_error"] == true {
                        let text = match &b["content"] {
                            Value::String(s) => s.clone(),
                            Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join(" "),
                            _ => String::new(),
                        };
                        eprintln!("  {} {}", term::red("✕"), text.lines().next().unwrap_or(""));
                    }
                }
            }
            Some("result") => {
                finished = true;
                if !at_line_start {
                    let _ = writeln!(out);
                }
                let secs = ev["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0;
                if ev["is_error"] == true {
                    let msg = ev["result"].as_str().unwrap_or("Claude Code stopped with an error.");
                    eprintln!("{} {msg}", term::red("Error:"));
                    code = ExitCode::FAILURE;
                }
                eprintln!("{}", term::dim(&format!("({secs:.1} s · continue this chat with tb ask --continue -c …)")));
            }
            _ => {}
        }
    }
    let _ = child.wait().await;
    if !finished && code == ExitCode::SUCCESS {
        let tail = err_tail.await.unwrap_or_default();
        eprintln!("{} Claude Code stopped unexpectedly.{}", term::red("Error:"), if tail.is_empty() { String::new() } else { format!("\n{tail}") });
        code = ExitCode::FAILURE;
    }
    let _ = std::fs::remove_file(&config);
    host.sessions.close_window(OWNER).await;
    Ok(code)
}
