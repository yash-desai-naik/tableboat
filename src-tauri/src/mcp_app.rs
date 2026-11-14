//! The app tools of Tableboat's MCP server: settings, saved connections, exports and opening
//! the app. Read tools run freely; every tool that changes something (or writes a file) asks the
//! user itself with a readable summary (a card in the Claude pane, a question in `tb ask`).
//! Tool results never contain passwords.

use crate::conn::model::{Connection, Engine, Secrets, SslMode};
use crate::conn::{ops, secrets, url};
use crate::db::{self, export};
use crate::mcp::{Host, clip, open_saved, session_for, target_session};
use crate::prefs;
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn tool_list() -> Vec<Value> {
    let conn_arg = json!({ "type": "string", "description": "Saved connection: name, id, or a part of the name only one has" });
    vec![
        json!({
            "name": "list_connections",
            "description": "The user's saved connections in Tableboat: name, engine, host or file, user, default database, tag, colour, SSH, whether a password is saved. Never includes passwords.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_settings",
            "description": "Tableboat's current settings and the values each one accepts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "update_settings",
            "description": "Change Tableboat settings, e.g. {\"theme\": \"dark\"} or {\"editorFontSize\": 15, \"density\": \"compact\"}. The app applies them at once. The user is asked first. See get_settings for the keys and allowed values.",
            "inputSchema": { "type": "object", "properties": {
                "changes": { "type": "object", "description": "Setting keys (camelCase, as get_settings shows) and their new values" }
            }, "required": ["changes"] }
        }),
        json!({
            "name": "save_connection",
            "description": "Create a saved connection, or change one (pass `connection`). Give a `url` (mysql://user:pass@host:3306/db, mariadb://…, sqlite:///path/file.db, mongodb://… or mongodb+srv://…, mysql+ssh://sshuser@bastion/user:pass@dbhost:3306/db) and/or the separate fields; fields override the URL. Passwords are stored in the system keychain. The user is asked first.",
            "inputSchema": { "type": "object", "properties": {
                "connection": { "type": "string", "description": "Existing connection to change (name or id); leave out to create a new one" },
                "url": { "type": "string" },
                "name": { "type": "string" },
                "engine": { "type": "string", "enum": ["mysql", "mariadb", "sqlite", "mongodb"] },
                "host": { "type": "string" },
                "port": { "type": "integer" },
                "user": { "type": "string" },
                "password": { "type": "string", "description": "\"\" removes the saved password" },
                "database": { "type": "string", "description": "Default database" },
                "file": { "type": "string", "description": "SQLite database file" },
                "color": { "type": "string", "description": "#rrggbb" },
                "tag": { "type": "string", "description": "e.g. local, development, staging, production (production makes the app careful with writes); \"\" removes it" },
                "ssl_mode": { "type": "string", "enum": ["disabled", "preferred", "required", "verify-ca", "verify-identity"] },
                "ask_password_every_time": { "type": "boolean" },
                "ssh_host": { "type": "string", "description": "Connect through this SSH server; \"\" turns SSH off" },
                "ssh_port": { "type": "integer" },
                "ssh_user": { "type": "string" },
                "ssh_password": { "type": "string" },
                "ssh_key_file": { "type": "string", "description": "Private key file (uses key authentication)" },
                "test": { "type": "boolean", "description": "Connect first and show the result in the question to the user", "default": true }
            } }
        }),
        json!({
            "name": "test_connection",
            "description": "Try to connect to a saved connection, or to a connection URL, and report the server and version.",
            "inputSchema": { "type": "object", "properties": {
                "connection": conn_arg,
                "url": { "type": "string" }
            } }
        }),
        json!({
            "name": "delete_connection",
            "description": "Delete a saved connection and its saved passwords. The user is asked first.",
            "inputSchema": { "type": "object", "properties": { "connection": conn_arg }, "required": ["connection"] }
        }),
        json!({
            "name": "export_connections",
            "description": "Write saved connections to a file, one connection URL per line (Tableboat's import format; also `tb conn import`). The user is asked first.",
            "inputSchema": { "type": "object", "properties": {
                "file": { "type": "string", "description": "File path; a bare name goes in the default folder" },
                "connections": { "type": "array", "items": { "type": "string" }, "description": "Which ones (names or ids); default: all" },
                "with_passwords": { "type": "boolean", "default": false }
            }, "required": ["file"] }
        }),
        json!({
            "name": "import_connections",
            "description": "Add the connections in a file of connection URLs (one per line, # comments). The user is asked first.",
            "inputSchema": { "type": "object", "properties": { "file": { "type": "string" } }, "required": ["file"] }
        }),
        json!({
            "name": "export_data",
            "description": "Export a table or collection, or the result of a query, to a file. The format follows the file extension: .csv, .tsv, .json, .xlsx, .md, .sql (INSERT statements). Default connection: the window's own. The user is asked first.",
            "inputSchema": { "type": "object", "properties": {
                "connection": conn_arg,
                "database": { "type": "string" },
                "table": { "type": "string", "description": "Table or collection (or give `query`)" },
                "query": { "type": "string", "description": "SQL, or mongosh for MongoDB" },
                "file": { "type": "string", "description": "File path; a bare name goes in the default folder" },
                "limit": { "type": "integer", "description": "At most this many rows (default: all)" }
            }, "required": ["file"] }
        }),
        json!({
            "name": "dump_database",
            "description": "Export a whole database: MySQL/MariaDB and SQLite to one .sql script (DROP + CREATE + INSERT for each table, then views, routines and triggers, or indexes for SQLite); MongoDB to a folder with one JSON file per collection. Default connection: the window's own; default database: the current one. The user is asked first.",
            "inputSchema": { "type": "object", "properties": {
                "connection": conn_arg,
                "database": { "type": "string" },
                "file": { "type": "string", "description": ".sql file (MongoDB: a folder); a bare name goes in the default folder" },
                "tables": { "type": "array", "items": { "type": "string" }, "description": "Only these tables / collections" },
                "data": { "type": "boolean", "default": true, "description": "false: structure only" }
            }, "required": ["file"] }
        }),
        json!({
            "name": "duplicate_connection",
            "description": "Copy a saved connection (with its saved passwords) as \"<name> copy\". The user is asked first.",
            "inputSchema": { "type": "object", "properties": { "connection": conn_arg }, "required": ["connection"] }
        }),
        json!({
            "name": "list_databases",
            "description": "The databases (schemas) on the window's connection, or on a connection the user added to this chat.",
            "inputSchema": { "type": "object", "properties": {
                "connection": { "type": "string", "description": "A connection the user added to the chat; default: the window's own" }
            } }
        }),
        json!({
            "name": "list_objects",
            "description": "Tables, views, functions, procedures and triggers in a database (MongoDB: collections and views), with estimated row counts where the server keeps them.",
            "inputSchema": { "type": "object", "properties": {
                "connection": { "type": "string", "description": "A connection the user added to the chat; default: the window's own" },
                "database": { "type": "string", "description": "Default: the current one" },
                "kind": { "type": "string", "enum": ["table", "view", "function", "procedure", "trigger"] }
            } }
        }),
        json!({
            "name": "get_definition",
            "description": "The CREATE statement of a table, view, function, procedure or trigger (MongoDB: a collection's options, validator and indexes).",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string" },
                "kind": { "type": "string", "enum": ["table", "view", "function", "procedure", "trigger"], "description": "Default: found by name" },
                "connection": { "type": "string", "description": "A connection the user added to the chat; default: the window's own" },
                "database": { "type": "string" }
            }, "required": ["name"] }
        }),
        json!({
            "name": "get_app_state",
            "description": "What the Tableboat window shows right now: its connection and database, the open tabs (tables, SQL editors) and which is active, unsaved table edits, and the last results in the active tab.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list_app_commands",
            "description": "Every command the window can run now (the command palette): themes, density, zoom, tabs, run/format/cancel SQL, refresh, save/discard/preview table edits, add/duplicate/delete rows, disconnect, settings, panes... with their ids and on/off state.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "run_app_command",
            "description": "Run one command from list_app_commands, as if the user picked it in the command palette. Commands that can change data or lose work (running SQL, saving or discarding edits, closing tabs, disconnecting...) ask the user first.",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }
        }),
        json!({
            "name": "open_object",
            "description": "Open a table, view, collection, function, procedure or trigger in a tab of the window (data, or the definition for routines), as a click in the sidebar does.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string" },
                "database": { "type": "string", "description": "Default: the window's current database" },
                "kind": { "type": "string", "enum": ["table", "view", "function", "procedure", "trigger"] }
            }, "required": ["name"] }
        }),
        json!({
            "name": "open_sql",
            "description": "Put SQL (or mongosh) into the window's editor: a new SQL tab (default) or inserted into the active editor at the cursor. With run: true it runs there too, showing the results in the window (the user is asked first).",
            "inputSchema": { "type": "object", "properties": {
                "sql": { "type": "string" },
                "mode": { "type": "string", "enum": ["new_tab", "insert"], "default": "new_tab" },
                "run": { "type": "boolean", "default": false }
            }, "required": ["sql"] }
        }),
        json!({
            "name": "use_database",
            "description": "Switch the window to another database on its connection (the sidebar and new queries follow).",
            "inputSchema": { "type": "object", "properties": { "database": { "type": "string" } }, "required": ["database"] }
        }),
        json!({
            "name": "check_for_updates",
            "description": "Ask GitHub whether a newer version of Tableboat is out: the version running now, the newest one and its release notes. If there is one, the app also shows its update notice.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "install_update",
            "description": "Install the newer Tableboat version found by check_for_updates: Tableboat saves its tabs, closes, installs and opens again (this chat stops; it can be continued from History). The user is asked first.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "open_connection",
            "description": "Open a saved connection in a new Tableboat window.",
            "inputSchema": { "type": "object", "properties": { "connection": conn_arg }, "required": ["connection"] }
        }),
    ]
}

