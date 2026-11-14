//! Claude Code (`claude.exe` on Windows, `claude` on macOS) helpers shared by the app's Claude
//! pane and `tb ask`.
//!
//! Each connection is its own Claude Code project: `~/.tableboat/sessions/<connection>/`. So its
//! chats, its CLAUDE.md notes and Claude Code's memory for it stay separate from other
//! connections. Without a connection, chats go to `~/.tableboat/sessions/general/`.

use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What Tableboat tells Claude about what the user has open (the `get_context` tool).
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

/// Where the per-connection project folders live: `~/.tableboat/sessions` (the user's profile,
/// so the app works the same on any PC).
pub fn sessions_base() -> PathBuf {
    crate::paths::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tableboat")
        .join("sessions")
}

/// A folder name from a connection name: letters, digits, '.', '_' and '-' only.
pub fn folder_name(name: &str) -> String {
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
    if out.is_empty() || reserved.contains(&out.as_str()) {
        format!("{}-db", if out.is_empty() { "connection" } else { &out })
    } else {
        out
    }
}

static PROJECTS_LOCK: Mutex<()> = Mutex::new(());

/// The connection's project folder if one was already chosen (nothing is created).
pub fn existing_project_folder(connection_id: &str) -> Option<PathBuf> {
    let path = crate::paths::data_dir().ok()?.join("state").join("claude-projects.json");
    let map: std::collections::BTreeMap<String, String> = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    map.get(connection_id).map(|f| sessions_base().join(f))
}

/// The project folder for a connection. The name is chosen the first time (from the connection's
/// name) and remembered in `state/claude-projects.json`, so renaming the connection keeps its chats.
pub fn project_folder(connection_id: Option<&str>, connection_name: Option<&str>) -> PathBuf {
    let base = sessions_base();
    let Some(id) = connection_id.filter(|id| !id.is_empty()) else { return base.join("general") };
    let _guard = PROJECTS_LOCK.lock().unwrap();
    let map_path = crate::paths::data_dir().ok().map(|d| d.join("state").join("claude-projects.json"));
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

/// Create the project folder and a starter CLAUDE.md (Claude Code reads it every chat): a place
/// for notes about this database that should carry over between chats. Never overwritten.
pub fn prepare_folder(folder: &Path, connection_name: Option<&str>, engine: Option<&str>) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
    let path = folder.join("CLAUDE.md");
    if path.exists() || folder.file_name().map(|n| n == "general").unwrap_or(false) {
        return Ok(());
    }
    let name = connection_name.unwrap_or("this connection");
    let engine = engine.unwrap_or("database");
    let text = format!(
        "# {name}\n\nNotes for Claude about the Tableboat connection \"{name}\" ({engine}). Claude Code reads this \
         file at the start of every chat for this connection, so keep what should be remembered here: what the \
         databases and tables are for, naming conventions, columns to be careful with, queries that are often \
         needed.\n\n## Notes\n\n"
    );
    let _ = std::fs::write(path, text);
    Ok(())
}

/// The Claude Code program on PATH or in a usual install folder.
pub fn find_claude() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    if let Some(path) = std::env::var_os("PATH") {
        if let Some(found) = find_in(&path, names) {
            return Some(found);
        }
    }
    // A macOS app started from the Dock or Finder gets the system's minimal PATH, not the
    // user's: ask their login shell where things really are (once per run).
    #[cfg(target_os = "macos")]
    if let Some(path) = login_shell_path() {
        if let Some(found) = find_in(&path, names) {
            return Some(found);
        }
    }
    // Well-known places, for installs that aren't on any PATH here.
    let home = crate::paths::home_dir();
    let exe = names[0];
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(home) = &home {
        candidates.push(home.join(".local/bin").join(exe));
        candidates.push(home.join(".claude/local").join(exe));
        candidates.push(home.join(".bun/bin").join(exe));
        candidates.push(home.join(".volta/bin").join(exe));
    }
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/opt/homebrew/bin").join(exe));
        candidates.push(PathBuf::from("/usr/local/bin").join(exe));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Each folder on `path`, looking for one of `names`.
fn find_in(path: &std::ffi::OsStr, names: &[&str]) -> Option<PathBuf> {
    for dir in std::env::split_paths(path) {
        for name in names {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The PATH of the user's login shell (macOS apps don't inherit it). Empty when the shell
/// can't be asked, e.g. in a sandbox.
#[cfg(target_os = "macos")]
fn login_shell_path() -> Option<std::ffi::OsString> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let out = std::process::Command::new(shell).args(["-l", "-c", "printf %s \"$PATH\""]).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let path = String::from_utf8(out.stdout).ok()?;
            (!path.trim().is_empty()).then(|| std::ffi::OsString::from(path))
        })
        .clone()
}

pub const NOT_INSTALLED: &str = "Claude Code (claude) wasn't found. Install it from \
    https://claude.com/claude-code and sign in once in a terminal.";

/// Claude Code keeps a project's sessions in ~/.claude/projects/<path with non-alphanumerics as '-'>.
pub fn sessions_store(folder: &Path) -> Option<PathBuf> {
    let encoded: String =
        folder.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Some(crate::paths::home_dir()?.join(".claude").join("projects").join(encoded))
}

