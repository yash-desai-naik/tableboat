//! Updates: Tableboat checks GitHub for a newer release (at start-up and once a day, unless
//! turned off in Settings, and whenever the user asks), tells every window, and on request
//! downloads the signed installer, verifies it against the public key in tauri.conf.json, runs
//! it (a small progress window, no questions) and restarts on the new version.
//!
//! The feed is `latest.json` on the latest published GitHub release, made by the release
//! workflow; drafts and pre-releases are never offered.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// What the windows show about an available update.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    /// Release notes (Markdown, from CHANGELOG.md).
    pub notes: String,
    pub date: Option<String>,
}

#[derive(Default)]
pub struct UpdateState {
    /// The update found by the last check, ready to install.
    pending: Mutex<Option<Update>>,
    installing: Mutex<bool>,
}

fn info(u: &Update) -> UpdateInfo {
    UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone().unwrap_or_default(),
        date: u.date.map(|d| d.to_string()),
    }
}

/// Ask GitHub for a newer version. `Ok(None)` = up to date.
async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    // TABLEBOAT_UPDATE_URL points a test build at another feed.
    let mut builder = app.updater_builder().timeout(Duration::from_secs(30));
    if let Some(url) = std::env::var("TABLEBOAT_UPDATE_URL").ok().and_then(|u| url::Url::parse(&u).ok()) {
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    let updater = builder.build().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let state = app.state::<UpdateState>();
    let out = found.as_ref().map(info);
    *state.pending.lock().unwrap() = found;
    Ok(out)
}

/// Check now and, if there's an update, show the notice in every window.
pub async fn check_and_announce(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let found = check(app).await?;
    if let Some(i) = &found {
        let _ = app.emit("update:available", i.clone());
    }
    Ok(found)
}

/// "Check for updates" (About box, command palette): always reports the result.
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    check_and_announce(&app).await
}

/// The update found earlier, if any (a window opened later asks this).
#[tauri::command]
pub fn update_status(state: tauri::State<'_, UpdateState>) -> Option<UpdateInfo> {
    state.pending.lock().unwrap().as_ref().map(info)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

/// Download, verify and install the pending update, and start the new version. On Windows the
/// installer takes over; on macOS Tableboat replaces its own app bundle and restarts.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    {
        let mut busy = state.installing.lock().unwrap();
        if *busy {
            return Err("The update is already being installed.".into());
        }
        *busy = true;
    }
    let update = state.pending.lock().unwrap().clone();
    let Some(update) = update else {
        *state.installing.lock().unwrap() = false;
        return Err("No update to install: check for updates first.".into());
    };
    let mut downloaded = 0u64;
    let progress_app = app.clone();
    let done_app = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update:progress", Progress { downloaded, total });
            },
            move || {
                let _ = done_app.emit("update:installing", ());
            },
        )
        .await;
    *state.installing.lock().unwrap() = false;
    if let Err(e) = result {
        return Err(format!("The update couldn't be installed: {e}"));
    }
    // The installer restarts the app on Windows; on macOS the app bundle was replaced in place,
    // so start the new version ourselves.
    #[cfg(not(windows))]
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

/// Check in the background ~20 s after start-up, then once a day, while "Check for updates
/// automatically" is on. A skipped version isn't announced (a manual check still shows it).
pub fn start_background_checks(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let settings = app.state::<crate::settings::SettingsStore>().current();
            if settings.check_updates {
                if let Ok(Some(i)) = check(&app).await {
                    if i.version != settings.skipped_update {
                        let _ = app.emit("update:available", i);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}
