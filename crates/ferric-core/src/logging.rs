//! Chat logging to files in the Ferric folder. Off by default.
//!
//! Privacy: `ChatLog::append` only ever writes the single `line` its caller
//! passes. It has no other input and no access to UI state, so callers must
//! never pass text typed into a password field (NickServ passwords, server
//! passwords, proxy credentials, and so on). Nothing here filters secrets.
//!
//! Files are `<base_dir>/<network>/<target>.log`. Network and target names
//! are validated by `log_path`, so a name from the wire cannot escape
//! `base_dir`. Std only.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_BYTES: u64 = 10 * 1024 * 1024;
const MAX_NAME_LEN: usize = 64;

pub struct ChatLog {
    base_dir: PathBuf,
    enabled: bool,
    max_bytes: u64,
}

impl ChatLog {
    /// Disabled by default; 10 MiB cap per file.
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            enabled: false,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// `base_dir/<network>/<target>.log`, or `None` if either name is unsafe.
    pub fn log_path(&self, network: &str, target: &str) -> Option<PathBuf> {
        if !is_safe_name(network) || !is_safe_name(target) {
            return None;
        }
        Some(self.base_dir.join(network).join(format!("{target}.log")))
    }

    /// Append one timestamped line. `Ok(false)` when disabled, when the name is
    /// unsafe, or when the file already reached `max_bytes`; `Ok(true)` when
    /// written. CR, LF and NUL are stripped from `line`.
    pub fn append(&self, network: &str, target: &str, line: &str) -> io::Result<bool> {
        if !self.enabled {
            return Ok(false);
        }
        let Some(path) = self.log_path(network, target) else {
            return Ok(false);
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if file.metadata()?.len() >= self.max_bytes {
            // Keep one previous file as <target>.1.log and start a new one, as DebugLog does.
            drop(file);
            let rotated = path.with_extension("1.log");
            let _ = fs::remove_file(&rotated);
            fs::rename(&path, &rotated)?;
            file = OpenOptions::new().create(true).append(true).open(&path)?;
        }
        let clean: String = line
            .chars()
            .filter(|c| !matches!(c, '\r' | '\n' | '\0'))
            .collect();
        // One write call keeps the line contiguous in append mode.
        let out = format!("{} {}\n", timestamp_utc(SystemTime::now()), clean);
        file.write_all(out.as_bytes())?;
        Ok(true)
    }
}

fn is_safe_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    // Letters of any script are allowed (#äpfel). Separators are not, so no path can escape.
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '#' | '@'))
    {
        return false;
    }
    // No ".." anywhere (also covers a lone ".."), no lone ".", and no trailing
    // dot because Windows strips it and the name would alias another file.
    if name.contains("..") || name == "." || name.ends_with('.') {
        return false;
    }
    !is_reserved_windows_name(name)
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        s if s.len() == 4 && (s.starts_with("COM") || s.starts_with("LPT")) => {
            matches!(s.as_bytes()[3], b'1'..=b'9')
        }
        _ => false,
    }
}

/// "YYYY-MM-DD HH:MM:SS" in UTC. Times before 1970 clamp to the epoch.
pub fn timestamp_utc(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Unique temp dir, removed on drop (also when an assert fails).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            Self(std::env::temp_dir().join(format!(
                "ferric-logging-test-{}-{nanos}-{n}",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn enabled_log(dir: &TempDir) -> ChatLog {
        let mut log = ChatLog::new(dir.0.clone());
        log.set_enabled(true);
        log
    }

    #[test]
    fn safe_name_accepted() {
        let log = ChatLog::new(PathBuf::from("base"));
        let p = log.log_path("Libera-Chat", "#rust_lang@x.y").unwrap();
        assert_eq!(
            p,
            PathBuf::from("base").join("Libera-Chat").join("#rust_lang@x.y.log")
        );
        assert!(log.log_path("net", "COMET").is_some());
    }

    #[test]
    fn unsafe_names_refused() {
        let log = ChatLog::new(PathBuf::from("base"));
        for bad in ["..", "a/b", "a\\b", "CON", "con.txt", "COM1", "lpt9", "NUL", "", ".", "a..b", "x.", "a b", "a:b"] {
            assert!(log.log_path("net", bad).is_none(), "target {bad:?}");
            assert!(log.log_path(bad, "chan").is_none(), "network {bad:?}");
        }
        let long = "a".repeat(65);
        assert!(log.log_path("net", &long).is_none());
        assert!(log.log_path("net", &"a".repeat(64)).is_some());
    }

    #[test]
    fn disabled_writes_nothing() {
        let dir = TempDir::new();
        let log = ChatLog::new(dir.0.clone());
        assert!(!log.append("net", "#chan", "hello").unwrap());
        assert!(!dir.0.exists());
    }

    #[test]
    fn enabled_writes_timestamped_line() {
        let dir = TempDir::new();
        let log = enabled_log(&dir);
        assert!(log.append("net", "#chan", "hello").unwrap());
        let text = fs::read_to_string(log.log_path("net", "#chan").unwrap()).unwrap();
        assert!(text.ends_with(" hello\n"), "{text:?}");
        let b = text.as_bytes();
        assert_eq!(text.lines().count(), 1);
        // "YYYY-MM-DD HH:MM:SS " shape
        assert!(b[..4].iter().all(u8::is_ascii_digit));
        assert_eq!((b[4], b[7], b[10], b[13], b[16], b[19]), (b'-', b'-', b' ', b':', b':', b' '));
        assert!(b[11..13].iter().chain(&b[14..16]).chain(&b[17..19]).all(u8::is_ascii_digit));
    }

    #[test]
    fn cr_lf_nul_removed() {
        let dir = TempDir::new();
        let log = enabled_log(&dir);
        assert!(log.append("net", "#chan", "a\r\nb\nc\rd\0e").unwrap());
        let text = fs::read_to_string(log.log_path("net", "#chan").unwrap()).unwrap();
        assert!(text.ends_with(" abcde\n"), "{text:?}");
        assert_eq!(text.lines().count(), 1);
    }

    #[test]
    fn full_file_rotates_and_logging_continues() {
        let dir = TempDir::new();
        let mut log = enabled_log(&dir);
        log.max_bytes = 30;
        assert!(log.append("net", "#chan", "first line").unwrap());
        // The first line is 31 bytes with its timestamp: at or above the cap now.
        assert!(log.append("net", "#chan", "second line").unwrap());
        let path = log.log_path("net", "#chan").unwrap();
        let rotated = path.with_extension("1.log");
        assert!(fs::read_to_string(&rotated).unwrap().ends_with(" first line\n"));
        assert!(fs::read_to_string(&path).unwrap().ends_with(" second line\n"));
    }

    #[test]
    fn non_ascii_channel_names_are_logged() {
        let dir = TempDir::new();
        let log = enabled_log(&dir);
        assert!(log.append("net", "#äpfel", "hallo").unwrap());
        assert!(log.log_path("net", "#äpfel").unwrap().exists());
        assert!(log.log_path("net", "../escape").is_none());
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