/// `None` when `name` isn't an app tool.
pub fn call(host: &dyn Host, name: &str, args: &Value) -> Option<Result<String, String>> {
    Some(match name {
        "list_connections" => list_connections(host),
        "get_settings" => get_settings(host),
        "update_settings" => update_settings(host, args),
        "save_connection" => save_connection(host, args),
        "test_connection" => test_connection(host, args),
        "delete_connection" => delete_connection(host, args),
        "export_connections" => export_connections(host, args),
        "import_connections" => import_connections(host, args),
        "export_data" => export_data(host, args),
        "dump_database" => dump_database(host, args),
        "open_connection" => open_connection(host, args),
        "check_for_updates" => check_for_updates(host),
        "duplicate_connection" => duplicate_connection(host, args),
        "list_databases" => list_databases(host, args),
        "list_objects" => list_objects(host, args),
        "get_definition" => get_definition(host, args),
        "get_app_state" => host.ui("state", json!({})).map(|v| serde_json::to_string_pretty(&v).unwrap_or_default()),
        "list_app_commands" => list_app_commands(host),
        "run_app_command" => run_app_command(host, args),
        "open_object" => open_object(host, args),
        "open_sql" => open_sql(host, args),
        "use_database" => use_database(host, args),
        "install_update" => install_update(host),
        _ => return None,
    })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    str_arg(args, key).map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| format!("`{key}` is required."))
}

/// Ask the user; `Err` when they say no.
fn confirm(host: &dyn Host, tool: &str, question: &str, summary: String) -> Result<(), String> {
    let input = json!({ "question": question, "summary": summary });
    if host.ask(&format!("mcp__tableboat__{tool}"), input)? { Ok(()) } else { Err("The user said no in Tableboat.".into()) }
}

fn find(host: &dyn Host, query: &str) -> Result<Connection, String> {
    ops::find(host.connections(), query)
}

/// The connection a data tool works on: the one named, else the window's own.
fn connection_or_current(host: &dyn Host, args: &Value) -> Result<Connection, String> {
    match str_arg(args, "connection").map(str::trim).filter(|s| !s.is_empty()) {
        Some(q) => find(host, q),
        None => {
            let id = host.context().connection_id.ok_or("Not connected here: pass `connection` (see list_connections).")?;
            host.connections().get(&id).ok_or_else(|| "The current connection is no longer saved.".to_string())
        }
    }
}

/// "root@127.0.0.1:3306/shop", a SQLite file, or a MongoDB host.
fn target(c: &Connection) -> String {
    match c.engine {
        Engine::Sqlite => c.file.clone().unwrap_or_default(),
        _ => {
            let user = if c.user.is_empty() { String::new() } else { format!("{}@", c.user) };
            let db = if c.database.is_empty() { String::new() } else { format!("/{}", c.database) };
            format!("{user}{}:{}{db}", c.host, c.effective_port())
        }
    }
}

fn describe_connection(c: &Connection, has_password: bool) -> String {
    let mut parts = vec![format!("{} {}", c.engine.label(), target(c))];
    if let Some(tag) = c.tag.as_deref().filter(|t| !t.is_empty()) {
        parts.push(format!("tag {tag}"));
    }
    parts.push(format!("colour {}", c.color));
    if c.ssh.enabled {
        parts.push(format!("via SSH {}@{}:{}", c.ssh.user, c.ssh.host, c.ssh.port));
    }
    if c.engine != Engine::Sqlite {
        parts.push(
            if !c.save_password {
                "password asked every time"
            } else if has_password {
                "password saved"
            } else {
                "no password"
            }
            .into(),
        );
    }
    parts.join(", ")
}

/// A file path from the user: `~\…` is the home folder; a bare or relative name goes in the
/// host's default folder.
fn resolve_path(host: &dyn Host, file: &str) -> Result<PathBuf, String> {
    let f = file.trim().trim_matches('"');
    if f.is_empty() {
        return Err("`file` is required.".into());
    }
    let p = match f.strip_prefix("~/").or_else(|| f.strip_prefix("~\\")) {
        Some(rest) => dirs::home_dir().ok_or("No home folder")?.join(rest),
        None => PathBuf::from(f),
    };
    let p = if p.is_absolute() { p } else { host.files_dir().join(p) };
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(format!("The folder {} doesn't exist.", parent.display()));
        }
    }
    Ok(p)
}

fn replaces(path: &std::path::Path) -> &'static str {
    if path.is_dir() {
        " (files in this folder are replaced)"
    } else if path.exists() {
        " (replaces the existing file)"
    } else {
        ""
    }
}

// ---------- settings ----------

fn get_settings(host: &dyn Host) -> Result<String, String> {
    let s = host.settings()?;
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
    Ok(format!("Current settings:\n{text}\n\nAllowed values:\n{}", prefs::HELP))
}

fn update_settings(host: &dyn Host, args: &Value) -> Result<String, String> {
    let changes = args.get("changes").filter(|v| v.is_object()).ok_or("`changes` must be an object such as {\"theme\": \"dark\"}.")?;
    let current = host.settings()?;
    let (next, diff) = current.checked_change(changes)?;
    if diff.is_empty() {
        return Ok("Nothing to change: those settings already have these values.".into());
    }
    let summary = diff.iter().map(|(k, old, new)| format!("{k}: {old} → {new}")).collect::<Vec<_>>().join("\n");
    confirm(host, "update_settings", "Change Tableboat's settings?", summary.clone())?;
    host.save_settings(&next)?;
    Ok(format!("Settings changed:\n{summary}"))
}

