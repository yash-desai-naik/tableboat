//! The app's side of the MCP tool server: one host per window. Permission questions become
//! Allow / Deny cards in that window's Claude pane.

use super::ClaudeHub;
use crate::db::{self, Sessions};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tableboat_core::claude::AiContext;
use tableboat_core::conn::store::ConnectionStore;
use tableboat_core::mcp::{Host, Resolver};
use tableboat_core::prefs::Settings;
use tauri::{AppHandle, Emitter, Manager};

pub struct GuiHost {
    app: AppHandle,
    window: String,
    runtime: tokio::runtime::Handle,
}

/// Requests to `/mcp/<window>` are served for that window.
pub fn resolver(app: AppHandle, runtime: tokio::runtime::Handle) -> Resolver {
    Arc::new(move |window: &str| {
        let host: Arc<dyn Host> = Arc::new(GuiHost { app: app.clone(), window: window.to_string(), runtime: runtime.clone() });
        Some(host)
    })
}

impl Host for GuiHost {
    fn context(&self) -> AiContext {
        self.app.state::<ClaudeHub>().context(&self.window)
    }
    fn sessions(&self) -> &Sessions {
        self.app.state::<Sessions>().inner()
    }
    fn runs(&self) -> &db::query::Runs {
        self.app.state::<db::query::Runs>().inner()
    }
    fn connections(&self) -> &ConnectionStore {
        self.app.state::<ConnectionStore>().inner()
    }
    fn runtime(&self) -> &tokio::runtime::Handle {
        &self.runtime
    }
    fn owner(&self) -> String {
        self.window.clone()
    }
    fn linked(&self) -> Vec<String> {
        self.app.state::<ClaudeHub>().linked(&self.window)
    }
    fn linked_session(&self, connection_id: &str) -> Option<String> {
        self.app.state::<ClaudeHub>().linked_session(&self.window, connection_id)
    }
    fn set_linked_session(&self, connection_id: &str, session_id: &str) {
        self.app.state::<ClaudeHub>().set_linked_session(&self.window, connection_id, session_id);
    }
    fn how_to_link(&self) -> &'static str {
        "typing @ in the chat input"
    }

    fn settings(&self) -> Result<Settings, String> {
        Ok(self.app.state::<crate::settings::SettingsStore>().current())
    }
    fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        self.app.state::<crate::settings::SettingsStore>().replace(&self.app, settings.clone())
    }
    fn connections_changed(&self) {
        let _ = self.app.emit("connections:changed", ());
    }
    fn open_in_app(&self, connection_id: &str) -> Result<String, String> {
        crate::commands::open_window(&self.app, Some(connection_id.to_string()))?;
        Ok("Opened it in a new Tableboat window.".into())
    }
    fn files_dir(&self) -> std::path::PathBuf {
        dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir)
    }

    /// Show an Allow / Deny card in the window's Claude pane and wait for the answer.
    fn ask(&self, tool: &str, input: Value) -> Result<bool, String> {
        let hub = self.app.state::<ClaudeHub>();
        if hub.is_always_allowed(&self.window, tool) {
            return Ok(true);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        hub.approvals.lock().unwrap().insert(id.clone(), tx);
        let _ = self.app.emit_to(&self.window, "claude:permission", json!({ "id": id, "toolName": tool, "input": input }));
        match rx.recv_timeout(Duration::from_secs(15 * 60)) {
            Ok((allowed, always)) => {
                if allowed && always {
                    hub.allow_always(&self.window, tool);
                }
                Ok(allowed)
            }
            Err(_) => {
                hub.approvals.lock().unwrap().remove(&id);
                Ok(false)
            }
        }
    }
}
