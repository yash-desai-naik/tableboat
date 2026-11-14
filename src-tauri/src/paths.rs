use std::path::{Path, PathBuf};

/// `%APPDATA%\Tableboat` on Windows, `~/Library/Application Support/Tableboat` on macOS,
/// created if missing: settings, saved connections, tab state.
///
/// `TABLEBOAT_DATA_DIR` overrides it (tests and a portable copy can use their own folder).
pub fn data_dir() -> Result<PathBuf, String> {
    let dir = match std::env::var_os("TABLEBOAT_DATA_DIR").filter(|v| !v.is_empty()) {
        Some(custom) => PathBuf::from(custom),
        None => dirs::config_dir().ok_or("Could not find the settings folder.")?.join("Tableboat"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Write to a temporary file, then rename over the target, so a crash mid-write
/// never leaves a half-written file.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

/// Expand a leading `~` to the user's home folder.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~").filter(|r| r.is_empty() || r.starts_with(['/', '\\'])) {
        if let Some(home) = home_dir() {
            return home.join(rest.trim_start_matches(['/', '\\']));
        }
    }
    PathBuf::from(path)
}

/// The user's home folder: `%USERPROFILE%` on Windows, `$HOME` on macOS.
pub fn home_dir() -> Option<PathBuf> {
    let preferred = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let other = if cfg!(windows) { "HOME" } else { "USERPROFILE" };
    std::env::var_os(preferred).or_else(|| std::env::var_os(other)).map(PathBuf::from)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// "2026-11-12 11:52 UTC" for a Unix time (no date library needed).
pub fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_stamp() {
        assert_eq!(super::utc_stamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(super::utc_stamp(1_793_620_800), "2026-11-02 12:00 UTC");
        assert_eq!(super::utc_stamp(951_782_400), "2000-02-29 00:00 UTC");
    }
}