// ---------- connections ----------

fn list_connections(host: &dyn Host) -> Result<String, String> {
    let all = host.connections().list();
    if all.is_empty() {
        return Ok("No saved connections.".into());
    }
    let current = host.context().connection_id;
    let mut out = format!("{} saved connection{}:\n", all.len(), if all.len() == 1 { "" } else { "s" });
    for c in all {
        let has = secrets::get(&c.id, secrets::PASSWORD).ok().flatten().is_some();
        let here = if current.as_deref() == Some(c.id.as_str()) { " (open here)" } else { "" };
        out.push_str(&format!("- \"{}\"{here}: {}\n", c.display_name(), describe_connection(&c, has)));
    }
    Ok(out)
}

fn parse_engine(s: &str) -> Result<Engine, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "mysql" => Ok(Engine::Mysql),
        "mariadb" => Ok(Engine::Mariadb),
        "sqlite" => Ok(Engine::Sqlite),
        "mongodb" | "mongo" => Ok(Engine::Mongodb),
        other => Err(format!("Tableboat works with mysql, mariadb, sqlite and mongodb, not \"{other}\".")),
    }
}

fn parse_ssl(s: &str) -> Result<SslMode, String> {
    serde_json::from_value(json!(s.trim().to_ascii_lowercase()))
        .map_err(|_| "ssl_mode is one of disabled, preferred, required, verify-ca, verify-identity.".to_string())
}

/// Connect once and close again; returns "MySQL 8.0.42" or the error.
fn try_connect(host: &dyn Host, conn: &Connection, secrets: &Secrets) -> Result<String, String> {
    let known_hosts = ops::known_hosts()?;
    host.runtime().block_on(async {
        let (backend, info, _tunnel) = db::open(conn, secrets, known_hosts).await?;
        backend.close().await;
        Ok(format!("{} {}", info.product, info.version).trim().to_string())
    })
}

/// The fields that differ between two versions of a connection, for the question to the user.
fn changes(before: &Connection, after: &Connection) -> Vec<String> {
    let (Ok(Value::Object(a)), Ok(Value::Object(b))) = (serde_json::to_value(before), serde_json::to_value(after)) else {
        return vec![];
    };
    let mut out = Vec::new();
    for (k, new) in &b {
        if matches!(k.as_str(), "id" | "createdAt" | "updatedAt") {
            continue;
        }
        let old = a.get(k).unwrap_or(&Value::Null);
        if old != new {
            match (old, new) {
                (Value::Object(o), Value::Object(n)) => {
                    for (sk, sv) in n {
                        let so = o.get(sk).unwrap_or(&Value::Null);
                        if so != sv {
                            out.push(format!("{k}.{sk}: {so} → {sv}"));
                        }
                    }
                }
                _ => out.push(format!("{k}: {old} → {new}")),
            }
        }
    }
    out
}

fn save_connection(host: &dyn Host, args: &Value) -> Result<String, String> {
    let store = host.connections();
    let existing = str_arg(args, "connection").map(str::trim).filter(|s| !s.is_empty()).map(|q| find(host, q)).transpose()?;
    let mut conn = existing.clone().unwrap_or_default();
    // `None` = keep what's stored; `Some("")` = remove.
    let mut new_secrets = Secrets::default();

    if let Some(u) = str_arg(args, "url").map(str::trim).filter(|s| !s.is_empty()) {
        let (parsed, s) = url::parse(u)?;
        conn = match &existing {
            // Editing: the URL replaces where and how to connect; name, colour and tag stay
            // unless the URL sets them.
            Some(old) => Connection {
                id: old.id.clone(),
                name: if parsed.name.is_empty() { old.name.clone() } else { parsed.name.clone() },
                color: if u.contains("color=") { parsed.color.clone() } else { old.color.clone() },
                tag: if u.contains("tag=") { parsed.tag.clone() } else { old.tag.clone() },
                created_at: old.created_at,
                ..parsed
            },
            None => parsed,
        };
        new_secrets = s;
    }
    let s = |k: &str| str_arg(args, k).map(str::trim);
    if let Some(v) = s("engine") {
        conn.engine = parse_engine(v)?;
    }
    if let Some(v) = s("name") {
        conn.name = v.to_string();
    }
    if let Some(v) = s("host") {
        conn.host = v.to_string();
    }
    if let Some(v) = args.get("port") {
        conn.port = match v.as_u64() {
            Some(p) if (1..=65535).contains(&p) => Some(p as u16),
            _ if v.is_null() => None,
            _ => return Err("`port` must be 1 to 65535.".into()),
        };
    }
    if let Some(v) = s("user") {
        conn.user = v.to_string();
    }
    if let Some(v) = str_arg(args, "password") {
        new_secrets.password = Some(v.to_string());
    }
    if let Some(v) = s("database") {
        conn.database = v.to_string();
    }
    if let Some(v) = s("file") {
        conn.file = Some(v.trim_matches('"').to_string());
        if s("engine").is_none() && s("url").is_none() && existing.is_none() {
            conn.engine = Engine::Sqlite;
        }
    }
    if let Some(v) = s("color") {
        conn.color = if v.starts_with('#') { v.to_string() } else { format!("#{v}") };
    }
    if let Some(v) = s("tag") {
        conn.tag = Some(v.to_lowercase()).filter(|t| !t.is_empty());
    }
    if let Some(v) = s("ssl_mode") {
        conn.ssl.mode = parse_ssl(v)?;
    }
    if let Some(v) = args.get("ask_password_every_time").and_then(Value::as_bool) {
        conn.save_password = !v;
    }
    if let Some(v) = s("ssh_host") {
        conn.ssh.enabled = !v.is_empty();
        conn.ssh.host = v.to_string();
    }
    if let Some(p) = args.get("ssh_port").and_then(Value::as_u64) {
        conn.ssh.port = u16::try_from(p).map_err(|_| "`ssh_port` must be 1 to 65535.")?;
    }
    if let Some(v) = s("ssh_user") {
        conn.ssh.user = v.to_string();
    }
    if let Some(v) = str_arg(args, "ssh_password") {
        new_secrets.ssh_password = Some(v.to_string());
        conn.ssh.auth = crate::conn::model::SshAuth::Password;
    }
    if let Some(v) = s("ssh_key_file") {
        conn.ssh.key_file = Some(v.to_string()).filter(|k| !k.is_empty());
        conn.ssh.auth = crate::conn::model::SshAuth::Key;
    }
    if conn.engine == Engine::Postgres {
        return Err("Tableboat doesn't support PostgreSQL yet.".into());
    }
    if conn.engine == Engine::Sqlite {
        conn.host = String::new();
        conn.port = None;
    }
    ops::validate(&conn)?;
    if existing.is_none() {
        let name = conn.display_name().to_lowercase();
        if let Some(same) = store.list().into_iter().find(|c| c.display_name().to_lowercase() == name) {
            return Err(format!(
                "A connection named \"{}\" already exists. Pass connection: \"{}\" to change it, or choose another name.",
                same.display_name(),
                same.display_name()
            ));
        }
    }

    // What the user is asked about: no passwords, only whether they change.
    let password_note = match new_secrets.password.as_deref() {
        _ if conn.engine == Engine::Sqlite => None,
        _ if !conn.save_password => Some("password: asked every time".to_string()),
        Some("") if existing.is_some() => Some("password: removed".into()),
        Some(_) if existing.is_some() => Some(format!("password: changed (saved in {})", secrets::STORE_NAME)),
        Some(_) => Some(format!("password: saved in {}", secrets::STORE_NAME)),
        None if existing.is_some() => None,
        None => Some("password: none".into()),
    };
    let ssh_note =
        new_secrets.ssh_password.as_ref().map(|_| format!("SSH password: saved in {}", secrets::STORE_NAME));

    let test_result = if args.get("test").and_then(Value::as_bool).unwrap_or(true) {
        // Test with what would be stored: the new secrets over the saved ones.
        let mut secrets_for_test = existing.as_ref().map(|c| secrets::load(&c.id)).transpose()?.unwrap_or_default();
        if let Some(p) = &new_secrets.password {
            secrets_for_test.password = Some(p.clone()).filter(|p| !p.is_empty());
        }
        if let Some(p) = &new_secrets.ssh_password {
            secrets_for_test.ssh_password = Some(p.clone()).filter(|p| !p.is_empty());
        }
        Some(if !conn.save_password && secrets_for_test.password.is_none() && conn.engine != Engine::Sqlite {
            "not tested (the password is asked every time)".to_string()
        } else {
            match try_connect(host, &conn, &secrets_for_test) {
                Ok(server) => format!("connected ({server})"),
                Err(e) => format!("FAILED: {e}"),
            }
        })
    } else {
        None
    };

    let (question, mut lines) = match &existing {
        Some(old) => {
            let mut lines = changes(old, &conn);
            if let Some(n) = password_note {
                lines.push(n);
            }
            if let Some(n) = ssh_note {
                lines.push(n);
            }
            if lines.is_empty() {
                return Ok(format!("Nothing to change in \"{}\".", old.display_name()));
            }
            (format!("Change the connection \"{}\"?", old.display_name()), lines)
        }
        None => {
            let mut lines = vec![format!("\"{}\": {}", conn.display_name(), describe_connection(&conn, false).replace(", no password", ""))];
            lines.extend(password_note);
            lines.extend(ssh_note);
            ("Save this new connection?".to_string(), lines)
        }
    };
    if let Some(t) = &test_result {
        lines.push(format!("Test: {t}"));
    }
    let summary = lines.join("\n");
    confirm(host, "save_connection", &question, summary.clone())?;

    let saved = ops::save(store, conn, new_secrets)?;
    host.connections_changed();
    let c = &saved.connection;
    Ok(format!(
        "{} \"{}\": {}{}",
        if existing.is_some() { "Changed" } else { "Saved" },
        c.display_name(),
        describe_connection(c, saved.has_password),
        test_result.map(|t| format!("\nTest: {t}")).unwrap_or_default()
    ))
}

