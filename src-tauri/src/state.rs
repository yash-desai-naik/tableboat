//! Small JSON files the UI keeps between runs (open tabs per connection, last connection),
//! in `%APPDATA%\Tableboat\state\`. Nothing secret is stored here.

use crate::paths::{data_dir, write_atomic};
use tauri::AppHandle;

fn file(app: &AppHandle, name: &str) -> Result<std::path::PathBuf, String> {
    // Names are connection ids or fixed words: never paths.
    if name.is_empty() || name.len() > 80 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Invalid state name.".into());
    }
    let dir = data_dir(app)?.join("state");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{name}.json")))
}

#[tauri::command]
pub fn state_read(app: AppHandle, name: String) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file(&app, &name)?) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn state_write(app: AppHandle, name: String, contents: String) -> Result<(), String> {
    if contents.len() > 20_000_000 {
        return Err("State too large to save.".into());
    }
    write_atomic(&file(&app, &name)?, &contents).map_err(|e| e.to_string())
}

/// The connection open in each extra window (every window but "main", which remembers its own
/// last connection), saved as `state\windows.json` so the next start reopens those windows.
#[derive(Default)]
pub struct WindowList {
    open: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl WindowList {
    /// Record (Some) or forget (None) the connection open in a window, and save the list.
    pub fn set(&self, app: &AppHandle, label: &str, connection_id: Option<&str>) {
        if label == "main" {
            return;
        }
        let ids: Vec<String> = {
            let mut open = self.open.lock().unwrap();
            match connection_id {
                Some(id) => open.insert(label.to_string(), id.to_string()),
                None => open.remove(label),
            };
            open.values().cloned().collect()
        };
        if let Ok(path) = file(app, "windows") {
            let _ = write_atomic(&path, &serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()));
        }
    }
}

/// Connections that were open in extra windows when Tableboat last closed (each listed once).
pub fn saved_windows(app: &AppHandle) -> Vec<String> {
    let Ok(path) = file(app, "windows") else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut ids: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

/// Open settings.json in the user's default editor for .json files (Notepad if none).
#[tauri::command]
pub fn open_settings_file(app: AppHandle) -> Result<String, String> {
    let path = data_dir(&app)?.join("settings.json");
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .status();
    match status {
        Ok(s) if s.success() => Ok(path.display().to_string()),
        _ => {
            std::process::Command::new("notepad.exe").arg(&path).spawn().map_err(|e| e.to_string())?;
            Ok(path.display().to_string())
        }
    }
}
