//! A small MCP server inside Tableboat, so Claude Code can see the user's database.
//!
//! Streamable-HTTP JSON-RPC on 127.0.0.1 (random port) at `/mcp/<window>`, protected by a
//! random bearer token and an Origin check. Tools:
//!   get_context     – connection, database, open table, SQL in the editor
//!   get_schema      – tables and columns (or one table in detail)
//!   run_read_query  – read-only, enforced by the database (READ ONLY transaction / query_only)
//!   execute_sql     – anything else; never pre-allowed, so the user always approves it
//!   approve         – Claude Code's permission prompt, answered in the pane (Allow / Deny)

use super::ClaudeHub;
use crate::db::{self, Backend, SessionHandle, Sessions};
use serde_json::{Value, json};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub struct McpServer {
    pub port: u16,
    pub token: String,
}

const READ_TOOLS: [&str; 3] = ["mcp__tableboat__get_context", "mcp__tableboat__get_schema", "mcp__tableboat__run_read_query"];
const MAX_OUTPUT: usize = 60_000;

impl McpServer {
    pub fn start(app: AppHandle) -> Result<Self, String> {
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| format!("Could not start the MCP server: {e}"))?;
        let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("MCP server has no port")?;
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let tok = token.clone();
        std::thread::Builder::new()
            .name("tableboat-mcp".into())
            .spawn(move || {
                for request in server.incoming_requests() {
                    let app = app.clone();
                    let tok = tok.clone();
                    // Each request on its own thread: `approve` waits for the user.
                    std::thread::spawn(move || handle(&app, &tok, request));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { port, token })
    }
}

fn respond(request: tiny_http::Request, status: u16, body: Option<Value>) {
    let mut response = match body {
        Some(v) => tiny_http::Response::from_string(v.to_string()).with_status_code(status).boxed(),
        None => tiny_http::Response::empty(status).boxed(),
    };
    if let Ok(h) = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]) {
        response.add_header(h);
    }
    let _ = request.respond(response);
}

fn header<'a>(request: &'a tiny_http::Request, name: &'static str) -> Option<&'a str> {
    request.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

fn handle(app: &AppHandle, token: &str, mut request: tiny_http::Request) {
    let authorised = header(&request, "Authorization") == Some(&format!("Bearer {token}"));
    // A web page in some browser can reach 127.0.0.1 too; real MCP clients send no Origin.
    let foreign_origin = header(&request, "Origin").map(|o| !o.starts_with("http://127.0.0.1")).unwrap_or(false);
    if !authorised || foreign_origin {
        respond(request, 401, None);
        return;
    }
    let window = request.url().strip_prefix("/mcp/").unwrap_or("").split('?').next().unwrap_or("").to_string();
    if window.is_empty() || !window.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        respond(request, 404, None);
        return;
    }
    if *request.method() != tiny_http::Method::Post {
        respond(request, 405, None);
        return;
    }
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        respond(request, 400, None);
        return;
    }
    let Ok(message) = serde_json::from_str::<Value>(&body) else {
        respond(request, 400, Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})));
        return;
    };
    let replies: Vec<Value> = match &message {
        Value::Array(batch) => batch.iter().filter_map(|m| rpc(app, &window, m)).collect(),
        single => rpc(app, &window, single).into_iter().collect(),
    };
    match (message.is_array(), replies.len()) {
        (_, 0) => respond(request, 202, None),
        (false, _) => respond(request, 200, Some(replies.into_iter().next().unwrap())),
        (true, _) => respond(request, 200, Some(Value::Array(replies))),
    }
}

