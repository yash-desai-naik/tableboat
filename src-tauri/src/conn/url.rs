//! Connection URLs, for "Import from URL", "Copy as URL" and the export file.
//!
//! Formats:
//!   mysql://user:pass@host:3306/db?ssl-mode=REQUIRED&name=Nick&color=1f6feb&tag=production
//!   mariadb://…   postgres://… (postgresql://)   mongodb://…   mongodb+srv://…
//!   sqlite:///C:/path/to/file.db
//!   mysql+ssh://sshuser:sshpass@sshhost:22/user:pass@host:3306/db?…   (SSH tunnel)
//! Every part is percent-encoded, so passwords may contain any character.
//! `statusColor` and `env` are accepted as aliases of `color` and `tag`.

use super::model::{Connection, Engine, Secrets, SshAuth, SslMode};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use url::Url;

/// Characters left as-is in user names, passwords and option values.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, COMPONENT).to_string()
}

fn dec(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn engine_from_scheme(scheme: &str) -> Result<(Engine, bool), String> {
    let (base, ssh) = match scheme.strip_suffix("+ssh") {
        Some(b) => (b, true),
        None => (scheme, false),
    };
    let engine = match base {
        "mysql" => Engine::Mysql,
        "mariadb" => Engine::Mariadb,
        "postgres" | "postgresql" => Engine::Postgres,
        "sqlite" | "sqlite3" => Engine::Sqlite,
        "mongodb" | "mongodb+srv" => Engine::Mongodb,
        other => return Err(format!("Unsupported URL type \"{other}://\"")),
    };
    Ok((engine, ssh))
}

fn ssl_mode_from(value: &str) -> Option<SslMode> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "disable" | "disabled" | "false" | "0" => Some(SslMode::Disabled),
        "prefer" | "preferred" | "allow" => Some(SslMode::Preferred),
        "require" | "required" | "true" | "1" => Some(SslMode::Required),
        "verify-ca" => Some(SslMode::VerifyCa),
        "verify-full" | "verify-identity" => Some(SslMode::VerifyIdentity),
        _ => None,
    }
}

fn ssl_mode_name(mode: SslMode, engine: Engine) -> &'static str {
    let pg = engine == Engine::Postgres;
    match mode {
        SslMode::Disabled => if pg { "disable" } else { "DISABLED" },
        SslMode::Preferred => if pg { "prefer" } else { "PREFERRED" },
        SslMode::Required => if pg { "require" } else { "REQUIRED" },
        SslMode::VerifyCa => if pg { "verify-ca" } else { "VERIFY_CA" },
        SslMode::VerifyIdentity => if pg { "verify-full" } else { "VERIFY_IDENTITY" },
    }
}

