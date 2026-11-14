//! Small JSON files the UI keeps between runs (open tabs per connection, last connection),
//! in `<data dir>/state/`. Nothing secret is stored here.

use crate::paths::{data_dir, write_atomic};

fn file(name: &str) -> Result<std::path::PathBuf, String> {
    // Names are connection ids or fixed words: never paths.
    if name.is_empty() || name.len() > 80 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Invalid state name.".into());
    }
    let dir = data_dir()?.join("state");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!("{name}.json")))
}

#[tauri::command]
pub fn state_read(name: String) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file(&name)?) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn state_write(name: String, contents: String) -> Result<(), String> {
    if contents.len() > 20_000_000 {
        return Err("State too large to save.".into());
    }
    write_atomic(&file(&name)?, &contents).map_err(|e| e.to_string())
}

/// The connection open in each extra window (every window but "main", which remembers its own
/// last connection), saved as `state\windows.json` so the next start reopens those windows.
#[derive(Default)]
pub struct WindowList {
    open: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl WindowList {
    /// Record (Some) or forget (None) the connection open in a window, and save the list.
    pub fn set(&self, label: &str, connection_id: Option<&str>) {
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
        if let Ok(path) = file("windows") {
            let _ = write_atomic(&path, &serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into()));
        }
    }
}

/// Connections that were open in extra windows when Tableboat last closed (each listed once).
pub fn saved_windows() -> Vec<String> {
    let Ok(path) = file("windows") else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut ids: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

/// Where Tableboat keeps things, for the About box.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    /// Settings, saved connections (no passwords), tab state.
    data_dir: String,
    /// Claude Code projects, one folder per connection.
    sessions_dir: String,
    /// The running program.
    exe: String,
}

#[tauri::command]
pub fn app_paths() -> Result<AppPaths, String> {
    Ok(AppPaths {
        data_dir: data_dir()?.display().to_string(),
        sessions_dir: crate::ai::sessions_base().display().to_string(),
        exe: std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
    })
}

/// Open a file or folder with the desktop: Explorer, Finder or xdg-open.
fn open_with_desktop(arg: &std::path::Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program).arg(arg).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Open one of Tableboat's folders in the file manager ("data" or "sessions"); created if missing.
#[tauri::command]
pub fn open_app_folder(which: String) -> Result<(), String> {
    let dir = match which.as_str() {
        "data" => data_dir()?,
        "sessions" => crate::ai::sessions_base(),
        _ => return Err("Unknown folder.".into()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open_with_desktop(&dir)
}

/// Tableboat's releases on GitHub, in the default browser (a fixed address, nothing from the page).
#[tauri::command]
pub fn open_releases_page() -> Result<(), String> {
    open_with_desktop(std::path::Path::new("https://github.com/yash-desai-naik/tableboat/releases"))
}

/// Open settings.json in the user's default editor for .json files (Notepad / TextEdit if none).
#[tauri::command]
pub fn open_settings_file() -> Result<String, String> {
    let path = data_dir()?.join("settings.json");
    #[cfg(windows)]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", &path.to_string_lossy()])
        .status();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg("-t").arg(&path).status();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(&path).status();
    match status {
        Ok(s) if s.success() => Ok(path.display().to_string()),
        _ => {
            #[cfg(windows)]
            std::process::Command::new("notepad.exe").arg(&path).spawn().map_err(|e| e.to_string())?;
            #[cfg(not(windows))]
            open_with_desktop(&path)?;
            Ok(path.display().to_string())
        }
    }
}
