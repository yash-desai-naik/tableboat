//! Connection operations shared by the app's commands and the `tb` command line.

use super::model::{Connection, ConnectionSummary, Engine, Secrets};
use super::store::ConnectionStore;
use super::{secrets, url};
use crate::paths::{data_dir, write_atomic};
use std::path::Path;

/// The connection plus which secrets are stored for it.
pub fn summary(conn: Connection) -> ConnectionSummary {
    let has = |field| secrets::get(&conn.id, field).ok().flatten().is_some();
    ConnectionSummary {
        has_password: has(secrets::PASSWORD),
        has_ssh_password: has(secrets::SSH_PASSWORD),
        has_ssh_passphrase: has(secrets::SSH_PASSPHRASE),
        connection: conn,
    }
}

pub fn validate(conn: &Connection) -> Result<(), String> {
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

/// The SSH known-hosts file (trust on first use).
pub fn known_hosts() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("known_hosts"))
}

/// Save a new or edited connection. `secrets`: `None` fields stay as stored, `Some("")` removes.
pub fn save(store: &ConnectionStore, connection: Connection, mut secrets: Secrets) -> Result<ConnectionSummary, String> {
    validate(&connection)?;
    if !connection.save_password {
        // "Ask every time": never keep the database password.
        secrets.password = Some(String::new());
    }
    let saved = store.upsert(connection)?;
    secrets::apply(&saved.id, &secrets)?;
    Ok(summary(saved))
}

pub fn delete(store: &ConnectionStore, id: &str) -> Result<(), String> {
    store.delete(id)?;
    secrets::delete_all(id)
}

pub fn duplicate(store: &ConnectionStore, id: &str) -> Result<ConnectionSummary, String> {
    let original = store.get(id).ok_or("That connection no longer exists.")?;
    let stored = secrets::load(id)?;
    let mut copy = original.clone();
    copy.id = String::new();
    copy.name = format!("{} copy", original.display_name());
    let saved = store.upsert(copy)?;
    secrets::apply(&saved.id, &stored)?;
    Ok(summary(saved))
}

/// The connection as a URL (passwords only when asked for).
pub fn to_url(store: &ConnectionStore, id: &str, include_passwords: bool) -> Result<String, String> {
    let conn = store.get(id).ok_or("That connection no longer exists.")?;
    let stored = if include_passwords { secrets::load(id)? } else { Secrets::default() };
    Ok(url::to_url(&conn, &stored, include_passwords))
}

/// A saved connection by id, exact name (any case), or a part of its name that only one has.
pub fn find(store: &ConnectionStore, query: &str) -> Result<Connection, String> {
    let all = store.list();
    if let Some(c) = all.iter().find(|c| c.id == query) {
        return Ok(c.clone());
    }
    let q = query.trim().to_lowercase();
    if let Some(c) = all.iter().find(|c| c.display_name().to_lowercase() == q) {
        return Ok(c.clone());
    }
    let matches: Vec<&Connection> = all.iter().filter(|c| c.display_name().to_lowercase().contains(&q)).collect();
    let names = |list: &[&Connection]| list.iter().map(|c| format!("\"{}\"", c.display_name())).collect::<Vec<_>>().join(", ");
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] if all.is_empty() => Err(format!("No saved connection matches \"{query}\": there are no saved connections.")),
        [] => Err(format!("No saved connection matches \"{query}\". Saved: {}", names(&all.iter().collect::<Vec<_>>()))),
        many => Err(format!("\"{query}\" matches several connections: {}. Be more specific.", names(many))),
    }
}

/// Write every saved connection as one URL per line. Returns how many.
pub fn export_file(store: &ConnectionStore, path: &Path, include_passwords: bool) -> Result<usize, String> {
    export_some(store, None, path, include_passwords)
}

/// Write the given connections (by id; `None` = all) as one URL per line. Returns how many.
pub fn export_some(store: &ConnectionStore, ids: Option<&[String]>, path: &Path, include_passwords: bool) -> Result<usize, String> {
    let mut lines = vec![
        "# Tableboat connections – one connection URL per line.".to_string(),
        if include_passwords {
            "# This file CONTAINS PASSWORDS. Keep it private.".to_string()
        } else {
            "# Passwords are not included.".to_string()
        },
    ];
    let all: Vec<Connection> = store.list().into_iter().filter(|c| ids.is_none_or(|ids| ids.contains(&c.id))).collect();
    for conn in &all {
        let stored = if include_passwords { secrets::load(&conn.id)? } else { Secrets::default() };
        lines.push(url::to_url(conn, &stored, include_passwords));
    }
    write_atomic(path, &(lines.join("\r\n") + "\r\n")).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(all.len())
}

/// Read a file of connection URLs (one per line, `#` comments allowed) and save each one.
/// Returns how many were imported and the lines that couldn't be read.
pub fn import_file(store: &ConnectionStore, path: &Path) -> Result<(usize, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
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
    Ok((imported, errors))
}