fn test_connection(host: &dyn Host, args: &Value) -> Result<String, String> {
    if let Some(u) = str_arg(args, "url").map(str::trim).filter(|s| !s.is_empty()) {
        let (conn, s) = url::parse(u)?;
        let server = try_connect(host, &conn, &s)?;
        return Ok(format!("Connected to {} ({server}).", target(&conn)));
    }
    let conn = find(host, required(args, "connection")?)?;
    let s = secrets::load(&conn.id)?;
    if !conn.save_password && conn.engine != Engine::Sqlite && s.password.is_none() {
        return Err(format!("\"{}\" asks for its password every time, so it can't be tested from here.", conn.display_name()));
    }
    let server = try_connect(host, &conn, &s)?;
    Ok(format!("Connected to \"{}\" ({server}).", conn.display_name()))
}

fn delete_connection(host: &dyn Host, args: &Value) -> Result<String, String> {
    let conn = find(host, required(args, "connection")?)?;
    let has = secrets::get(&conn.id, secrets::PASSWORD).ok().flatten().is_some();
    let mut summary = format!("\"{}\": {}", conn.display_name(), describe_connection(&conn, has));
    if host.context().connection_id.as_deref() == Some(conn.id.as_str()) {
        summary.push_str("\nThis window is connected to it; the window stays connected until it disconnects.");
    }
    confirm(host, "delete_connection", "Delete this saved connection and its saved passwords?", summary)?;
    ops::delete(host.connections(), &conn.id)?;
    host.connections_changed();
    Ok(format!("Deleted \"{}\".", conn.display_name()))
}

fn export_connections(host: &dyn Host, args: &Value) -> Result<String, String> {
    let path = resolve_path(host, required(args, "file")?)?;
    let with_passwords = args.get("with_passwords").and_then(Value::as_bool).unwrap_or(false);
    let chosen: Option<Vec<Connection>> = match args.get("connections").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => {
            Some(list.iter().filter_map(Value::as_str).map(|q| find(host, q)).collect::<Result<Vec<_>, _>>()?)
        }
        _ => None,
    };
    let names = match &chosen {
        Some(list) => list.iter().map(|c| format!("\"{}\"", c.display_name())).collect::<Vec<_>>().join(", "),
        None => format!("all {} saved connections", host.connections().list().len()),
    };
    let summary = format!(
        "{names}\nto {}{}\n{}",
        path.display(),
        replaces(&path),
        if with_passwords { "WITH PASSWORDS: anyone with the file can connect." } else { "Without passwords." }
    );
    confirm(host, "export_connections", "Export connections to a file?", summary)?;
    let ids = chosen.map(|l| l.into_iter().map(|c| c.id).collect::<Vec<_>>());
    let n = ops::export_some(host.connections(), ids.as_deref(), &path, with_passwords)?;
    Ok(format!(
        "Exported {n} connection{} to {}{}.",
        if n == 1 { "" } else { "s" },
        path.display(),
        if with_passwords { " (with passwords)" } else { "" }
    ))
}

fn import_connections(host: &dyn Host, args: &Value) -> Result<String, String> {
    let path = resolve_path(host, required(args, "file")?)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let found: Vec<String> = text
        .lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match url::parse(l) {
            Ok((c, _)) => format!("\"{}\": {} {}", c.display_name(), c.engine.label(), target(&c)),
            Err(e) => format!("(unreadable line: {e})"),
        })
        .collect();
    if found.is_empty() {
        return Err(format!("{} has no connection URLs.", path.display()));
    }
    let summary = format!("From {}:\n{}", path.display(), clip(&found.join("\n"), 3000));
    confirm(host, "import_connections", &format!("Add {} connection{}?", found.len(), if found.len() == 1 { "" } else { "s" }), summary)?;
    let (n, errors) = ops::import_file(host.connections(), &path)?;
    host.connections_changed();
    let mut out = format!("Imported {n} connection{}.", if n == 1 { "" } else { "s" });
    for e in errors {
        out.push_str(&format!("\n{e}"));
    }
    Ok(out)
}

// ---------- data ----------