/// Parse a connection URL into a connection (not yet saved) and its secrets.
pub fn parse(input: &str) -> Result<(Connection, Secrets), String> {
    let input = input.trim();
    let scheme_end = input.find("://").ok_or("Not a connection URL: it should start with e.g. mysql://")?;
    let scheme = input[..scheme_end].to_ascii_lowercase();
    let (engine, over_ssh) = engine_from_scheme(&scheme)?;

    let mut conn = Connection { engine, ..Connection::default() };
    let mut secrets = Secrets::default();

    if engine == Engine::Sqlite {
        let rest = &input[scheme_end + 3..];
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut path = dec(path);
        // sqlite:///C:/x → "/C:/x"; drop the slash before a drive letter.
        if path.len() > 2 && path.starts_with('/') && path.as_bytes()[2] == b':' {
            path.remove(0);
        }
        if path.is_empty() {
            return Err("The SQLite URL has no file path".into());
        }
        // Windows keeps its native separators; macOS and Linux keep the URL's forward slashes.
        conn.file = Some(if cfg!(windows) { path.replace('/', "\\") } else { path });
        conn.host = String::new();
        apply_query(&mut conn, query);
        return Ok((conn, secrets));
    }

    // With +ssh the outer URL is the SSH server and the path holds the database URL.
    let db_part: String;
    if over_ssh {
        let outer = Url::parse(&input.replacen(&input[..scheme_end], "ssh", 1))
            .map_err(|e| format!("Invalid SSH part of the URL: {e}"))?;
        conn.ssh.enabled = true;
        conn.ssh.host = outer.host_str().unwrap_or_default().to_string();
        conn.ssh.port = outer.port().unwrap_or(22);
        conn.ssh.user = dec(outer.username());
        if let Some(p) = outer.password() {
            secrets.ssh_password = Some(dec(p));
            conn.ssh.auth = SshAuth::Password;
        }
        let inner = outer.path().trim_start_matches('/');
        let query = outer.query().map(|q| format!("?{q}")).unwrap_or_default();
        db_part = format!("{}://{}{}", input[..scheme_end].trim_end_matches("+ssh"), inner, query);
    } else {
        db_part = input.to_string();
    }

    // mongodb+srv has no port and may list several hosts; url::Url can't parse host lists,
    // so take the authority apart by hand.
    let after_scheme = &db_part[db_part.find("://").unwrap() + 3..];
    let (authority, rest) = match after_scheme.find(['/', '?']) {
        Some(i) => (&after_scheme[..i], &after_scheme[i..]),
        None => (after_scheme, ""),
    };
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    if let Some(ui) = userinfo {
        let (u, p) = ui.split_once(':').map(|(u, p)| (u, Some(p))).unwrap_or((ui, None));
        conn.user = dec(u);
        if let Some(p) = p {
            secrets.password = Some(dec(p));
        }
    }

    if scheme.starts_with("mongodb+srv") {
        conn.options.insert("srv".into(), "true".into());
        conn.host = hostport.to_string();
    } else if hostport.contains(',') {
        // Replica-set host list: keep it whole in host.
        conn.host = hostport.to_string();
    } else if let Some(stripped) = hostport.strip_prefix('[') {
        // [ipv6]:port
        let (h, p) = stripped.split_once(']').unwrap_or((stripped, ""));
        conn.host = h.to_string();
        conn.port = p.trim_start_matches(':').parse().ok();
    } else {
        let (h, p) = hostport.rsplit_once(':').map(|(h, p)| (h, Some(p))).unwrap_or((hostport, None));
        conn.host = dec(h);
        conn.port = p.and_then(|p| p.parse().ok());
    }
    if conn.host.is_empty() {
        conn.host = "127.0.0.1".into();
    }

    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    conn.database = dec(path.trim_start_matches('/'));
    apply_query(&mut conn, query);
    Ok((conn, secrets))
}

fn apply_query(conn: &mut Connection, query: &str) {
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = dec(k);
        let value = dec(&v.replace('+', " "));
        match key.as_str() {
            "name" => conn.name = value,
            "color" | "statusColor" => {
                let hex = value.trim_start_matches('#');
                if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    conn.color = format!("#{}", hex.to_ascii_lowercase());
                }
            }
            "tag" | "env" => conn.tag = Some(value).filter(|v| !v.is_empty()),
            "sslmode" | "ssl-mode" | "ssl_mode" | "sslMode" | "tls" | "ssl" => {
                if let Some(mode) = ssl_mode_from(&value) {
                    conn.ssl.mode = mode;
                }
            }
            "sslrootcert" | "ssl-ca" => conn.ssl.ca_file = Some(value),
            "sslcert" | "ssl-cert" => conn.ssl.cert_file = Some(value),
            "sslkey" | "ssl-key" => conn.ssl.key_file = Some(value),
            "sshKey" | "ssh-key" => {
                conn.ssh.auth = SshAuth::Key;
                conn.ssh.key_file = Some(value);
            }
            "sshAgent" | "ssh-agent" if value == "true" => conn.ssh.auth = SshAuth::Agent,
            _ => {
                conn.options.insert(key, value);
            }
        }
    }
}

