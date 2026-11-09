//! MySQL and MariaDB through mysql_async (pure Rust, rustls for TLS).

use super::ServerInfo;
use crate::conn::model::{Connection, Engine, Secrets, SslMode};
use crate::paths::expand_home;
use mysql_async::prelude::*;
use mysql_async::{ClientIdentity, OptsBuilder, Pool, PoolConstraints, PoolOpts, SslOpts};
use std::time::Duration;

#[derive(Clone)]
pub struct MysqlBackend {
    pub pool: Pool,
    /// The SQL editor's own connection, kept for the whole session so that session state
    /// (variables, BEGIN, temporary tables, USE) carries over between runs.
    pub editor: std::sync::Arc<tokio::sync::Mutex<Option<mysql_async::Conn>>>,
}

impl MysqlBackend {
    /// `host`/`port` are where to connect (127.0.0.1 and the local port when tunnelled);
    /// `conn.host` stays the name the TLS certificate is checked against.
    pub async fn connect(conn: &Connection, secrets: &Secrets, host: &str, port: u16) -> Result<(Self, ServerInfo), String> {
        let ssl = match conn.ssl.mode {
            SslMode::Disabled => None,
            _ => Some(ssl_opts(conn, host)?),
        };
        match try_connect(conn, secrets, host, port, ssl).await {
            Ok(done) => Ok(done),
            // "Preferred": fall back to an unencrypted connection when TLS itself fails.
            // A server error (wrong password, unknown database) would fail the same way, so don't retry those.
            Err(e) if conn.ssl.mode == SslMode::Preferred && !matches!(e, mysql_async::Error::Server(_)) => {
                try_connect(conn, secrets, host, port, None).await.map_err(|e| describe(&e, conn, host, port))
            }
            Err(e) => Err(describe(&e, conn, host, port)),
        }
    }

    /// Closes the editor connection and every pooled connection (clones share them).
    pub async fn close(&self) {
        // The pool waits for borrowed connections, so release the editor's first.
        if let Some(conn) = self.editor.lock().await.take() {
            let _ = conn.disconnect().await;
        }
        let _ = self.pool.clone().disconnect().await;
    }
}

fn ssl_opts(conn: &Connection, connect_host: &str) -> Result<SslOpts, String> {
    let mut opts = SslOpts::default();
    match conn.ssl.mode {
        SslMode::Preferred | SslMode::Required => {
            opts = opts.with_danger_accept_invalid_certs(true).with_danger_skip_domain_validation(true);
        }
        SslMode::VerifyCa => opts = opts.with_danger_skip_domain_validation(true),
        SslMode::VerifyIdentity | SslMode::Disabled => {}
    }
    if let Some(ca) = conn.ssl.ca_file.as_deref().filter(|s| !s.is_empty()) {
        let path = expand_home(ca);
        if !path.exists() {
            return Err(format!("The CA certificate {} doesn't exist.", path.display()));
        }
        opts = opts.with_root_certs(vec![path.into()]);
    }
    match (
        conn.ssl.cert_file.as_deref().filter(|s| !s.is_empty()),
        conn.ssl.key_file.as_deref().filter(|s| !s.is_empty()),
    ) {
        (Some(cert), Some(key)) => {
            opts = opts.with_client_identity(Some(ClientIdentity::new(expand_home(cert).into(), expand_home(key).into())));
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err("A client certificate needs both the certificate and its key file.".into());
        }
        (None, None) => {}
    }
    // Through a tunnel we connect to 127.0.0.1, but the certificate names the real host.
    if connect_host != conn.host {
        opts = opts.with_danger_tls_hostname_override(Some(conn.host.clone()));
    }
    Ok(opts)
}

async fn try_connect(
    conn: &Connection,
    secrets: &Secrets,
    host: &str,
    port: u16,
    ssl: Option<SslOpts>,
) -> Result<(MysqlBackend, ServerInfo), mysql_async::Error> {
    let encrypted = ssl.is_some();
    let constraints = PoolConstraints::new(0, 4).expect("valid pool constraints");
    let opts = OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .user(Some(conn.user.clone()).filter(|u| !u.is_empty()))
        .pass(secrets.password.clone().filter(|p| !p.is_empty()))
        .db_name(Some(conn.database.clone()).filter(|d| !d.is_empty()))
        .prefer_socket(false)
        // "Affected rows" = rows matched, so saving an unchanged value still counts as 1 row.
        .client_found_rows(true)
        .ssl_opts(ssl)
        .pool_opts(
            PoolOpts::default()
                .with_constraints(constraints)
                .with_inactive_connection_ttl(Duration::from_secs(300)),
        );

    let pool = Pool::new(opts);
    let mut c = match tokio::time::timeout(Duration::from_secs(15), pool.get_conn()).await {
        Ok(result) => result?,
        Err(_) => {
            let _ = pool.disconnect().await;
            return Err(mysql_async::Error::Other(
                format!("Timed out after 15 s connecting to {host}:{port}.").into(),
            ));
        }
    };

    let (version, database): (String, Option<String>) = c
        .query_first("SELECT VERSION(), DATABASE()")
        .await?
        .unwrap_or_default();
    let cipher: Option<(String, String)> = if encrypted {
        c.query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'").await?
    } else {
        None
    };
    drop(c);

    let is_maria = version.to_ascii_lowercase().contains("mariadb");
    let info = ServerInfo {
        engine: Some(if is_maria { Engine::Mariadb } else { Engine::Mysql }),
        product: if is_maria { "MariaDB" } else { "MySQL" }.into(),
        version: short_version(&version),
        full_version: version,
        database,
        tls: cipher.map(|(_, v)| !v.is_empty()).unwrap_or(false),
        ssh: false,
    };
    Ok((MysqlBackend { pool, editor: Default::default() }, info))
}

/// "10.11.6-MariaDB-0+deb12u1" → "10.11.6"; "8.0.42" stays.
fn short_version(v: &str) -> String {
    v.split(['-', ' ']).next().unwrap_or(v).to_string()
}

fn describe(e: &mysql_async::Error, conn: &Connection, host: &str, port: u16) -> String {
    let target = if host == conn.host { format!("{host}:{port}") } else { format!("{} (through SSH)", conn.host) };
    match e {
        mysql_async::Error::Server(s) => match s.code {
            1045 => format!("Access denied for user \"{}\". Check the user name and password.", conn.user),
            1049 => format!("The database \"{}\" doesn't exist on {target}.", conn.database),
            _ => format!("{} (MySQL error {})", s.message, s.code),
        },
        mysql_async::Error::Io(io) => {
            let text = io.to_string();
            if text.to_lowercase().contains("refused") {
                format!("Nothing is accepting connections at {target}. Is the server running, and is the port right?")
            } else {
                format!("Could not connect to {target}: {text}")
            }
        }
        other => format!("Could not connect to {target}: {other}"),
    }
}