fn export_data(host: &dyn Host, args: &Value) -> Result<String, String> {
    let conn = connection_or_current(host, args)?;
    let path = resolve_path(host, required(args, "file")?)?;
    let format = export::format_for(&path, None)?;
    let table = str_arg(args, "table").map(str::trim).filter(|s| !s.is_empty());
    let query = str_arg(args, "query").map(str::trim).filter(|s| !s.is_empty());
    let limit = args.get("limit").and_then(Value::as_u64).map(|n| n as usize);
    if table.is_none() && query.is_none() {
        return Err("Give `table` (or a collection) or `query`.".into());
    }
    let (session, default_db) = open_saved(host, &conn)?;
    if let Some(q) = query {
        check_read_only(&session, q)?;
    }
    let database = str_arg(args, "database").map(str::to_string).or(default_db);
    let what = match (table, query) {
        (_, Some(q)) => format!("the result of:\n{}", clip(q, 2000)),
        (Some(t), None) => format!("{}{t}", database.as_deref().map(|d| format!("{d}.")).unwrap_or_default()),
        (None, None) => unreachable!(),
    };
    let summary = format!(
        "{what}\nfrom \"{}\"{}\nto {}{}",
        conn.display_name(),
        limit.map(|n| format!(", at most {n} rows")).unwrap_or_default(),
        path.display(),
        replaces(&path)
    );
    confirm(host, "export_data", "Export to a file?", summary)?;
    let sql = match (table, query) {
        (_, Some(q)) => q.trim().trim_end_matches(';').to_string(),
        (Some(t), None) => export::table_query(&session, database.as_deref(), t, limit),
        (None, None) => unreachable!(),
    };
    let started = std::time::Instant::now();
    let n = host.runtime().block_on(export::query_to_file(
        &session,
        host.runs(),
        database.as_deref(),
        sql,
        table,
        &path,
        format,
        limit.unwrap_or(50_000_000),
    ))?;
    Ok(format!("Exported {n} row{} to {} in {:.1} s.", if n == 1 { "" } else { "s" }, path.display(), started.elapsed().as_secs_f64()))
}

/// export_data runs one statement, and only one that reads.
fn check_read_only(session: &db::SessionHandle, sql: &str) -> Result<(), String> {
    let sql = sql.trim().trim_end_matches(';');
    if matches!(session.backend, db::Backend::Mongo(_)) {
        use crate::db::mongo_shell::{Command, parse};
        return match parse(sql)? {
            Command::Collection { op, .. } if op.is_read_only() => Ok(()),
            Command::Collection { .. } => Err("export_data only exports reads (find, aggregate without $out/$merge).".into()),
            _ => Err("export_data needs a collection query such as db.users.find({}).".into()),
        };
    }
    if sql.contains(';') {
        return Err("export_data exports one query at a time.".into());
    }
    let first = sql.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if ["select", "with", "show", "values", "table", "pragma", "explain", "describe", "desc"].contains(&first.as_str()) {
        Ok(())
    } else {
        Err(format!("export_data exports the result of a read query, not {}.", first.to_uppercase()))
    }
}

fn dump_database(host: &dyn Host, args: &Value) -> Result<String, String> {
    let conn = connection_or_current(host, args)?;
    let (session, default_db) = open_saved(host, &conn)?;
    let rt = host.runtime();
    let database = match str_arg(args, "database").map(str::to_string).or(default_db) {
        Some(d) => d,
        None => {
            let list = rt.block_on(db::meta::databases(&session, false)).unwrap_or_default();
            return Err(format!("Which database? Pass `database`. \"{}\" has: {}", conn.display_name(), list.join(", ")));
        }
    };
    let mongo = conn.engine == Engine::Mongodb;
    let mut path = resolve_path(host, required(args, "file")?)?;
    if !mongo && path.extension().is_none() {
        path.set_extension("sql");
    }
    let tables: Option<Vec<String>> = args
        .get("tables")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
        .filter(|l| !l.is_empty());
    let data = args.get("data").and_then(Value::as_bool).unwrap_or(true);

    // Size hint for the question (MySQL keeps estimated row counts).
    let objects = rt.block_on(db::meta::objects(&session, &database, false)).unwrap_or_default();
    let in_scope = |name: &str| tables.as_ref().is_none_or(|t| t.iter().any(|x| x == name));
    let count = objects.iter().filter(|o| o.kind == db::meta::ObjectKind::Table && in_scope(&o.name)).count();
    let rows: u64 = objects.iter().filter(|o| in_scope(&o.name)).filter_map(|o| o.rows).sum();
    let summary = format!(
        "{database} on \"{}\": {} {}{}{}\n{}\nto {}{}",
        conn.display_name(),
        count,
        if mongo { "collections" } else { "tables" },
        if rows > 0 { format!(", about {rows} rows") } else { String::new() },
        tables.as_ref().map(|t| format!(" (only {})", t.join(", "))).unwrap_or_default(),
        if !data {
            "Structure only."
        } else if mongo {
            "One JSON file per collection (canonical Extended JSON, for mongoimport), in this folder."
        } else {
            "Structure and data, as a SQL script."
        },
        path.display(),
        replaces(&path)
    );
    confirm(host, "dump_database", "Export this database?", summary)?;
    let started = std::time::Instant::now();
    let opts = export::DumpOptions { tables, data };
    let stats = rt.block_on(export::dump(&session, host.runs(), &database, &path, &opts))?;
    Ok(format!("Exported {database} ({}) to {} in {:.1} s.", stats.describe(), stats.path.display(), started.elapsed().as_secs_f64()))
}

// ---------- app ----------

fn open_connection(host: &dyn Host, args: &Value) -> Result<String, String> {
    let conn = find(host, required(args, "connection")?)?;
    host.open_in_app(&conn.id)
}

fn duplicate_connection(host: &dyn Host, args: &Value) -> Result<String, String> {
    let conn = find(host, required(args, "connection")?)?;
    let has = secrets::get(&conn.id, secrets::PASSWORD).ok().flatten().is_some();
    confirm(
        host,
        "duplicate_connection",
        "Copy this connection?",
        format!(
            "\"{}\": {}\nThe copy is named \"{} copy\" and keeps the saved passwords.",
            conn.display_name(),
            describe_connection(&conn, has),
            conn.display_name()
        ),
    )?;
    let copy = ops::duplicate(host.connections(), &conn.id)?;
    host.connections_changed();
    Ok(format!("Saved \"{}\".", copy.connection.display_name()))
}

// ---------- schema browsing ----------

fn kind_arg(args: &Value) -> Result<Option<db::meta::ObjectKind>, String> {
    match str_arg(args, "kind").map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(k) => serde_json::from_value(json!(k.to_ascii_lowercase()))
            .map(Some)
            .map_err(|_| "kind is one of table, view, function, procedure, trigger.".to_string()),
    }
}

fn kind_name(k: db::meta::ObjectKind) -> &'static str {
    match k {
        db::meta::ObjectKind::Table => "table",
        db::meta::ObjectKind::View => "view",
        db::meta::ObjectKind::Function => "function",
        db::meta::ObjectKind::Procedure => "procedure",
        db::meta::ObjectKind::Trigger => "trigger",
    }
}

/// The database a schema tool works in: the one asked for, else the default.
fn database_or(args: &Value, default_db: Option<String>, session: &db::SessionHandle, host: &dyn Host) -> Result<String, String> {
    if let Some(d) = str_arg(args, "database").map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(d.to_string());
    }
    default_db.ok_or_else(|| {
        let list = host.runtime().block_on(db::meta::databases(session, false)).unwrap_or_default();
        format!("No database selected; pass `database`. Databases: {}", list.join(", "))
    })
}

fn list_databases(host: &dyn Host, args: &Value) -> Result<String, String> {
    let (session, current) = target_session(host, args)?;
    let list = host.runtime().block_on(db::meta::databases(&session, true))?;
    let lines: String = list
        .iter()
        .map(|d| format!("- {d}{}\n", if current.as_deref() == Some(d.as_str()) { " (current)" } else { "" }))
        .collect();
    Ok(format!("{} database{}:\n{lines}", list.len(), if list.len() == 1 { "" } else { "s" }))
}

