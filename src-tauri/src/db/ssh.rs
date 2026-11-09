//! SSH tunnels: a local port on 127.0.0.1 forwarded through the SSH server to the
//! database host. Supports password, private key (OpenSSH / PEM, RSA / ECDSA / Ed25519,
//! with passphrase) and agents (Windows OpenSSH agent pipe, which 1Password also uses,
//! then Pageant).
//!
//! Host keys are trusted on first use and remembered in `%APPDATA%\Tableboat\known_hosts`;
//! a changed key is refused.

use crate::conn::model::{Secrets, SshAuth, SshConfig};
use crate::paths::expand_home;
use russh::client;
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct Tunnel {
    pub local_port: u16,
    task: JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Stops accepting and drops the SSH session (closing the connection).
        self.task.abort();
    }
}

struct Client {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    rejection: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key,
            PublicKeyOrCertificate::Certificate(_) => {
                *self.rejection.lock().unwrap() =
                    Some("The SSH server uses a host certificate, which Tableboat doesn't support yet.".into());
                return Ok(false);
            }
        };
        match russh::keys::check_known_hosts_path(&self.host, self.port, key, &self.known_hosts) {
            Ok(true) => Ok(true),
            Ok(false) => {
                // First connection to this server: remember its key.
                let _ = russh::keys::known_hosts::learn_known_hosts_path(&self.host, self.port, key, &self.known_hosts);
                Ok(true)
            }
            Err(_) => {
                *self.rejection.lock().unwrap() = Some(format!(
                    "The SSH host key for {}:{} has changed since the last connection. This can mean someone is \
                     intercepting the connection. If the server was reinstalled, remove its line from {}.",
                    self.host,
                    self.port,
                    self.known_hosts.display()
                ));
                Ok(false)
            }
        }
    }
}

pub async fn open(
    cfg: &SshConfig,
    secrets: &Secrets,
    remote_host: &str,
    remote_port: u16,
    known_hosts: PathBuf,
) -> Result<Tunnel, String> {
    if cfg.host.trim().is_empty() {
        return Err("Enter the SSH server.".into());
    }
    if cfg.user.trim().is_empty() {
        return Err("Enter the SSH user.".into());
    }
    let target = format!("{}:{}", cfg.host, cfg.port);

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let rejection = Arc::new(Mutex::new(None));
    let handler = Client {
        host: cfg.host.clone(),
        port: cfg.port,
        known_hosts,
        rejection: rejection.clone(),
    };

    let mut session = match tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(config, (cfg.host.as_str(), cfg.port), handler),
    )
    .await
    {
        Err(_) => return Err(format!("Timed out after 15 s connecting to the SSH server {target}.")),
        Ok(Err(e)) => {
            let reason = rejection.lock().unwrap().take();
            return Err(reason.unwrap_or_else(|| format!("Could not connect to the SSH server {target}: {e}")));
        }
        Ok(Ok(s)) => s,
    };

    let user = cfg.user.trim().to_string();
    let authenticated = match cfg.auth {
        SshAuth::Password => session
            .authenticate_password(user.clone(), secrets.ssh_password.clone().unwrap_or_default())
            .await
            .map_err(|e| format!("SSH login failed: {e}"))?
            .success(),
        SshAuth::Key => {
            let path = key_path(cfg)?;
            let key = russh::keys::load_secret_key(&path, secrets.ssh_passphrase.as_deref().filter(|p| !p.is_empty()))
                .map_err(|e| key_error(&path, secrets, e))?;
            let hash = session.best_supported_rsa_hash().await.map_err(|e| e.to_string())?.flatten();
            session
                .authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(|e| format!("SSH key login failed: {e}"))?
                .success()
        }
        SshAuth::Agent => agent_login(&mut session, &user).await?,
    };
    if !authenticated {
        return Err(format!(
            "The SSH server {target} rejected the login for \"{user}\". Check the user and the {}.",
            match cfg.auth {
                SshAuth::Password => "password",
                SshAuth::Key => "key (and that its public half is on the server)",
                SshAuth::Agent => "keys loaded in your SSH agent",
            }
        ));
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("Could not open a local port for the SSH tunnel: {e}"))?;
    let local_port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let session = Arc::new(session);
    let remote_host = remote_host.to_string();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, peer)) = listener.accept().await else { break };
            let session = session.clone();
            let remote_host = remote_host.clone();
            tokio::spawn(async move {
                match session
                    .channel_open_direct_tcpip(remote_host, remote_port as u32, "127.0.0.1", peer.port() as u32)
                    .await
                {
                    Ok(channel) => {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                    }
                    Err(e) => eprintln!("SSH tunnel: could not open channel: {e}"),
                }
            });
        }
    });

    Ok(Tunnel { local_port, task })
}

/// The chosen key file, or the usual default keys in ~/.ssh.
fn key_path(cfg: &SshConfig) -> Result<PathBuf, String> {
    if let Some(file) = cfg.key_file.as_deref().filter(|f| !f.trim().is_empty()) {
        let path = expand_home(file.trim());
        if !path.exists() {
            return Err(format!("The SSH key file {} doesn't exist.", path.display()));
        }
        return Ok(path);
    }
    for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
        let path = expand_home(&format!("~/.ssh/{name}"));
        if path.exists() {
            return Ok(path);
        }
    }
    Err("Choose a private key file (none found in your .ssh folder).".into())
}

fn key_error(path: &std::path::Path, secrets: &Secrets, e: russh::keys::Error) -> String {
    let text = e.to_string();
    let no_passphrase = secrets.ssh_passphrase.as_deref().unwrap_or("").is_empty();
    if text.to_lowercase().contains("encrypted") || (no_passphrase && text.to_lowercase().contains("passphrase")) {
        format!("The key {} is protected by a passphrase. Enter it in the SSH section.", path.display())
    } else if path.extension().map(|e| e.eq_ignore_ascii_case("ppk")).unwrap_or(false) {
        format!(
            "{} is a PuTTY key (.ppk), which isn't supported yet. Export it in OpenSSH format with PuTTYgen \
             (Conversions > Export OpenSSH key), or use Pageant with the \"SSH agent\" option.",
            path.display()
        )
    } else {
        format!("Could not read the SSH key {}: {text}", path.display())
    }
}

/// Try every key offered by the Windows OpenSSH agent (also used by 1Password), then Pageant.
async fn agent_login(session: &mut client::Handle<Client>, user: &str) -> Result<bool, String> {
    let hash = session.best_supported_rsa_hash().await.map_err(|e| e.to_string())?.flatten();
    let mut tried_any = false;

    if let Ok(mut agent) =
        russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
    {
        if let Ok(identities) = agent.request_identities().await {
            for identity in identities {
                if let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity {
                    tried_any = true;
                    if let Ok(result) = session.authenticate_publickey_with(user, key, hash, &mut agent).await {
                        if result.success() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    if let Ok(mut agent) = russh::keys::agent::client::AgentClient::connect_pageant().await {
        if let Ok(identities) = agent.request_identities().await {
            for identity in identities {
                if let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity {
                    tried_any = true;
                    if let Ok(result) = session.authenticate_publickey_with(user, key, hash, &mut agent).await {
                        if result.success() {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    if !tried_any {
        return Err("No SSH agent with keys was found. Start the OpenSSH Authentication Agent service \
                    (or 1Password / Pageant) and add your key."
            .into());
    }
    Ok(false)
}
