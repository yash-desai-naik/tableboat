//! User settings, stored as `%APPDATA%\Tableboat\settings.json` (shared by the app and `tb`).
//!
//! The file is meant to be editable by hand. Unknown or missing keys fall back to
//! defaults; a file that can't be parsed is moved aside (never silently overwritten).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const ALL_KINDS: [&str; 5] = ["table", "view", "function", "procedure", "trigger"];
const DEFAULT_LIGHT_THEME: &str = "tableboat-light";
const DEFAULT_DARK_THEME: &str = "tableboat-dark";

/// Is `id` a theme of this mode ("light" or "dark")?
fn theme_fits(id: &str, mode: &str) -> bool {
    crate::themes::mode_of(id) == Some(mode)
}

/// Refuse a `lightTheme` / `darkTheme` value that isn't a theme id of that mode, naming the
/// valid ones (rather than quietly falling back to the default, as a hand-edited file does).
fn check_theme_ids(patch: &Value) -> Result<(), String> {
    for (key, mode) in [("lightTheme", "light"), ("darkTheme", "dark")] {
        let Some(asked) = patch.get(key) else { continue };
        let id = asked.as_str().unwrap_or("");
        if theme_fits(id, mode) {
            continue;
        }
        let why = match crate::themes::mode_of(id) {
            Some(other) => format!("\"{id}\" is a {other} theme"),
            None => format!("there is no theme {asked}"),
        };
        let valid = crate::themes::ids_for(mode).join(", ");
        return Err(format!("\"{key}\" must be a {mode} theme id ({why}). {mode} themes: {valid}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// The colour theme used in light mode: a light theme id from `src/themes/themes.json`.
    pub light_theme: String,
    /// The colour theme used in dark mode: a dark theme id from `src/themes/themes.json`.
    pub dark_theme: String,
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
    /// Model for Claude ("" = Claude Code's own default), e.g. "sonnet", "haiku", "opus".
    pub claude_model: String,
    /// Reopen each connection's tabs (and SQL text) when connecting again.
    pub restore_tabs: bool,
    /// At start-up, reconnect the main window to the last connection used.
    pub reopen_last_connection: bool,
    /// How MongoDB results show: "documents" or "table".
    pub mongo_view: String,
    /// How SQL results show: "table" or "json".
    pub sql_view: String,
    /// Look for a new version on GitHub at start-up and once a day.
    pub check_updates: bool,
    /// A version the user chose to skip (no notice for it; a manual check still shows it).
    pub skipped_update: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            light_theme: DEFAULT_LIGHT_THEME.into(),
            dark_theme: DEFAULT_DARK_THEME.into(),
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
            mongo_view: "documents".into(),
            sql_view: "table".into(),
            check_updates: true,
            skipped_update: String::new(),
        }
    }
}

impl Settings {
    /// Clamp values a hand edit could have broken.
    pub fn sanitized(mut self) -> Self {
        let d = Settings::default();
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = d.theme;
        }
        if !theme_fits(&self.light_theme, "light") {
            self.light_theme = d.light_theme;
        }
        if !theme_fits(&self.dark_theme, "dark") {
            self.dark_theme = d.dark_theme;
        }
        if !matches!(self.density.as_str(), "compact" | "default" | "comfortable") {
            self.density = d.density;
        }
        if !matches!(self.mongo_view.as_str(), "documents" | "table") {
            self.mongo_view = d.mongo_view;
        }
        if !matches!(self.sql_view.as_str(), "table" | "json") {
            self.sql_view = d.sql_view;
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

    /// Merge a partial settings object (`{"theme": "dark"}`) into these settings.
    /// Unknown keys and wrong types are refused; values out of range are clamped.
    pub fn merged(&self, patch: &Value) -> Result<Settings, String> {
        let mut merged = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let (Some(target), Some(changes)) = (merged.as_object_mut(), patch.as_object()) else {
            return Err("settings patch must be an object".into());
        };
        for (key, value) in changes {
            if !target.contains_key(key) {
                return Err(format!("unknown setting \"{key}\""));
            }
            target.insert(key.clone(), value.clone());
        }
        let next: Settings = serde_json::from_value(merged).map_err(|e| format!("invalid setting: {e}"))?;
        Ok(next.sanitized())
    }
}

/// What each setting accepts (for Claude and `tb settings`).
pub const HELP: &str = "theme: \"system\" | \"light\" | \"dark\" (\"system\" follows your system setting)
lightTheme: colour theme used in light mode: \"tableboat-light\" (default) | \"catppuccin-latte\" | \"github-light\" | \"one-light\" | \"rose-pine-dawn\" | \"gruvbox-light\" | \"kanagawa-lotus\" | \"hc-light\"
darkTheme: colour theme used in dark mode: \"tableboat-dark\" (default) | \"catppuccin-mocha\" | \"tokyo-night\" | \"github-dark\" | \"one-dark\" | \"rose-pine\" | \"gruvbox-dark\" | \"kanagawa-wave\" | \"nord\" | \"dracula\" | \"hc-dark\"
density: \"compact\" | \"default\" | \"comfortable\"
uiZoom: 0.5 to 3.0 (whole window zoom)
editorFontSize, gridFontSize: 8 to 40
wordWrap (SQL editor), showSidebar, showAiPane (the Claude pane): true | false
sidebarWidth: 160 to 900; aiPaneWidth: 260 to 1200; resultsHeight: 80 to 4000 (pixels)
pageSize: 10 to 100000 (rows per page when a table is opened)
sidebarKinds: any of [\"table\", \"view\", \"function\", \"procedure\", \"trigger\"]
maxRows: 100 to 5000000 (most rows kept from one editor query)
stopOnError: stop a script at the first failing statement
sortDescendingFirst: a header click sorts descending first
claudeModel: \"\" (Claude Code's default) | \"sonnet\" | \"opus\" | \"haiku\" | a full model id
restoreTabs: reopen a connection's tabs when connecting again
reopenLastConnection: reconnect to the last connection at start-up
mongoView: \"documents\" | \"table\" (how MongoDB results show)
sqlView: \"table\" | \"json\" (how SQL results show)
checkUpdates: look for a new Tableboat version at start-up and once a day
skippedUpdate: a version not to be told about again (\"\" = none)";

/// One changed setting: key, old value, new value.
pub type Change = (String, Value, Value);

impl Settings {
    /// Apply `patch` strictly: unknown keys, wrong types and values that would be clamped are
    /// refused (the app's own controls clamp quietly; a typed or asked-for value should not).
    /// Returns the new settings and what actually changes.
    pub fn checked_change(&self, patch: &Value) -> Result<(Settings, Vec<Change>), String> {
        check_theme_ids(patch)?;
        let next = self.merged(patch)?;
        let before = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let after = serde_json::to_value(&next).map_err(|e| e.to_string())?;
        let same = |a: &Value, b: &Value| match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => a == b,
        };
        let mut changes = Vec::new();
        for (key, asked) in patch.as_object().into_iter().flatten() {
            let now = &after[key];
            if !same(asked, now) {
                return Err(format!("\"{key}\" can't be {asked} (allowed values would make it {now}). Allowed:
{HELP}"));
            }
            if !same(&before[key], now) {
                changes.push((key.clone(), before[key].clone(), now.clone()));
            }
        }
        Ok((next, changes))
    }
}

/// `settings.json` in the data folder.
pub fn path() -> Result<PathBuf, String> {
    Ok(crate::paths::data_dir()?.join("settings.json"))
}

/// Read the settings (defaults when there's no file). A file that can't be parsed is moved
/// aside as `settings.invalid-<time>.json` and defaults are used.
pub fn load(path: &Path) -> Settings {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(s) => s.sanitized(),
            Err(_) => {
                let invalid = path.with_file_name(format!("settings.invalid-{}.json", crate::paths::now_secs()));
                let _ = fs::rename(path, invalid);
                Settings::default()
            }
        },
        Err(_) => Settings::default(),
    }
}

/// Save atomically (a crash mid-write can't leave a half-written file).
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    crate::paths::write_atomic(path, &text).map_err(|e| format!("could not save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_file(text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("tableboat-prefs-{}.json", uuid::Uuid::new_v4().simple()));
        fs::write(&p, text).unwrap();
        p
    }

    #[test]
    fn theme_ids_default_to_tableboat() {
        let d = Settings::default();
        assert_eq!((d.light_theme.as_str(), d.dark_theme.as_str()), ("tableboat-light", "tableboat-dark"));
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["lightTheme"], "tableboat-light");
        assert_eq!(v["darkTheme"], "tableboat-dark");
    }

    #[test]
    fn old_files_and_bad_theme_ids_load_with_defaults() {
        // A settings file from before the theme keys existed.
        let p = temp_file(r#"{ "theme": "dark", "editorFontSize": 15 }"#);
        let s = load(&p);
        assert_eq!((s.theme.as_str(), s.editor_font_size), ("dark", 15));
        assert_eq!((s.light_theme.as_str(), s.dark_theme.as_str()), ("tableboat-light", "tableboat-dark"));
        // Hand-edited ids that don't exist or have the wrong mode fall back; good ones stay.
        fs::write(&p, r#"{ "lightTheme": "catppuccin-mocha", "darkTheme": "nope" }"#).unwrap();
        let s = load(&p);
        assert_eq!((s.light_theme.as_str(), s.dark_theme.as_str()), ("tableboat-light", "tableboat-dark"));
        fs::write(&p, r#"{ "lightTheme": "github-light", "darkTheme": "tokyo-night" }"#).unwrap();
        let s = load(&p);
        assert_eq!((s.light_theme.as_str(), s.dark_theme.as_str()), ("github-light", "tokyo-night"));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn theme_ids_are_checked_by_mode() {
        let s = Settings::default();
        let (next, changes) = s.checked_change(&json!({ "darkTheme": "catppuccin-mocha" })).unwrap();
        assert_eq!(next.dark_theme, "catppuccin-mocha");
        assert_eq!(changes, vec![("darkTheme".to_string(), json!("tableboat-dark"), json!("catppuccin-mocha"))]);
        assert_eq!(s.checked_change(&json!({ "lightTheme": "catppuccin-latte" })).unwrap().0.light_theme, "catppuccin-latte");

        let err = s.checked_change(&json!({ "darkTheme": "catppuccin-latte" })).unwrap_err();
        assert!(err.contains("\"catppuccin-latte\" is a light theme") && err.contains("tableboat-dark, catppuccin-mocha"), "{err}");
        assert!(!err.contains("tableboat-light"), "only dark ids are listed: {err}");
        let err = s.checked_change(&json!({ "lightTheme": "solarized" })).unwrap_err();
        assert!(err.contains("no theme \"solarized\"") && err.contains("tableboat-light, catppuccin-latte"), "{err}");
        assert!(s.checked_change(&json!({ "lightTheme": 5 })).is_err());
    }

    #[test]
    fn help_lists_every_theme_id() {
        for (key, mode) in [("lightTheme", "light"), ("darkTheme", "dark")] {
            let line = HELP.lines().find(|l| l.starts_with(&format!("{key}:"))).expect("a HELP line");
            let listed: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
            assert_eq!(listed, crate::themes::ids_for(mode), "HELP's {key} line must list the {mode} themes of themes.json");
        }
    }
}