fn list_objects(host: &dyn Host, args: &Value) -> Result<String, String> {
    let (session, default_db) = target_session(host, args)?;
    let database = database_or(args, default_db, &session, host)?;
    let kind = kind_arg(args)?;
    let objects = host.runtime().block_on(db::meta::objects(&session, &database, true))?;
    let shown: Vec<_> = objects.iter().filter(|o| kind.is_none_or(|k| o.kind == k)).collect();
    let mut out = format!("{database}: {} object{}\n", shown.len(), if shown.len() == 1 { "" } else { "s" });
    for o in shown {
        let rows = o.rows.map(|r| format!(", ~{r} rows")).unwrap_or_default();
        let parent = o.parent.as_deref().map(|p| format!(", on {p}")).unwrap_or_default();
        out.push_str(&format!("- {} ({}{rows}{parent})\n", o.name, kind_name(o.kind)));
    }
    Ok(clip(&out, 60_000))
}

/// An object by name (exact, else ignoring case), optionally of one kind.
fn find_object(
    host: &dyn Host,
    session: &db::SessionHandle,
    database: &str,
    name: &str,
    kind: Option<db::meta::ObjectKind>,
) -> Result<db::meta::DbObject, String> {
    let objects = host.runtime().block_on(db::meta::objects(session, database, false))?;
    let fits = |o: &&db::meta::DbObject| kind.is_none_or(|k| o.kind == k);
    objects
        .iter()
        .filter(fits)
        .find(|o| o.name == name)
        .or_else(|| objects.iter().filter(fits).find(|o| o.name.eq_ignore_ascii_case(name)))
        .cloned()
        .ok_or_else(|| format!("There's no {} named \"{name}\" in {database} (see list_objects).", kind.map(kind_name).unwrap_or("object")))
}

fn get_definition(host: &dyn Host, args: &Value) -> Result<String, String> {
    let (session, default_db) = target_session(host, args)?;
    let database = database_or(args, default_db, &session, host)?;
    let obj = find_object(host, &session, &database, required(args, "name")?, kind_arg(args)?)?;
    let ddl = host.runtime().block_on(db::meta::ddl(&session, &database, obj.kind, &obj.name))?;
    Ok(format!("{} {database}.{}:\n```sql\n{}\n```", kind_name(obj.kind), obj.name, clip(ddl.trim(), 60_000)))
}

// ---------- the window's UI ----------

/// Palette commands that only change the view (they run without asking). Anything else
/// (running SQL, saving or discarding edits, closing tabs, disconnecting...) asks first.
const SAFE_COMMANDS: &[&str] = &[
    "theme.", "density.", "view.zoom", "view.refresh", "tab.next", "tab.previous", "tab.goto", "tab.keep", "sidebar.",
    "table.reload", "query.new", "query.format", "query.focus", "query.cancel", "query.stopOnError", "results.", "help.",
    "settings.", "go.", "palette.", "connection.copyUrl",
];

fn list_app_commands(host: &dyn Host) -> Result<String, String> {
    let cmds = host.ui("commands", json!({}))?;
    let list = cmds.as_array().cloned().unwrap_or_default();
    let mut out = format!("{} commands (run one with run_app_command):\n", list.len());
    for c in &list {
        let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let on = match c.get("checked").and_then(Value::as_bool) {
            Some(true) => " [on]",
            Some(false) => " [off]",
            None => "",
        };
        let keys = c.get("keys").and_then(Value::as_str).map(|k| format!(" ({k})")).unwrap_or_default();
        out.push_str(&format!("- {}: {} > {}{on}{keys}\n", text("id"), text("category"), text("title")));
    }
    Ok(out)
}

fn run_app_command(host: &dyn Host, args: &Value) -> Result<String, String> {
    let id = required(args, "id")?;
    let cmds = host.ui("commands", json!({}))?;
    let cmd = cmds
        .as_array()
        .and_then(|l| l.iter().find(|c| c.get("id").and_then(Value::as_str) == Some(id)).cloned())
        .ok_or_else(|| format!("There's no command \"{id}\" right now (see list_app_commands)."))?;
    let title = format!(
        "{} > {}",
        cmd.get("category").and_then(Value::as_str).unwrap_or(""),
        cmd.get("title").and_then(Value::as_str).unwrap_or(id)
    );
    if !SAFE_COMMANDS.iter().any(|p| id.starts_with(p)) {
        confirm(host, "run_app_command", "Run this command in Tableboat?", title.clone())?;
    }
    host.ui("run", json!({ "id": id }))?;
    Ok(format!("Done: {title}."))
}

fn open_object(host: &dyn Host, args: &Value) -> Result<String, String> {
    let (session, ctx) = session_for(host)?;
    let database = database_or(args, ctx.database, &session, host)?;
    let obj = find_object(host, &session, &database, required(args, "name")?, kind_arg(args)?)?;
    host.ui("open_object", json!({ "database": database, "name": obj.name, "kind": kind_name(obj.kind) }))?;
    Ok(format!("Opened {} {database}.{} in a tab.", kind_name(obj.kind), obj.name))
}

fn open_sql(host: &dyn Host, args: &Value) -> Result<String, String> {
    let sql = str_arg(args, "sql").filter(|s| !s.trim().is_empty()).ok_or("`sql` is required.")?;
    let insert = str_arg(args, "mode") == Some("insert");
    let run = args.get("run").and_then(Value::as_bool).unwrap_or(false);
    if run && !host.ask("mcp__tableboat__execute_sql", json!({ "sql": sql }))? {
        // The same question as execute_sql: running it does whatever the SQL does.
        return Err("The user denied running this SQL in Tableboat.".into());
    }
    host.ui("open_sql", json!({ "sql": sql, "mode": if insert { "insert" } else { "new_tab" }, "run": run }))?;
    Ok(match (insert, run) {
        (true, false) => "Inserted into the active editor.".into(),
        (_, false) => "Opened in a new SQL tab.".into(),
        (_, true) => "Opened in a new SQL tab and started; the results show in the window (get_app_state reads them).".into(),
    })
}

fn use_database(host: &dyn Host, args: &Value) -> Result<String, String> {
    let (session, _) = session_for(host)?;
    let wanted = required(args, "database")?;
    let list = host.runtime().block_on(db::meta::databases(&session, false))?;
    let db = list
        .iter()
        .find(|d| d.as_str() == wanted)
        .or_else(|| list.iter().find(|d| d.eq_ignore_ascii_case(wanted)))
        .ok_or_else(|| format!("There's no database \"{wanted}\". Databases: {}", list.join(", ")))?;
    host.ui("use_database", json!({ "database": db }))?;
    Ok(format!("The window now uses {db}."))
}

fn check_for_updates(host: &dyn Host) -> Result<String, String> {
    let c = host.check_update()?;
    Ok(match c.available {
        None => format!("Tableboat {} is the latest version.", c.current),
        Some((version, notes)) => format!(
            "Tableboat {version} is available (running: {}). The app shows its update notice; install_update installs it (after asking the user).\n\nRelease notes:\n{}",
            c.current,
            clip(notes.trim(), 8000)
        ),
    })
}

fn install_update(host: &dyn Host) -> Result<String, String> {
    let c = host.check_update()?;
    let Some((version, _)) = c.available else {
        return Ok(format!("Nothing to install: Tableboat {} is the latest version.", c.current));
    };
    confirm(
        host,
        "install_update",
        &format!("Install Tableboat {version} and restart?"),
        format!(
            "Tableboat {version} replaces {}. Tableboat saves your tabs, closes, installs the update and opens again.\nThis chat stops; you can continue it from History.",
            c.current
        ),
    )?;
    host.start_update_install()?;
    Ok(format!("Installing Tableboat {version}: Tableboat will close and open again in a moment."))
}