/// One JSON-RPC message. Notifications (no id) get no reply.
fn rpc(app: &AppHandle, window: &str, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "tableboat", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_tool(app, window, name, &args) {
                Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
                Err(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
            })
        }
        _ => Err(json!({ "code": -32601, "message": format!("Method not found: {method}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn tool_list() -> Value {
    json!([
        {
            "name": "get_context",
            "description": "What the user has open in Tableboat: connection name, database engine, current database, the open table, and the SQL in the active editor tab.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_schema",
            "description": "Tables and their columns in a database (default: the current one). With `table`, that table's columns, keys and CREATE statement.",
            "inputSchema": { "type": "object", "properties": {
                "database": { "type": "string" },
                "table": { "type": "string" }
            } }
        },
        {
            "name": "run_read_query",
            "description": "Run ONE read-only statement in the current database and get the rows as a Markdown table. SQL databases: SELECT, WITH, SHOW, DESCRIBE, EXPLAIN (writes are rejected by the database). MongoDB: mongosh syntax such as db.users.find({ age: { $gt: 30 } }).limit(10) or db.orders.aggregate([...]).",
            "inputSchema": { "type": "object", "properties": {
                "sql": { "type": "string" },
                "max_rows": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 }
            }, "required": ["sql"] }
        },
        {
            "name": "execute_sql",
            "description": "Run SQL that may change data or schema. The user is always asked to approve it first. Prefer showing SQL in a ```sql block instead, so the user can run it themselves.",
            "inputSchema": { "type": "object", "properties": { "sql": { "type": "string" } }, "required": ["sql"] }
        },
        {
            "name": "approve",
            "description": "Permission prompt handler used by Tableboat (not for direct use).",
            "inputSchema": { "type": "object", "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            }, "required": ["tool_name", "input"] }
        }
    ])
}

fn session_for(app: &AppHandle, window: &str) -> Result<(SessionHandle, super::AiContext), String> {
    let ctx = app.state::<ClaudeHub>().context(window);
    let id = ctx.session_id.clone().ok_or("Tableboat isn't connected to a database in this window.")?;
    let sessions = app.state::<Sessions>();
    let session = tauri::async_runtime::block_on(sessions.get(&id))?;
    Ok((session, ctx))
}

fn call_tool(app: &AppHandle, window: &str, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "approve" => approve(app, window, args),
        "get_context" => {
            let ctx = app.state::<ClaudeHub>().context(window);
            if ctx.session_id.is_none() {
                return Ok("Not connected to any database in this Tableboat window.".into());
            }
            let mut out = format!(
                "Connection: {}\nEngine: {}\nDatabase: {}\n",
                ctx.connection_name.as_deref().unwrap_or("?"),
                ctx.engine.as_deref().unwrap_or("?"),
                ctx.database.as_deref().unwrap_or("(none selected)"),
            );
            if let Some(t) = &ctx.open_table {
                out.push_str(&format!("Open table: {t}\n"));
            }
            match ctx.editor_sql.as_deref().filter(|s| !s.trim().is_empty()) {
                Some(sql) => out.push_str(&format!("SQL in the active editor tab:\n```sql\n{}\n```\n", clip(sql, 20_000))),
                None => out.push_str("No SQL editor tab is active.\n"),
            }
            Ok(out)
        }
        "get_schema" => {
            let (session, ctx) = session_for(app, window)?;
            let database = args
                .get("database")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(ctx.database.clone())
                .ok_or("No database selected; pass `database`.")?;
            match args.get("table").and_then(Value::as_str) {
                Some(table) => tauri::async_runtime::block_on(async {
                    let cols = db::meta::table_columns(&session, &database, table).await?;
                    let mut out = format!("Table {database}.{table}\n");
                    for c in &cols {
                        out.push_str(&format!(
                            "- {} {}{}{}{}{}\n",
                            c.name,
                            c.type_name,
                            if c.primary_key { " PRIMARY KEY" } else { "" },
                            if c.auto_increment { " AUTO_INCREMENT" } else { "" },
                            if c.nullable { " NULL" } else { " NOT NULL" },
                            c.default.as_deref().map(|d| format!(" DEFAULT {d}")).unwrap_or_default()
                        ));
                    }
                    if let Ok(ddl) = db::meta::ddl(&session, &database, db::meta::ObjectKind::Table, table).await {
                        out.push_str(&format!("\n```sql\n{ddl}\n```\n"));
                    }
                    Ok(out)
                }),
                None if matches!(session.backend, Backend::Mongo(_)) => tauri::async_runtime::block_on(async {
                    let collections = db::meta::objects(&session, &database, false).await?;
                    let mut out = format!("MongoDB database {database}: {} collections (fields sampled from up to 20 documents)\n", collections.len());
                    for c in collections.iter().take(150) {
                        let fields = db::meta::table_columns(&session, &database, &c.name).await.unwrap_or_default();
                        let list = fields.iter().take(60).map(|f| format!("{} {}", f.name, f.type_name)).collect::<Vec<_>>().join(", ");
                        out.push_str(&format!("{}{}({list})\n", c.name, if c.kind == db::meta::ObjectKind::View { " [view]" } else { "" }));
                        if out.len() > MAX_OUTPUT {
                            break;
                        }
                    }
                    Ok(clip(&out, MAX_OUTPUT))
                }),
                None => tauri::async_runtime::block_on(async {
                    let tables = db::meta::columns(&session, &database).await?;
                    let mut names: Vec<&String> = tables.keys().collect();
                    names.sort();
                    let full: String = names
                        .iter()
                        .map(|t| {
                            let cols = tables[*t].iter().map(|c| format!("{} {}", c.name, c.type_name)).collect::<Vec<_>>().join(", ");
                            format!("{t}({cols})\n")
                        })
                        .collect();
                    if full.len() <= MAX_OUTPUT {
                        Ok(format!("Database {database}: {} tables\n{full}", names.len()))
                    } else {
                        let list: String = names.iter().map(|t| format!("{t} ({} columns)\n", tables[*t].len())).collect();
                        Ok(format!(
                            "Database {database}: {} tables (columns left out: too many; call get_schema with `table` for details)\n{}",
                            names.len(),
                            clip(&list, MAX_OUTPUT)
                        ))
                    }
                }),
            }
        }
        "run_read_query" => {
            let (session, ctx) = session_for(app, window)?;
            let sql = args.get("sql").and_then(Value::as_str).ok_or("`sql` is required.")?;
            let max_rows = args.get("max_rows").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
            run_read(app, &session, ctx.database.as_deref(), sql, max_rows)
        }
        "execute_sql" => {
            let (session, ctx) = session_for(app, window)?;
            let sql = args.get("sql").and_then(Value::as_str).ok_or("`sql` is required.")?;
            // Tableboat asks the user itself, so this holds even if Claude Code's own permission
            // settings (auto mode, allow rules) would let the call through without a prompt.
            if !ask_user(app, window, "mcp__tableboat__execute_sql", json!({ "sql": sql }))? {
                return Err("The user denied running this SQL in Tableboat.".into());
            }
            let database = if matches!(session.backend, Backend::Mysql(_) | Backend::Mongo(_)) { ctx.database.clone() } else { None };
            let outcomes = run(app, &session, vec![sql.to_string()], database, 200)?;
            Ok(describe(outcomes))
        }
        other => Err(format!("Unknown tool {other}")),
    }
}

fn run(
    app: &AppHandle,
    session: &SessionHandle,
    statements: Vec<String>,
    database: Option<String>,
    max_rows: usize,
) -> Result<Vec<db::query::StatementOutcome>, String> {
    let runs = app.state::<db::query::Runs>();
    let opts = db::query::RunOptions { database, max_rows, use_pool: true, stop_on_error: false };
    let id = uuid::Uuid::new_v4().to_string();
    tauri::async_runtime::block_on(db::query::run(session, &runs, &id, statements, opts))
}

fn run_read(app: &AppHandle, session: &SessionHandle, database: Option<&str>, sql: &str, max_rows: usize) -> Result<String, String> {
    let sql = sql.trim().trim_end_matches(';').trim();
    if sql.is_empty() {
        return Err("Empty query.".into());
    }
    if let Backend::Mongo(_) = session.backend {
        use crate::db::mongo_shell::{Command, parse};
        let read_only = match parse(sql)? {
            Command::ShowDatabases | Command::ShowCollections | Command::CollectionNames => true,
            Command::Collection { op, .. } => op.is_read_only(),
            Command::Use(_) | Command::RunCommand { .. } => false,
        };
        if !read_only {
            return Err("run_read_query only runs reads (find, findOne, aggregate without $out/$merge, countDocuments, distinct, getIndexes, show). Use execute_sql for anything else; the user will be asked.".into());
        }
        let outcomes = run(app, session, vec![sql.to_string()], database.map(str::to_string), max_rows)?;
        return Ok(describe(outcomes));
    }
    if sql.contains(';') {
        return Err("Run one statement at a time.".into());
    }
    let first = sql.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let allowed = ["select", "with", "show", "describe", "desc", "explain", "values", "table", "pragma"];
    if !allowed.contains(&first.as_str()) {
        return Err(format!("run_read_query only runs read-only statements, not {}. Use execute_sql (the user will be asked).", first.to_uppercase()));
    }
    // The database itself refuses writes, whatever the statement turns out to do.
    let (statements, database) = match session.backend {
        Backend::Mysql(_) => (
            vec!["START TRANSACTION READ ONLY".to_string(), sql.to_string(), "ROLLBACK".to_string()],
            database.map(str::to_string),
        ),
        Backend::Sqlite(_) => (
            vec!["PRAGMA query_only = ON".to_string(), sql.to_string(), "PRAGMA query_only = OFF".to_string()],
            None,
        ),
        Backend::Mongo(_) => unreachable!("handled above"),
    };
    let mut outcomes = run(app, session, statements, database, max_rows)?;
    if outcomes.len() < 2 {
        return Err(outcomes.first().and_then(|o| o.error.clone()).unwrap_or_else(|| "The query didn't run.".into()));
    }
    Ok(describe(vec![outcomes.swap_remove(1)]))
}

fn describe(outcomes: Vec<db::query::StatementOutcome>) -> String {
    let mut out = String::new();
    for o in outcomes {
        if let Some(e) = &o.error {
            out.push_str(&format!("Error: {e}\n"));
            continue;
        }
        match &o.result {
            Some(rs) => {
                let sel = rs.select(None, None);
                let n = sel.rows.len();
                let table = db::results::to_text(rs, &sel, db::results::Format::Markdown, db::results::Dialect::Mysql, None)
                    .unwrap_or_default();
                out.push_str(&format!(
                    "{n} row{}{} in {:.1} ms\n\n{}\n",
                    if n == 1 { "" } else { "s" },
                    if rs.truncated { " (more rows exist; raise max_rows or add LIMIT)" } else { "" },
                    o.elapsed_ms,
                    clip(&table, MAX_OUTPUT)
                ));
            }
            None => out.push_str(&format!(
                "{} row(s) affected in {:.1} ms{}\n",
                o.affected_rows.unwrap_or(0),
                o.elapsed_ms,
                o.last_insert_id.map(|id| format!(", last insert id {id}")).unwrap_or_default()
            )),
        }
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut at {max} characters)", &s[..end])
}

/// Claude Code's permission prompt: ask the user in the pane and wait for the answer.
fn approve(app: &AppHandle, window: &str, args: &Value) -> Result<String, String> {
    let tool = args.get("tool_name").and_then(Value::as_str).unwrap_or("").to_string();
    let input = args.get("input").cloned().unwrap_or(json!({}));
    let allow = |input: Value| json!({ "behavior": "allow", "updatedInput": input }).to_string();

    // execute_sql asks the user inside the tool itself, so it isn't asked twice here.
    if READ_TOOLS.contains(&tool.as_str()) || tool == "mcp__tableboat__execute_sql" {
        return Ok(allow(input));
    }
    if ask_user(app, window, &tool, input.clone())? {
        Ok(allow(input))
    } else {
        Ok(json!({ "behavior": "deny", "message": "The user denied this in Tableboat." }).to_string())
    }
}

/// Show an Allow / Deny card in the window's Claude pane and wait for the answer.
fn ask_user(app: &AppHandle, window: &str, tool: &str, input: Value) -> Result<bool, String> {
    let hub = app.state::<ClaudeHub>();
    if hub.is_always_allowed(window, tool) {
        return Ok(true);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    hub.approvals.lock().unwrap().insert(id.clone(), tx);
    let _ = app.emit_to(window, "claude:permission", json!({ "id": id, "toolName": tool, "input": input }));
    match rx.recv_timeout(Duration::from_secs(15 * 60)) {
        Ok((allowed, always)) => {
            if allowed && always {
                hub.allow_always(window, tool);
            }
            Ok(allowed)
        }
        Err(_) => {
            hub.approvals.lock().unwrap().remove(&id);
            Ok(false)
        }
    }
}
