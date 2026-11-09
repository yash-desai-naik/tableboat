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