#[cfg(test)]
mod tests {
    //! The app tools end to end, through a fake host that records every question, against a
    //! throwaway data folder and SQLite files (nothing real is touched; SQLite needs no password,
    //! so the system keychain isn't written either).

    use super::call;
    use crate::claude::AiContext;
    use crate::conn::store::ConnectionStore;
    use crate::db::{Sessions, query::Runs};
    use crate::mcp::Host;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct TestHost {
        dir: PathBuf,
        store: ConnectionStore,
        sessions: Sessions,
        runs: Runs,
        rt: tokio::runtime::Handle,
        answer: Mutex<bool>,
        asked: Mutex<Vec<(String, String)>>,
        opened: Mutex<Vec<String>>,
        linked: Mutex<Vec<(String, String)>>,
        /// Connections "added to the chat" (readable with `connection`).
        links: Mutex<Vec<String>>,
        /// What the pretend window was asked to do.
        ui_log: Mutex<Vec<(String, Value)>>,
    }

    impl Host for TestHost {
        fn context(&self) -> AiContext {
            AiContext::default()
        }
        fn sessions(&self) -> &Sessions {
            &self.sessions
        }
        fn runs(&self) -> &Runs {
            &self.runs
        }
        fn connections(&self) -> &ConnectionStore {
            &self.store
        }
        fn runtime(&self) -> &tokio::runtime::Handle {
            &self.rt
        }
        fn owner(&self) -> String {
            "test".into()
        }
        fn linked(&self) -> Vec<String> {
            self.links.lock().unwrap().clone()
        }
        fn linked_session(&self, id: &str) -> Option<String> {
            self.linked.lock().unwrap().iter().find(|(c, _)| c == id).map(|(_, s)| s.clone())
        }
        fn set_linked_session(&self, id: &str, sid: &str) {
            self.linked.lock().unwrap().push((id.into(), sid.into()));
        }
        fn ask(&self, tool: &str, input: Value) -> Result<bool, String> {
            let summary = input.get("summary").and_then(Value::as_str).unwrap_or("").to_string();
            self.asked.lock().unwrap().push((tool.into(), summary));
            Ok(*self.answer.lock().unwrap())
        }
        fn how_to_link(&self) -> &'static str {
            "--with"
        }
        fn open_in_app(&self, id: &str) -> Result<String, String> {
            self.opened.lock().unwrap().push(id.into());
            Ok("opened".into())
        }
        fn files_dir(&self) -> PathBuf {
            self.dir.clone()
        }
        fn check_update(&self) -> Result<crate::mcp::UpdateCheck, String> {
            Ok(crate::mcp::UpdateCheck { current: "0.1.0".into(), available: Some(("0.2.0".into(), "### Added\n- Things".into())) })
        }
        fn ui(&self, action: &str, args: Value) -> Result<Value, String> {
            self.ui_log.lock().unwrap().push((action.to_string(), args));
            Ok(match action {
                "commands" => json!([
                    { "id": "theme.dark", "category": "Theme", "title": "Dark", "checked": false },
                    { "id": "edit.save", "category": "Edit", "title": "Save Changes", "keys": "Ctrl+S" }
                ]),
                "state" => json!({ "database": "main", "tabs": [] }),
                _ => Value::Null,
            })
        }
        fn start_update_install(&self) -> Result<(), String> {
            self.opened.lock().unwrap().push("update".into());
            Ok(())
        }
    }

    fn tool(h: &TestHost, name: &str, args: Value) -> Result<String, String> {
        call(h, name, &args).expect("an app tool")
    }

    #[test]
    fn app_tools() {
        let dir = std::env::temp_dir().join(format!("tableboat-apptools-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before anything in this test reads it; no other test reads the data folder.
        unsafe { std::env::set_var("TABLEBOAT_DATA_DIR", &dir) };
        crate::conn::secrets::init().unwrap();

        let db_file = dir.join("shop.sqlite");
        {
            let c = rusqlite::Connection::open(&db_file).unwrap();
            c.execute_batch(
                "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, note TEXT);
                 CREATE INDEX users_name ON users(name);
                 INSERT INTO users (name, note) VALUES ('Ann', 'it''s; tricky'), ('Bob', NULL), ('Zoë', 'x');
                 CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id), total REAL);
                 INSERT INTO orders (user_id, total) VALUES (1, 9.5), (2, 12);
                 CREATE VIEW big AS SELECT * FROM orders WHERE total > 10;",
            )
            .unwrap();
        }

        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let h = TestHost {
            dir: dir.clone(),
            store: ConnectionStore::load(&dir).unwrap(),
            sessions: Sessions::default(),
            runs: Runs::default(),
            rt: rt.handle().clone(),
            answer: Mutex::new(true),
            asked: Mutex::new(vec![]),
            opened: Mutex::new(vec![]),
            linked: Mutex::new(vec![]),
            links: Mutex::new(vec![]),
            ui_log: Mutex::new(vec![]),
        };

        // Settings: asked, saved, refused when out of range, nothing asked when nothing changes.
        let out = tool(&h, "update_settings", json!({ "changes": { "theme": "dark", "editorFontSize": 15 } })).unwrap();
        assert!(out.contains("theme: \"system\" → \"dark\""), "{out}");
        assert_eq!(crate::prefs::load(&crate::prefs::path().unwrap()).theme, "dark");
        assert!(tool(&h, "update_settings", json!({ "changes": { "theme": "purple" } })).is_err());
        assert!(tool(&h, "update_settings", json!({ "changes": { "nope": 1 } })).is_err());
        let n = h.asked.lock().unwrap().len();
        assert!(tool(&h, "update_settings", json!({ "changes": { "theme": "dark" } })).unwrap().contains("Nothing to change"));
        assert_eq!(h.asked.lock().unwrap().len(), n);
        assert!(tool(&h, "get_settings", json!({})).unwrap().contains("\"theme\": \"dark\""));
        // Colour themes: an id of the right mode is saved; a light theme as darkTheme or an
        // unknown id is refused with the valid ids.
        let out = tool(&h, "update_settings", json!({ "changes": { "darkTheme": "catppuccin-mocha" } })).unwrap();
        assert!(out.contains("darkTheme: \"tableboat-dark\" → \"catppuccin-mocha\""), "{out}");
        assert_eq!(crate::prefs::load(&crate::prefs::path().unwrap()).dark_theme, "catppuccin-mocha");
        let err = tool(&h, "update_settings", json!({ "changes": { "darkTheme": "catppuccin-latte" } })).unwrap_err();
        assert!(err.contains("light theme") && err.contains("tokyo-night"), "{err}");
        assert!(tool(&h, "update_settings", json!({ "changes": { "lightTheme": "nope" } })).unwrap_err().contains("github-light"));
        assert_eq!(crate::prefs::load(&crate::prefs::path().unwrap()).dark_theme, "catppuccin-mocha");
        assert!(tool(&h, "get_settings", json!({})).unwrap().contains("darkTheme: colour theme used in dark mode"));

        // A new connection from a file path; tested before the question.
        let out = tool(&h, "save_connection", json!({ "name": "Shop", "file": db_file.display().to_string(), "tag": "Local" })).unwrap();
        assert!(out.starts_with("Saved \"Shop\""), "{out}");
        let (t, summary) = h.asked.lock().unwrap().last().cloned().unwrap();
        assert_eq!(t, "mcp__tableboat__save_connection");
        assert!(summary.contains("Test: connected (SQLite"), "{summary}");
        assert!(tool(&h, "save_connection", json!({ "name": "shop", "file": "x.db" })).unwrap_err().contains("already exists"));
        assert!(tool(&h, "list_connections", json!({})).unwrap().contains("\"Shop\": SQLite"));
        // Editing shows only what changes.
        tool(&h, "save_connection", json!({ "connection": "shop", "color": "#22c55e" })).unwrap();
        let (_, summary) = h.asked.lock().unwrap().last().cloned().unwrap();
        assert!(summary.starts_with("color: \"#6b7280\" → \"#22c55e\""), "{summary}");
        assert!(tool(&h, "test_connection", json!({ "connection": "Shop" })).unwrap().contains("Connected"));

        // Exports.
        let out = tool(&h, "export_data", json!({ "connection": "Shop", "table": "users", "file": "users.csv" })).unwrap();
        assert!(out.starts_with("Exported 3 rows"), "{out}");
        let csv = std::fs::read_to_string(dir.join("users.csv")).unwrap();
        assert!(csv.contains("id,name,note") && csv.contains("Zoë"), "{csv}");
        tool(&h, "export_data", json!({ "connection": "Shop", "query": "select name from users where id > 1", "file": "q.json" })).unwrap();
        assert!(std::fs::read_to_string(dir.join("q.json")).unwrap().contains("\"Bob\""));
        assert!(tool(&h, "export_data", json!({ "connection": "Shop", "query": "delete from users", "file": "x.csv" })).is_err());

        let out = tool(&h, "dump_database", json!({ "connection": "Shop", "file": "shop-dump" })).unwrap();
        assert!(out.contains("2 tables, 1 view, 1 routine/trigger, 5 rows"), "{out}");
        let sql = std::fs::read_to_string(dir.join("shop-dump.sql")).unwrap();
        // The dump restores into an empty database with the same rows.
        let copy = rusqlite::Connection::open(dir.join("restored.sqlite")).unwrap();
        copy.execute_batch(&sql).unwrap();
        let count = |sql: &str| copy.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count("SELECT count(*) FROM users"), 3);
        assert_eq!(count("SELECT count(*) FROM big"), 1);
        assert_eq!(count("SELECT count(*) FROM sqlite_master WHERE name = 'users_name'"), 1);
        let note: String = copy.query_row("SELECT note FROM users WHERE name = 'Ann'", [], |r| r.get(0)).unwrap();
        assert_eq!(note, "it's; tricky");
        drop(copy);

        tool(&h, "export_connections", json!({ "file": "conns.txt" })).unwrap();
        let text = std::fs::read_to_string(dir.join("conns.txt")).unwrap();
        assert!(text.contains("sqlite:///") && text.contains("name=Shop"), "{text}");

        // Saying no changes nothing.
        *h.answer.lock().unwrap() = false;
        assert!(tool(&h, "delete_connection", json!({ "connection": "Shop" })).is_err());
        assert!(tool(&h, "update_settings", json!({ "changes": { "theme": "light" } })).is_err());
        assert_eq!(crate::prefs::load(&crate::prefs::path().unwrap()).theme, "dark");
        assert_eq!(h.store.list().len(), 1);
        *h.answer.lock().unwrap() = true;

        tool(&h, "open_connection", json!({ "connection": "shop" })).unwrap();
        assert_eq!(h.opened.lock().unwrap().len(), 1);

        // Schema browsing on a connection added to the chat (not added: refused).
        assert!(tool(&h, "list_objects", json!({ "connection": "Shop" })).unwrap_err().contains("hasn't added"));
        let shop = h.store.list().into_iter().find(|c| c.name == "Shop").unwrap();
        h.links.lock().unwrap().push(shop.id.clone());
        assert!(tool(&h, "list_databases", json!({ "connection": "Shop" })).unwrap().contains("- main (current)"));
        let objs = tool(&h, "list_objects", json!({ "connection": "Shop" })).unwrap();
        assert!(objs.contains("- users (table)") && objs.contains("- big (view)"), "{objs}");
        assert!(!tool(&h, "list_objects", json!({ "connection": "Shop", "kind": "view" })).unwrap().contains("users"));
        let def = tool(&h, "get_definition", json!({ "connection": "Shop", "name": "BIG" })).unwrap();
        assert!(def.starts_with("view main.big:") && def.contains("CREATE VIEW big"), "{def}");
        assert!(tool(&h, "get_definition", json!({ "connection": "Shop", "name": "nope" })).unwrap_err().contains("no object named"));

        // The window: palette commands; view-only ones run freely, others ask first.
        let list = tool(&h, "list_app_commands", json!({})).unwrap();
        assert!(list.contains("- theme.dark: Theme > Dark [off]") && list.contains("- edit.save: Edit > Save Changes (Ctrl+S)"), "{list}");
        let asked = h.asked.lock().unwrap().len();
        tool(&h, "run_app_command", json!({ "id": "theme.dark" })).unwrap();
        assert_eq!(h.asked.lock().unwrap().len(), asked, "a theme change doesn't ask");
        *h.answer.lock().unwrap() = false;
        assert!(tool(&h, "run_app_command", json!({ "id": "edit.save" })).is_err());
        assert!(!h.ui_log.lock().unwrap().iter().any(|(a, v)| a == "run" && v["id"] == "edit.save"), "denied: not run");
        *h.answer.lock().unwrap() = true;
        tool(&h, "run_app_command", json!({ "id": "edit.save" })).unwrap();
        assert!(h.ui_log.lock().unwrap().iter().any(|(a, v)| a == "run" && v["id"] == "edit.save"));
        assert!(tool(&h, "run_app_command", json!({ "id": "nope" })).unwrap_err().contains("no command"));
        // Opening SQL is free; running it asks like execute_sql.
        let asked = h.asked.lock().unwrap().len();
        tool(&h, "open_sql", json!({ "sql": "select 1" })).unwrap();
        assert_eq!(h.asked.lock().unwrap().len(), asked);
        tool(&h, "open_sql", json!({ "sql": "delete from t", "run": true })).unwrap();
        assert_eq!(h.asked.lock().unwrap().last().unwrap().0, "mcp__tableboat__execute_sql");
        assert!(tool(&h, "get_app_state", json!({})).unwrap().contains("\"database\": \"main\""));
        h.links.lock().unwrap().clear();

        // Updates: the check reads freely; installing asks, and "no" installs nothing.
        let out = tool(&h, "check_for_updates", json!({})).unwrap();
        assert!(out.starts_with("Tableboat 0.2.0 is available (running: 0.1.0)") && out.contains("- Things"), "{out}");
        *h.answer.lock().unwrap() = false;
        assert!(tool(&h, "install_update", json!({})).is_err());
        assert_eq!(h.opened.lock().unwrap().len(), 1);
        *h.answer.lock().unwrap() = true;
        assert!(tool(&h, "install_update", json!({})).unwrap().starts_with("Installing Tableboat 0.2.0"));
        assert_eq!(h.opened.lock().unwrap().last().map(String::as_str), Some("update"));
        let (t, summary) = h.asked.lock().unwrap().last().cloned().unwrap();
        assert_eq!(t, "mcp__tableboat__install_update");
        assert!(summary.contains("replaces 0.1.0"), "{summary}");
        rt.block_on(h.sessions.close_window("test"));
        tool(&h, "delete_connection", json!({ "connection": "Shop" })).unwrap();
        assert!(h.store.list().is_empty());
        let out = tool(&h, "import_connections", json!({ "file": "conns.txt" })).unwrap();
        assert!(out.starts_with("Imported 1 connection"), "{out}");
        for c in h.store.list() {
            crate::conn::ops::delete(&h.store, &c.id).unwrap();
        }

        drop(h);
        rt.shutdown_background();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
