//! Saved connection definitions. Passwords are never part of these structs on disk;
//! they live in Windows Credential Manager (see `secrets.rs`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Mysql,
    Mariadb,
    Postgres,
    Sqlite,
    Mongodb,
}

impl Engine {
    pub fn default_port(self) -> Option<u16> {
        match self {
            Engine::Mysql | Engine::Mariadb => Some(3306),
            Engine::Postgres => Some(5432),
            Engine::Mongodb => Some(27017),
            Engine::Sqlite => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Mysql => "MySQL",
            Engine::Mariadb => "MariaDB",
            Engine::Postgres => "PostgreSQL",
            Engine::Sqlite => "SQLite",
            Engine::Mongodb => "MongoDB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disabled,
    #[default]
    Preferred,
    Required,
    VerifyCa,
    VerifyIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SslConfig {
    pub mode: SslMode,
    pub ca_file: Option<String>,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    #[default]
    Password,
    Key,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SshConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub key_file: Option<String>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self { enabled: false, host: String::new(), port: 22, user: String::new(), auth: SshAuth::Password, key_file: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Connection {
    pub id: String,
    /// The nickname shown everywhere.
    pub name: String,
    pub engine: Engine,
    /// `#rrggbb`
    pub color: String,
    /// "local", "development", "testing", "staging", "production" or a custom label.
    pub tag: Option<String>,
    pub host: String,
    /// `None` means the engine's default port.
    pub port: Option<u16>,
    pub user: String,
    pub database: String,
    /// SQLite database file.
    pub file: Option<String>,
    pub ssl: SslConfig,
    pub ssh: SshConfig,
    /// When false, the password is asked for on every connect.
    pub save_password: bool,
    /// Engine-specific URL options kept as-is (e.g. MongoDB `authSource`, `replicaSet`).
    pub options: BTreeMap<String, String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            engine: Engine::Mysql,
            color: "#6b7280".into(),
            tag: None,
            host: "127.0.0.1".into(),
            port: None,
            user: String::new(),
            database: String::new(),
            file: None,
            ssl: SslConfig::default(),
            ssh: SshConfig::default(),
            save_password: true,
            options: BTreeMap::new(),
            created_at: 0,
            updated_at: 0,
        }
    }
}

impl Connection {
    pub fn effective_port(&self) -> u16 {
        self.port.or(self.engine.default_port()).unwrap_or(0)
    }

    /// A readable fallback name when the user didn't give one.
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.trim().to_string();
        }
        match self.engine {
            Engine::Sqlite => self
                .file
                .as_deref()
                .and_then(|f| std::path::Path::new(f).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "SQLite".into()),
            _ if !self.database.is_empty() => format!("{}@{}", self.database, self.host),
            _ => self.host.clone(),
        }
    }
}

/// Secret values travelling with a connection between the UI and Rust.
/// `None` = leave the stored value unchanged; `Some("")` = remove it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Secrets {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
}

/// What the connection list shows: the connection plus which secrets are stored.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    #[serde(flatten)]
    pub connection: Connection,
    pub has_password: bool,
    pub has_ssh_password: bool,
    pub has_ssh_passphrase: bool,
}