/// The "model" entry of Claude Code's user settings, if any.
pub fn default_model() -> Option<String> {
    let home = crate::paths::home_dir()?;
    let text = std::fs::read_to_string(home.join(".claude").join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let model = value.get("model")?.as_str()?.trim();
    (!model.is_empty()).then(|| model.to_string())
}

/// What the app tools can do (shared by both prompts).
const APP_TOOLS: &str = "Tableboat's app tools work on the app itself: list_connections; get_settings and update_settings (theme, density, zoom, font sizes, panes, rows per page, Claude model, ...); save_connection (create a connection from the details or a URL the user gives, or change one), test_connection, delete_connection; export_connections and import_connections (files of connection URLs); export_data (a table, collection or query result to .csv, .tsv, .json, .xlsx, .md or .sql); dump_database (a whole database to a .sql script; MongoDB: a folder with one JSON file per collection); open_connection (a new Tableboat window); check_for_updates and install_update (Tableboat itself); list_databases, list_objects and get_definition (schema browsing). In the app's window: get_app_state (tabs, edits and results on screen), list_app_commands and run_app_command (everything in the command palette: theme, zoom, tabs, run or format SQL, save or discard table edits, disconnect...), open_object, open_sql and use_database. Tools that change something ask the user first, so call them directly instead of asking for confirmation in the chat. For anything about Tableboat itself, use these tools rather than shell commands or the registry. Never repeat passwords back.";

/// For the app's Claude pane.
pub fn pane_prompt() -> String {
    format!(
        "You are running inside Tableboat, a desktop database client, in a side pane next to the user's SQL editor. Use the tableboat MCP tools to look at the user's database: get_context (connection, database, open table, current editor SQL), get_schema, and run_read_query (read-only, enforced by the database). execute_sql changes data and always asks the user first; prefer giving the SQL in a ```sql code block so the user can insert or run it from the pane. {APP_TOOLS} Files given without a folder are saved in the user's Downloads folder. Keep answers short and practical."
    )
}

/// For `tb ask` in a terminal.
pub fn cli_prompt() -> String {
    format!(
        "You are answering in a terminal through Tableboat's `tb ask` command. Use the tableboat MCP tools to look at the user's database: get_context (connection and database), get_schema, and run_read_query (read-only, enforced by the database). execute_sql changes data and asks the user in the terminal first; prefer giving SQL in a ```sql code block they can run with `tb query`. {APP_TOOLS} Files given without a folder are saved in the terminal's current folder. Answer in plain text suitable for a terminal; keep it short and practical."
    )
}

/// Tools Claude may use without asking: they only read, or (open_connection) only open a window.
pub const READ_TOOLS: &[&str] = &[
    "mcp__tableboat__get_context",
    "mcp__tableboat__get_schema",
    "mcp__tableboat__run_read_query",
    "mcp__tableboat__list_connections",
    "mcp__tableboat__get_settings",
    "mcp__tableboat__test_connection",
    "mcp__tableboat__open_connection",
    "mcp__tableboat__check_for_updates",
    "mcp__tableboat__list_databases",
    "mcp__tableboat__list_objects",
    "mcp__tableboat__get_definition",
    "mcp__tableboat__get_app_state",
    "mcp__tableboat__list_app_commands",
    "mcp__tableboat__open_object",
    "mcp__tableboat__use_database",
];

/// Tools that change something: each asks the user itself (with a readable summary), whatever
/// Claude Code's own permission settings say, so `approve` lets them through without asking twice.
pub const ASKS_ITSELF: &[&str] = &[
    "mcp__tableboat__execute_sql",
    "mcp__tableboat__update_settings",
    "mcp__tableboat__save_connection",
    "mcp__tableboat__delete_connection",
    "mcp__tableboat__export_connections",
    "mcp__tableboat__import_connections",
    "mcp__tableboat__export_data",
    "mcp__tableboat__dump_database",
    "mcp__tableboat__install_update",
    "mcp__tableboat__duplicate_connection",
    "mcp__tableboat__run_app_command",
    "mcp__tableboat__open_sql",
];

/// Write the MCP config file that points Claude Code at Tableboat's tool server.
pub fn write_mcp_config(path: &Path, port: u16, token: &str, route: &str) -> Result<(), String> {
    let config = json!({
        "mcpServers": {
            "tableboat": {
                "type": "http",
                "url": format!("http://127.0.0.1:{port}/mcp/{route}"),
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    });
    std::fs::write(path, config.to_string()).map_err(|e| e.to_string())
}

/// The arguments shared by every Claude Code run from Tableboat: only Tableboat's MCP server,
/// read tools pre-allowed, every other permission question sent to `approve`, the given model.
pub fn base_args(config_path: &Path, system_prompt: &str, model: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--mcp-config".into(),
        config_path.display().to_string(),
        // Only Tableboat's MCP server: the user's other servers would cost memory and start-up time.
        "--strict-mcp-config".into(),
        "--allowedTools".into(),
        READ_TOOLS.join(" "),
        "--permission-prompt-tool".into(),
        "mcp__tableboat__approve".into(),
        // Ask for everything not pre-allowed, regardless of the user's own Claude Code default
        // mode (e.g. auto or bypass): the questions come to Tableboat.
        "--permission-mode".into(),
        "manual".into(),
        "--append-system-prompt".into(),
        system_prompt.into(),
    ];
    let model = model.trim();
    if !model.is_empty() && model.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]".contains(c)) {
        args.push("--model".into());
        args.push(model.into());
    }
    args
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
