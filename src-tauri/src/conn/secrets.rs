//! Passwords and passphrases in the OS keychain (Windows Credential Manager, macOS Keychain),
//! one generic credential per secret: service "Tableboat", user "<connection-id>/<field>".
//! Nothing secret is ever written to connections.json.

use super::model::Secrets;
use keyring_core::{Entry, Error};

const SERVICE: &str = "Tableboat";

/// The OS store passwords live in, for messages shown to the user.
pub const STORE_NAME: &str = if cfg!(windows) { "Windows Credential Manager" } else { "the macOS Keychain" };

pub const PASSWORD: &str = "password";
pub const SSH_PASSWORD: &str = "ssh-password";
pub const SSH_PASSPHRASE: &str = "ssh-passphrase";
const FIELDS: [&str; 3] = [PASSWORD, SSH_PASSWORD, SSH_PASSPHRASE];

pub fn init() -> Result<(), String> {
    #[cfg(windows)]
    let store = windows_native_keyring_store::Store::new()
        .map_err(|e| format!("Windows Credential Manager is not available: {e}"))?;
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|e| format!("The macOS Keychain is not available: {e}"))?;
    #[cfg(not(any(windows, target_os = "macos")))]
    return Err("Tableboat can only save passwords on Windows and macOS.".into());
    #[cfg(any(windows, target_os = "macos"))]
    keyring_core::set_default_store(store);
    Ok(())
}

fn entry(connection_id: &str, field: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, &format!("{connection_id}/{field}")).map_err(|e| e.to_string())
}

pub fn get(connection_id: &str, field: &str) -> Result<Option<String>, String> {
    match entry(connection_id, field)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read a saved password: {e}")),
    }
}

pub fn set(connection_id: &str, field: &str, value: &str) -> Result<(), String> {
    entry(connection_id, field)?
        .set_password(value)
        .map_err(|e| format!("Could not save the password: {e}"))
}

pub fn delete(connection_id: &str, field: &str) -> Result<(), String> {
    match entry(connection_id, field)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove a saved password: {e}")),
    }
}

/// `None` leaves a secret unchanged, `Some("")` removes it, anything else saves it.
pub fn apply(connection_id: &str, secrets: &Secrets) -> Result<(), String> {
    let pairs = [
        (PASSWORD, &secrets.password),
        (SSH_PASSWORD, &secrets.ssh_password),
        (SSH_PASSPHRASE, &secrets.ssh_passphrase),
    ];
    for (field, value) in pairs {
        match value.as_deref() {
            None => {}
            Some("") => delete(connection_id, field)?,
            Some(v) => set(connection_id, field, v)?,
        }
    }
    Ok(())
}

pub fn load(connection_id: &str) -> Result<Secrets, String> {
    Ok(Secrets {
        password: get(connection_id, PASSWORD)?,
        ssh_password: get(connection_id, SSH_PASSWORD)?,
        ssh_passphrase: get(connection_id, SSH_PASSPHRASE)?,
    })
}

pub fn delete_all(connection_id: &str) -> Result<(), String> {
    for field in FIELDS {
        delete(connection_id, field)?;
    }
    Ok(())
}

/// Fill in any secret the caller didn't provide from what's stored for `connection_id`.
pub fn merge_with_stored(connection_id: &str, given: &Secrets) -> Result<Secrets, String> {
    let stored = if connection_id.is_empty() { Secrets::default() } else { load(connection_id)? };
    Ok(Secrets {
        password: given.password.clone().or(stored.password),
        ssh_password: given.ssh_password.clone().or(stored.ssh_password),
        ssh_passphrase: given.ssh_passphrase.clone().or(stored.ssh_passphrase),
    })
}
