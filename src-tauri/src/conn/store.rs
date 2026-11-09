//! Saved connections in `%APPDATA%\Tableboat\connections.json` (no passwords).

use super::model::Connection;
use crate::paths::{now_secs, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    version: u32,
    connections: Vec<Connection>,
}

pub struct ConnectionStore {
    path: PathBuf,
    items: Mutex<Vec<Connection>>,
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
        Ok(Self { path, items: Mutex::new(items) })
    }

    fn save(&self, items: &[Connection]) -> Result<(), String> {
        let file = FileFormat { version: 1, connections: items.to_vec() };
        let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &text).map_err(|e| format!("Could not save connections: {e}"))
    }

    pub fn list(&self) -> Vec<Connection> {
        self.items.lock().expect("connections lock").clone()
    }

    pub fn get(&self, id: &str) -> Option<Connection> {
        self.items.lock().expect("connections lock").iter().find(|c| c.id == id).cloned()
    }

    /// Insert or replace by id; assigns an id and timestamps as needed.
    pub fn upsert(&self, mut conn: Connection) -> Result<Connection, String> {
        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        let now = now_secs();
        if conn.id.is_empty() {
            conn.id = uuid::Uuid::new_v4().to_string();
            conn.created_at = now;
        }
        conn.updated_at = now;
        match items.iter_mut().find(|c| c.id == conn.id) {
            Some(existing) => {
                conn.created_at = existing.created_at;
                *existing = conn.clone();
            }
            None => {
                if conn.created_at == 0 {
                    conn.created_at = now;
                }
                items.push(conn.clone());
            }
        }
        self.save(&items)?;
        Ok(conn)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut items = self.items.lock().map_err(|e| e.to_string())?;
        items.retain(|c| c.id != id);
        self.save(&items)
    }
}
