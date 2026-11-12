//! Saved connections in `%APPDATA%\Tableboat\connections.json` (no passwords).
//!
//! The app and `tb` (or Claude through either) may both change the file: each access first
//! re-reads it when it changed on disk, so neither overwrites the other's changes.

use super::model::Connection;
use crate::paths::{now_secs, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    version: u32,
    connections: Vec<Connection>,
}

pub struct ConnectionStore {
    path: PathBuf,
    items: Mutex<Items>,
}

struct Items {
    list: Vec<Connection>,
    /// The file's modification time when it was last read or written.
    modified: Option<SystemTime>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl ConnectionStore {
    pub fn load(dir: &std::path::Path) -> Result<Self, String> {
        let path = dir.join("connections.json");
        let items = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<FileFormat>(&text) {
                Ok(f) => f.connections,
                Err(e) => {
                    // Never overwrite a file we couldn't read: move it aside for the user.
                    let aside = dir.join(format!("connections.unreadable-{}.json", now_secs()));
                    std::fs::rename(&path, &aside).map_err(|err| {
                        format!("connections.json is invalid ({e}) and could not be moved aside: {err}")
                    })?;
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let modified = modified(&path);
        Ok(Self { path, items: Mutex::new(Items { list: items, modified }) })
    }

    /// The list, re-read first if another program changed the file (an unreadable file is
    /// left alone and the list in memory kept).
    fn items(&self) -> MutexGuard<'_, Items> {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let now = modified(&self.path);
        if now.is_some() && now != items.modified {
            if let Some(f) = std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str::<FileFormat>(&t).ok()) {
                items.list = f.connections;
                items.modified = now;
            }
        }
        items
    }

    fn save(&self, items: &mut Items) -> Result<(), String> {
        let file = FileFormat { version: 1, connections: items.list.clone() };
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &text).map_err(|e| format!("Could not save connections: {e}"))?;
        items.modified = modified(&self.path);
        Ok(())
    }

    pub fn list(&self) -> Vec<Connection> {
        self.items().list.clone()
    }

    pub fn get(&self, id: &str) -> Option<Connection> {
        self.items().list.iter().find(|c| c.id == id).cloned()
    }

    /// Insert or replace by id; assigns an id and timestamps as needed.
    pub fn upsert(&self, mut conn: Connection) -> Result<Connection, String> {
        let mut items = self.items();
        let now = now_secs();
        if conn.id.is_empty() {
            conn.id = uuid::Uuid::new_v4().to_string();
            conn.created_at = now;
        }
        conn.updated_at = now;
        match items.list.iter_mut().find(|c| c.id == conn.id) {
            Some(existing) => {
                conn.created_at = existing.created_at;
                *existing = conn.clone();
            }
            None => {
                if conn.created_at == 0 {
                    conn.created_at = now;
                }
                items.list.push(conn.clone());
            }
        }
        self.save(&mut items)?;
        Ok(conn)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut items = self.items();
        items.list.retain(|c| c.id != id);
        self.save(&mut items)
    }
}
