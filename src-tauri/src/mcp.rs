//! A small MCP server inside Tableboat, so Claude Code can see the user's database.
//!
//! Streamable-HTTP JSON-RPC on 127.0.0.1 (random port) at `/mcp/<window>`, protected by a
//! random bearer token and an Origin check. Tools:
//!   get_context     – connection, database, open table, SQL in the editor
//!   get_schema      – tables and columns (or one table in detail)
//!   run_read_query  – read-only, enforced by the database (READ ONLY transaction / query_only)
//!   (both take `connection` to read another saved connection the user @-mentioned in the chat;
//!   Tableboat opens a session to it on first use and closes it with the chat or the window)
//!   execute_sql     – anything else; never pre-allowed, so the user always approves it
//!   approve         – Claude Code's permission prompt, answered in the pane (Allow / Deny)
//! and the app tools in `mcp_app.rs`: settings, saved connections, exports, opening the app.

use crate::claude::{AiContext, ASKS_ITSELF, READ_TOOLS};
use crate::conn::store::ConnectionStore;
use crate::db::{self, Backend, SessionHandle, Sessions};
use crate::prefs::{self, Settings};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// Where the tools run: the app (one host per window) or the `tb ask` command line.
pub trait Host: Send + Sync {
    /// What the user has open: connection, database, open table, editor SQL.
    fn context(&self) -> AiContext;
    fn sessions(&self) -> &Sessions;
    fn runs(&self) -> &db::query::Runs;
    fn connections(&self) -> &ConnectionStore;
    /// Runs async database work from the server's request threads.
    fn runtime(&self) -> &tokio::runtime::Handle;
    /// The owner of sessions opened for linked connections (closed with it).
    fn owner(&self) -> String;
    /// Other saved connections the user added (ids): Claude may read them.
    fn linked(&self) -> Vec<String>;
    fn linked_session(&self, connection_id: &str) -> Option<String>;
    fn set_linked_session(&self, connection_id: &str, session_id: &str);
    /// Ask the user to allow a tool call (Allow / Deny in the pane, y/N in a terminal).
    /// "Always" answers are remembered by the host.
    fn ask(&self, tool: &str, input: Value) -> Result<bool, String>;
    /// Where to tell the user to add a connection ("typing @ in the chat input", "--with").
    fn how_to_link(&self) -> &'static str;

    /// The current settings.
    fn settings(&self) -> Result<Settings, String> {
        Ok(prefs::load(&prefs::path()?))
    }
    /// Save changed settings (the app also applies them to its open windows).
    fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        prefs::save(&prefs::path()?, settings)
    }
    /// Saved connections were added, changed or removed (the app refreshes its lists).
    fn connections_changed(&self) {}
    /// Open a saved connection in the app (a new window). Returns what happened.
    fn open_in_app(&self, connection_id: &str) -> Result<String, String>;
    /// Where files given with a relative name are written (the app: Downloads; `tb`: the
    /// current folder).
    fn files_dir(&self) -> PathBuf;
}

/// Finds the host for a request's route (`/mcp/<route>`).
pub type Resolver = Arc<dyn Fn(&str) -> Option<Arc<dyn Host>> + Send + Sync>;

pub struct McpServer {
    pub port: u16,
    pub token: String,
}

const MAX_OUTPUT: usize = 60_000;

