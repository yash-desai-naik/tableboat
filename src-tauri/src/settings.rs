//! The app's settings store and commands (the settings themselves are in `tableboat_core::prefs`).
//!
//! `tb settings set` and Claude (through `tb ask`) can change settings.json while the app runs:
//! the store re-reads the file when it changed on disk, and the windows reload their copy when
//! they get focus or hear `settings:changed`.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;
use tableboat_core::prefs;
use tauri::{AppHandle, Emitter, State};

pub use prefs::Settings;

pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<(Settings, Option<SystemTime>)>,
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl SettingsStore {
    pub fn load() -> Result<Self, String> {
        let path = prefs::path()?;
        let settings = prefs::load(&path);
        prefs::save(&path, &settings)?;
        let stamp = modified(&path);
        Ok(Self { path, current: Mutex::new((settings, stamp)) })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Settings, Option<SystemTime>)> {
        let mut guard = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let now = modified(&self.path);
        if now.is_some() && now != guard.1 {
            *guard = (prefs::load(&self.path), now);
        }
        guard
    }

    pub fn current(&self) -> Settings {
        self.lock().0.clone()
    }

    /// Save new settings and tell every window to reload them.
    pub fn replace(&self, app: &AppHandle, next: Settings) -> Result<(), String> {
        let mut guard = self.lock();
        prefs::save(&self.path, &next)?;
        *guard = (next.clone(), modified(&self.path));
        drop(guard);
        let _ = app.emit("settings:changed", next);
        Ok(())
    }
}

#[tauri::command]
pub fn get_settings(store: State<'_, SettingsStore>) -> Settings {
    store.current()
}

/// Merge `patch` (a partial settings object) into the current settings and save.
#[tauri::command]
pub fn update_settings(store: State<'_, SettingsStore>, patch: Value) -> Result<Settings, String> {
    let mut guard = store.lock();
    let next = guard.0.merged(&patch)?;
    prefs::save(&store.path, &next)?;
    *guard = (next.clone(), modified(&store.path));
    Ok(next)
}
