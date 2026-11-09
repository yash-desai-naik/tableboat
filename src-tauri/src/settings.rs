//! User settings, stored as `%APPDATA%\Tableboat\settings.json`.
//!
//! The file is meant to be editable by hand. Unknown or missing keys fall back to
//! defaults; a file that can't be parsed is moved aside (never silently overwritten).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};

const ALL_KINDS: [&str; 5] = ["table", "view", "function", "procedure", "trigger"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// "compact" | "default" | "comfortable"
    pub density: String,
    /// Whole-UI zoom factor (Ctrl+= / Ctrl+-).
    pub ui_zoom: f64,
    pub editor_font_size: u32,
    pub grid_font_size: u32,
    pub word_wrap: bool,
    pub show_sidebar: bool,
    pub show_ai_pane: bool,
    pub sidebar_width: u32,
    pub ai_pane_width: u32,
    /// Rows fetched per page when a table is opened.
    pub page_size: u32,
    /// Object types shown in the sidebar: "table", "view", "function", "procedure", "trigger".
    pub sidebar_kinds: Vec<String>,
    /// Most rows kept from one editor query (the rest are not downloaded).
    pub max_rows: u32,
    /// Stop running further statements after one fails.
    pub stop_on_error: bool,
    /// Height of the results pane under the SQL editor, in pixels.
    pub results_height: u32,
    /// Header click sorts descending first (TablePlus) instead of ascending.
    pub sort_descending_first: bool,
    /// Model for the Claude pane ("" = Claude Code's own default), e.g. "sonnet", "haiku", "opus".
    pub claude_model: String,
    /// Reopen each connection's tabs (and SQL text) when connecting again.
    pub restore_tabs: bool,
    /// At start-up, reconnect the main window to the last connection used.
    pub reopen_last_connection: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            density: "default".into(),
            ui_zoom: 1.0,
            editor_font_size: 13,
            grid_font_size: 12,
            word_wrap: false,
            show_sidebar: true,
            show_ai_pane: false,
            sidebar_width: 260,
            ai_pane_width: 380,
            page_size: 300,
            sidebar_kinds: ALL_KINDS.iter().map(|k| k.to_string()).collect(),
            max_rows: 10_000,
            stop_on_error: true,
            results_height: 320,
            sort_descending_first: true,
            claude_model: String::new(),
            restore_tabs: true,
            reopen_last_connection: true,
        }
    }
}

impl Settings {
    /// Clamp values a hand edit could have broken.
    fn sanitized(mut self) -> Self {
        let d = Settings::default();
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = d.theme;
        }
        if !matches!(self.density.as_str(), "compact" | "default" | "comfortable") {
            self.density = d.density;
        }
        if !self.ui_zoom.is_finite() {
            self.ui_zoom = d.ui_zoom;
        }
        self.ui_zoom = self.ui_zoom.clamp(0.5, 3.0);
        self.editor_font_size = self.editor_font_size.clamp(8, 40);
        self.grid_font_size = self.grid_font_size.clamp(8, 40);
        self.sidebar_width = self.sidebar_width.clamp(160, 900);
        self.ai_pane_width = self.ai_pane_width.clamp(260, 1200);
        self.page_size = self.page_size.clamp(10, 100_000);
        self.sidebar_kinds.retain(|k| ALL_KINDS.contains(&k.as_str()));
        self.sidebar_kinds.dedup();
        self.max_rows = self.max_rows.clamp(100, 5_000_000);
        self.results_height = self.results_height.clamp(80, 4000);
        self
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load(app: &AppHandle) -> Result<Self, Box<dyn std::error::Error>> {
        let dir = app.path().config_dir()?.join("Tableboat");
        fs::create_dir_all(&dir)?;
        let path = dir.join("settings.json");

        let settings = match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => s.sanitized(),
                Err(_) => {
                    // Keep the user's broken file for them to fix, and start from defaults.
                    let stamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let _ = fs::rename(&path, dir.join(format!("settings.invalid-{stamp}.json")));
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        };

        write_atomic(&path, &settings)?;
        Ok(Self { path, current: Mutex::new(settings) })
    }
}

/// Write to a temporary file, then rename over the target, so a crash mid-write
/// can't leave a half-written settings file.
fn write_atomic(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

impl SettingsStore {
    pub fn current(&self) -> Settings {
        self.current.lock().expect("settings lock").clone()
    }
}

#[tauri::command]
pub fn get_settings(store: State<'_, SettingsStore>) -> Settings {
    store.current.lock().expect("settings lock").clone()
}

/// Merge `patch` (a partial settings object) into the current settings and save.
#[tauri::command]
pub fn update_settings(store: State<'_, SettingsStore>, patch: Value) -> Result<Settings, String> {
    let mut current = store.current.lock().map_err(|e| e.to_string())?;

    let mut merged = serde_json::to_value(&*current).map_err(|e| e.to_string())?;
    match (merged.as_object_mut(), patch.as_object()) {
        (Some(target), Some(changes)) => {
            for (key, value) in changes {
                target.insert(key.clone(), value.clone());
            }
        }
        _ => return Err("settings patch must be an object".into()),
    }

    let next: Settings = serde_json::from_value(merged)
        .map_err(|e| format!("invalid setting: {e}"))?;
    let next = next.sanitized();

    write_atomic(&store.path, &next).map_err(|e| format!("could not save settings: {e}"))?;
    *current = next.clone();
    Ok(next)
}