/// Build a URL for a connection. Passwords are included only when asked for.
pub fn to_url(conn: &Connection, secrets: &Secrets, include_passwords: bool) -> String {
    let mut params: Vec<(String, String)> = Vec::new();
    if !conn.name.is_empty() {
        params.push(("name".into(), conn.name.clone()));
    }
    params.push(("color".into(), conn.color.trim_start_matches('#').to_string()));
    if let Some(tag) = &conn.tag {
        params.push(("tag".into(), tag.clone()));
    }

    let query = |extra: &[(String, String)], params: &[(String, String)]| {
        let all: Vec<String> = extra
            .iter()
            .chain(params.iter())
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect();
        if all.is_empty() { String::new() } else { format!("?{}", all.join("&")) }
    };

    if conn.engine == Engine::Sqlite {
        let file = conn.file.clone().unwrap_or_default().replace('\\', "/");
        let encoded: Vec<String> = file.split('/').map(enc_path_segment).collect();
        // An absolute path already starts with "/", so it needs one slash, not the drive-letter form.
        let head = if cfg!(windows) { "sqlite:///" } else { "sqlite://" };
        return format!("{head}{}{}", encoded.join("/"), query(&[], &params));
    }

    let mut extra: Vec<(String, String)> = Vec::new();
    if conn.engine != Engine::Mongodb && conn.ssl.mode != SslMode::Preferred {
        let key = if conn.engine == Engine::Postgres { "sslmode" } else { "ssl-mode" };
        extra.push((key.into(), ssl_mode_name(conn.ssl.mode, conn.engine).into()));
    }
    if let Some(ca) = &conn.ssl.ca_file {
        extra.push(("sslrootcert".into(), ca.clone()));
    }
    if let Some(cert) = &conn.ssl.cert_file {
        extra.push(("sslcert".into(), cert.clone()));
    }
    if let Some(key) = &conn.ssl.key_file {
        extra.push(("sslkey".into(), key.clone()));
    }
    let srv = conn.options.get("srv").map(|v| v == "true").unwrap_or(false);
    for (k, v) in &conn.options {
        if k != "srv" {
            extra.push((k.clone(), v.clone()));
        }
    }
    if conn.ssh.enabled {
        match conn.ssh.auth {
            SshAuth::Key => {
                if let Some(k) = &conn.ssh.key_file {
                    extra.push(("sshKey".into(), k.clone()));
                }
            }
            SshAuth::Agent => extra.push(("sshAgent".into(), "true".into())),
            SshAuth::Password => {}
        }
    }

    let scheme = match conn.engine {
        Engine::Mysql => "mysql",
        Engine::Mariadb => "mariadb",
        Engine::Postgres => "postgresql",
        Engine::Mongodb if srv => "mongodb+srv",
        Engine::Mongodb => "mongodb",
        Engine::Sqlite => unreachable!(),
    };

    let userinfo = {
        let mut s = enc(&conn.user);
        if include_passwords {
            if let Some(p) = secrets.password.as_deref().filter(|p| !p.is_empty()) {
                s.push(':');
                s.push_str(&enc(p));
            }
        }
        if s.is_empty() { String::new() } else { format!("{s}@") }
    };
    let host = if conn.host.contains(':') && !conn.host.contains(',') && !conn.host.starts_with('[') {
        format!("[{}]", conn.host)
    } else {
        conn.host.clone()
    };
    let port = match conn.port {
        Some(p) if !srv && !conn.host.contains(',') => format!(":{p}"),
        _ => String::new(),
    };
    let db_part = format!("{userinfo}{host}{port}/{}", enc(&conn.database));

    if conn.ssh.enabled {
        let mut ssh_user = enc(&conn.ssh.user);
        if include_passwords && conn.ssh.auth == SshAuth::Password {
            if let Some(p) = secrets.ssh_password.as_deref().filter(|p| !p.is_empty()) {
                ssh_user.push(':');
                ssh_user.push_str(&enc(p));
            }
        }
        let ssh_user = if ssh_user.is_empty() { String::new() } else { format!("{ssh_user}@") };
        format!(
            "{scheme}+ssh://{ssh_user}{}:{}/{db_part}{}",
            conn.ssh.host,
            conn.ssh.port,
            query(&extra, &params)
        )
    } else {
        format!("{scheme}://{db_part}{}", query(&extra, &params))
    }
}