impl McpServer {
    pub fn start(resolve: Resolver) -> Result<Self, String> {
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| format!("Could not start the MCP server: {e}"))?;
        let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("MCP server has no port")?;
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let tok = token.clone();
        std::thread::Builder::new()
            .name("tableboat-mcp".into())
            .spawn(move || {
                for request in server.incoming_requests() {
                    let resolve = resolve.clone();
                    let tok = tok.clone();
                    // Each request on its own thread: `approve` waits for the user.
                    std::thread::spawn(move || handle(&resolve, &tok, request));
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

fn handle(resolve: &Resolver, token: &str, mut request: tiny_http::Request) {
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
    let Some(host) = resolve(&window) else {
        respond(request, 404, None);
        return;
    };
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
        Value::Array(batch) => batch.iter().filter_map(|m| rpc(host.as_ref(), m)).collect(),
        single => rpc(host.as_ref(), single).into_iter().collect(),
    };
    match (message.is_array(), replies.len()) {
        (_, 0) => respond(request, 202, None),
        (false, _) => respond(request, 200, Some(replies.into_iter().next().unwrap())),
        (true, _) => respond(request, 200, Some(Value::Array(replies))),
    }
}

/// One JSON-RPC message. Notifications (no id) get no reply.
fn rpc(host: &dyn Host, msg: &Value) -> Option<Value> {
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
        "tools/list" => {
            let mut tools = tool_list();
            if let Value::Array(list) = &mut tools {
                list.extend(crate::mcp_app::tool_list());
            }
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call_tool(host, name, &args) {
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
            "description": "Tables and their columns in a database (default: the current one). With `table`, that table's columns, keys and CREATE statement. With `connection`, read another connection the user added to this chat (see get_context).",
            "inputSchema": { "type": "object", "properties": {
                "connection": { "type": "string", "description": "Name or id of a connection the user added to the chat; default: the window's own connection" },
                "database": { "type": "string" },
                "table": { "type": "string" }
            } }
        },
        {
            "name": "run_read_query",
            "description": "Run ONE read-only statement and get the rows as a Markdown table (default: the window's connection and current database). SQL databases: SELECT, WITH, SHOW, DESCRIBE, EXPLAIN (writes are rejected by the database). MongoDB: mongosh syntax such as db.users.find({ age: { $gt: 30 } }).limit(10) or db.orders.aggregate([...]). With `connection`, run it on another connection the user added to this chat.",
            "inputSchema": { "type": "object", "properties": {
                "sql": { "type": "string" },
                "connection": { "type": "string", "description": "Name or id of a connection the user added to the chat; default: the window's own connection" },
                "database": { "type": "string", "description": "Database to run in; default: the current one (or the connection's default)" },
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

pub(crate) fn session_for(host: &dyn Host) -> Result<(SessionHandle, AiContext), String> {
    let ctx = host.context();
    let id = ctx.session_id.clone().ok_or("Tableboat isn't connected to a database here.")?;
    let session = host.runtime().block_on(host.sessions().get(&id))?;
    Ok((session, ctx))
}

pub fn display_name(c: &crate::conn::model::Connection) -> String {
    if !c.name.trim().is_empty() {
        c.name.trim().to_string()
    } else if let Some(file) = c.file.as_deref().filter(|f| !f.is_empty()) {
        file.rsplit(['\\', '/']).next().unwrap_or(file).to_string()
    } else {
        format!("{}@{}", c.user, c.host)
    }
}

/// The session a read tool should use, and the database to default to: the window's own, or,
/// with `connection`, another saved connection the user @-mentioned in this chat (opened on
/// first use, read-only by the tools that use it).
fn target_session(host: &dyn Host, args: &Value) -> Result<(SessionHandle, Option<String>), String> {
    let wanted = args.get("connection").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    let ctx = host.context();
    let Some(wanted) = wanted else {
        let (session, ctx) = session_for(host)?;
        return Ok((session, ctx.database));
    };
    let store = host.connections();
    let linked: Vec<_> = host.linked().iter().filter_map(|id| store.get(id)).collect();
    let matches = |c: &crate::conn::model::Connection| c.id == wanted || display_name(c).eq_ignore_ascii_case(wanted);
    // The window's own connection, asked for by name.
    if let Some(own) = ctx.connection_id.as_deref().and_then(|id| store.get(id)).filter(|c| matches(c)) {
        let _ = own;
        let (session, ctx) = session_for(host)?;
        return Ok((session, ctx.database));
    }
    let Some(conn) = linked.iter().find(|c| matches(c)).cloned() else {
        let names: Vec<String> = linked.iter().map(|c| format!("\"{}\"", display_name(c))).collect();
        return Err(format!(
            "The user hasn't added a connection named \"{wanted}\" to this chat{}. They can add one by {}.",
            if names.is_empty() { String::new() } else { format!(" (added: {})", names.join(", ")) },
            host.how_to_link()
        ));
    };
    open_saved(host, &conn)
}

/// A session to a saved connection (the window's own, or one opened for this chat and kept
/// until the chat or window closes), and the database to default to.
pub(crate) fn open_saved(host: &dyn Host, conn: &crate::conn::model::Connection) -> Result<(SessionHandle, Option<String>), String> {
    if host.context().connection_id.as_deref() == Some(conn.id.as_str()) {
        if let Ok((session, ctx)) = session_for(host) {
            return Ok((session, ctx.database));
        }
    }
    let default_db = Some(conn.database.clone())
        .filter(|d| !d.is_empty())
        .or_else(|| (conn.engine == crate::conn::model::Engine::Sqlite).then(|| "main".to_string()));
    let sessions = host.sessions();
    let rt = host.runtime();
    if let Some(sid) = host.linked_session(&conn.id) {
        if let Ok(session) = rt.block_on(sessions.get(&sid)) {
            return Ok((session, default_db));
        }
    }
    let secrets = crate::conn::secrets::load(&conn.id)?;
    if !conn.save_password && conn.engine != crate::conn::model::Engine::Sqlite && secrets.password.is_none() {
        return Err(format!(
            "\"{}\" asks for its password every time, so Tableboat can't open it for you. The user can connect to it in Tableboat instead.",
            display_name(conn)
        ));
    }
    let known_hosts = crate::paths::data_dir()?.join("known_hosts");
    let (backend, info, tunnel) = rt
        .block_on(db::open(conn, &secrets, known_hosts))
        .map_err(|e| format!("Could not connect to \"{}\": {e}", display_name(conn)))?;
    let sid = rt.block_on(sessions.insert(host.owner(), conn.clone(), info, backend, tunnel));
    host.set_linked_session(&conn.id, &sid);
    Ok((rt.block_on(sessions.get(&sid))?, default_db))
}

fn call_tool(host: &dyn Host, name: &str, args: &Value) -> Result<String, String> {
    let rt = host.runtime();
    match name {
        "approve" => approve(host, args),
        "get_context" => {
            let ctx = host.context();
            let store = host.connections();
            let linked: Vec<String> = host
                .linked()
                .iter()
                .filter_map(|id| store.get(id))
                .filter(|c| Some(&c.id) != ctx.connection_id.as_ref())
                .map(|c| {
                    let db = if c.database.is_empty() { String::new() } else { format!(", database {}", c.database) };
                    format!("- \"{}\" ({:?}{db}): pass connection: \"{}\" to get_schema / run_read_query\n", display_name(&c), c.engine, display_name(&c))
                })
                .collect();
            let linked_text =
                if linked.is_empty() { String::new() } else { format!("Other connections the user added to this chat (read-only):\n{}", linked.concat()) };
            if ctx.session_id.is_none() {
                return Ok(format!("Not connected to any database.\n{linked_text}"));
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
            out.push_str(&linked_text);
            Ok(out)
        }
        "get_schema" => {
            let (session, default_db) = target_session(host, args)?;
            let database = match args.get("database").and_then(Value::as_str).map(str::to_string).or(default_db) {
                Some(d) => d,
                None => {
                    let list = rt.block_on(db::meta::databases(&session, false)).unwrap_or_default();
                    return Err(format!("No database selected; pass `database`. Databases: {}", list.join(", ")));
                }
            };
            match args.get("table").and_then(Value::as_str) {
                Some(table) => rt.block_on(async {
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
                None if matches!(session.backend, Backend::Mongo(_)) => rt.block_on(async {
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
                None => rt.block_on(async {
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
            let (session, default_db) = target_session(host, args)?;
            // Models sometimes call it `query`: accept both.
            let sql = args.get("sql").or_else(|| args.get("query")).and_then(Value::as_str).ok_or("`sql` is required.")?;
            let max_rows = args.get("max_rows").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
            let database = args.get("database").and_then(Value::as_str).map(str::to_string).or(default_db);
            run_read(host, &session, database.as_deref(), sql, max_rows)
        }
        "execute_sql" => {
            let (session, ctx) = session_for(host)?;
            // Models sometimes call it `query`: accept both.
            let sql = args.get("sql").or_else(|| args.get("query")).and_then(Value::as_str).ok_or("`sql` is required.")?;
            // Tableboat asks the user itself, so this holds even if Claude Code's own permission
            // settings (auto mode, allow rules) would let the call through without a prompt.
            if !host.ask("mcp__tableboat__execute_sql", json!({ "sql": sql }))? {
                return Err("The user denied running this SQL in Tableboat.".into());
            }
            let database = if matches!(session.backend, Backend::Mysql(_) | Backend::Mongo(_)) { ctx.database.clone() } else { None };
            let outcomes = run(host, &session, vec![sql.to_string()], database, 200)?;
            Ok(describe(outcomes))
        }
        other => crate::mcp_app::call(host, other, args).unwrap_or_else(|| Err(format!("Unknown tool {other}"))),
    }
}

fn run(
    host: &dyn Host,
    session: &SessionHandle,
    statements: Vec<String>,
    database: Option<String>,
    max_rows: usize,
) -> Result<Vec<db::query::StatementOutcome>, String> {
    let opts = db::query::RunOptions { database, max_rows, use_pool: true, stop_on_error: false };
    let id = uuid::Uuid::new_v4().to_string();
    host.runtime().block_on(db::query::run(session, host.runs(), &id, statements, opts))
}

pub fn run_read(host: &dyn Host, session: &SessionHandle, database: Option<&str>, sql: &str, max_rows: usize) -> Result<String, String> {
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
        let outcomes = run(host, session, vec![sql.to_string()], database.map(str::to_string), max_rows)?;
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
    let mut outcomes = run(host, session, statements, database, max_rows)?;
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

pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut at {max} characters)", &s[..end])
}

/// Claude Code's permission prompt: the host asks the user (pane or terminal) and waits.
fn approve(host: &dyn Host, args: &Value) -> Result<String, String> {
    let tool = args.get("tool_name").and_then(Value::as_str).unwrap_or("").to_string();
    let input = args.get("input").cloned().unwrap_or(json!({}));
    let allow = |input: Value| json!({ "behavior": "allow", "updatedInput": input }).to_string();

    // Tools that change something ask the user inside the tool itself, so they aren't asked twice.
    if READ_TOOLS.contains(&tool.as_str()) || ASKS_ITSELF.contains(&tool.as_str()) {
        return Ok(allow(input));
    }
    if host.ask(&tool, input.clone())? {
        Ok(allow(input))
    } else {
        Ok(json!({ "behavior": "deny", "message": "The user denied this in Tableboat." }).to_string())
    }
}