fn enc_path_segment(seg: &str) -> String {
    // Keep drive letters ("C:") readable.
    if seg.len() == 2 && seg.ends_with(':') {
        seg.to_string()
    } else {
        enc(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_round_trip_with_special_password() {
        let (c, s) = parse("mysql://app%40user:p%40ss%3Aw%2Frd%20x@db.example.com:3307/shop?ssl-mode=REQUIRED&name=Shop%20prod&color=FF0000&tag=production").unwrap();
        assert_eq!(c.engine, Engine::Mysql);
        assert_eq!(c.user, "app@user");
        assert_eq!(s.password.as_deref(), Some("p@ss:w/rd x"));
        assert_eq!(c.host, "db.example.com");
        assert_eq!(c.port, Some(3307));
        assert_eq!(c.database, "shop");
        assert_eq!(c.ssl.mode, SslMode::Required);
        assert_eq!(c.name, "Shop prod");
        assert_eq!(c.color, "#ff0000");
        assert_eq!(c.tag.as_deref(), Some("production"));

        let url = to_url(&c, &s, true);
        let (c2, s2) = parse(&url).unwrap();
        assert_eq!(c2.user, c.user);
        assert_eq!(s2.password, s.password);
        assert_eq!(c2.port, c.port);
        assert_eq!(c2.name, c.name);
        assert_eq!(c2.ssl.mode, c.ssl.mode);

        let no_pw = to_url(&c, &s, false);
        assert!(!no_pw.contains("p%40ss"));
    }

    #[test]
    fn ssh_url() {
        let (c, s) = parse("mysql+ssh://deploy:sshpw@bastion.example.com:2222/root:dbpw@10.0.0.5:3306/app?name=Via%20SSH").unwrap();
        assert!(c.ssh.enabled);
        assert_eq!(c.ssh.host, "bastion.example.com");
        assert_eq!(c.ssh.port, 2222);
        assert_eq!(c.ssh.user, "deploy");
        assert_eq!(s.ssh_password.as_deref(), Some("sshpw"));
        assert_eq!(c.user, "root");
        assert_eq!(s.password.as_deref(), Some("dbpw"));
        assert_eq!(c.host, "10.0.0.5");
        assert_eq!(c.database, "app");
        assert_eq!(c.name, "Via SSH");
        let (c2, s2) = parse(&to_url(&c, &s, true)).unwrap();
        assert_eq!(c2.ssh.host, c.ssh.host);
        assert_eq!(s2.ssh_password, s.ssh_password);
        assert_eq!(c2.host, c.host);
    }

    #[test]
    fn sqlite_windows_path() {
        let (c, _) = parse("sqlite:///C:/Data/my%20db.sqlite").unwrap();
        // Paths keep the platform's separators: Windows backslashes, macOS and Linux slashes.
        #[cfg(windows)]
        assert_eq!(c.file.as_deref(), Some("C:\\Data\\my db.sqlite"));
        #[cfg(not(windows))]
        assert_eq!(c.file.as_deref(), Some("C:/Data/my db.sqlite"));
        let (c2, _) = parse(&to_url(&c, &Secrets::default(), false)).unwrap();
        assert_eq!(c2.file, c.file);
    }

    #[test]
    #[cfg(not(windows))]
    fn sqlite_unix_path_round_trip() {
        let (c, _) = parse("sqlite:///Users/me/shop.db").unwrap();
        assert_eq!(c.file.as_deref(), Some("/Users/me/shop.db"));
        let url = to_url(&c, &Secrets::default(), false);
        assert!(url.starts_with("sqlite:///Users/me/shop.db"), "{url}");
        let (c2, _) = parse(&url).unwrap();
        assert_eq!(c2.file, c.file);
    }

    #[test]
    fn mongodb_srv_and_options() {
        let (c, s) = parse("mongodb+srv://u:p%3D@cluster0.abc.mongodb.net/app?authSource=admin&retryWrites=true").unwrap();
        assert_eq!(c.engine, Engine::Mongodb);
        assert_eq!(c.host, "cluster0.abc.mongodb.net");
        assert_eq!(s.password.as_deref(), Some("p="));
        assert_eq!(c.options.get("authSource").map(String::as_str), Some("admin"));
        let url = to_url(&c, &s, false);
        assert!(url.starts_with("mongodb+srv://u@cluster0.abc.mongodb.net/app?"));
    }

    #[test]
    fn tableplus_style_aliases() {
        let (c, _) = parse("postgresql://u@h/db?statusColor=007F3D&env=staging").unwrap();
        assert_eq!(c.engine, Engine::Postgres);
        assert_eq!(c.color, "#007f3d");
        assert_eq!(c.tag.as_deref(), Some("staging"));
    }
}
