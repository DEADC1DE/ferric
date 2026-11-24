//! Connection debug log: `<logs>/connect-debug.log`. On by default. Std only.
//!
//! Line: `YYYY-MM-DD HH:MM:SS <network> <step> | <result>` (UTC).
//!
//! Privacy: `write` takes only the network id, a step name and a short result.
//! There is no field for a password, key, message text or proxy credential, so
//! callers cannot pass one by accident; they must never put one in `result`.
//! Control characters are stripped and `result` is cut to `MAX_RESULT` bytes.
//!
//! The file rotates at 1 MiB: `connect-debug.log` becomes `connect-debug.1.log`
//! (an older `.1` is deleted), so at most two files exist.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use crate::logging::timestamp_utc;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_RESULT: usize = 300;
const MAX_FIELD: usize = 64;
const FILE: &str = "connect-debug.log";
const OLD_FILE: &str = "connect-debug.1.log";

pub struct DebugLog {
    dir: PathBuf,
    enabled: AtomicBool,
    max_bytes: u64,
    /// Serializes writes and rotation across networks that connect in parallel.
    lock: Mutex<()>,
}

impl DebugLog {
    /// Enabled; files go to `dir` (the `logs` folder of the project folder).
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            enabled: AtomicBool::new(true),
            max_bytes: MAX_BYTES,
            lock: Mutex::new(()),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Append one line. Errors are dropped on purpose: a full disk must never
    /// stop a connection.
    pub fn write(&self, network: &str, step: &str, result: &str) {
        if !self.is_enabled() {
            return;
        }
        let line = format!(
            "{} {} {} | {}\n",
            timestamp_utc(SystemTime::now()),
            token(network),
            clean(step, MAX_FIELD).replace('|', "/"),
            clean(result, MAX_RESULT)
        );
        let Ok(_guard) = self.lock.lock() else {
            return;
        };
        let _ = self.append(&line);
    }

    fn append(&self, line: &str) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(FILE);
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if len > 0 && len + line.len() as u64 > self.max_bytes {
            let old = self.dir.join(OLD_FILE);
            let _ = fs::remove_file(&old);
            fs::rename(&path, &old)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(line.as_bytes())
    }
}

/// One whitespace-free word: control characters dropped, whitespace as `_`.
fn token(s: &str) -> String {
    let t = clean(s, MAX_FIELD).replace(' ', "_");
    if t.is_empty() {
        "-".to_string()
    } else {
        t
    }
}

/// `s` without control characters, at most `max` bytes (cut at a char boundary).
fn clean(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// Per-connect handle: network id, start time and the shared log. Adds the
/// milliseconds since the start of the connect to every line.
#[derive(Clone)]
pub struct Trace {
    log: Option<Arc<DebugLog>>,
    network: String,
    start: Instant,
}

impl Trace {
    pub fn new(log: Option<Arc<DebugLog>>, network: &str) -> Self {
        Self {
            log,
            network: network.to_string(),
            start: Instant::now(),
        }
    }

    pub fn step(&self, step: &str, result: &str) {
        if let Some(log) = &self.log {
            let ms = self.start.elapsed().as_millis();
            let result = if result.is_empty() {
                format!("t={ms}ms")
            } else {
                format!("{} t={ms}ms", clean(result, MAX_RESULT))
            };
            log.write(&self.network, step, &result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::UNIX_EPOCH;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Unique temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            Self(std::env::temp_dir().join(format!(
                "ferric-debuglog-test-{}-{nanos}-{n}",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn line_has_timestamp_network_step_and_result() {
        let dir = TempDir::new();
        let log = DebugLog::new(dir.0.clone());
        log.write("my net", "dns resolved", "2 addresses");
        let text = fs::read_to_string(dir.0.join(FILE)).unwrap();
        assert_eq!(text.lines().count(), 1);
        let b = text.as_bytes();
        assert!(b[..4].iter().all(u8::is_ascii_digit));
        assert_eq!((b[4], b[7], b[10], b[13], b[16], b[19]), (b'-', b'-', b' ', b':', b':', b' '));
        assert_eq!(&text[20..], "my_net dns resolved | 2 addresses\n");
    }

    #[test]
    fn control_chars_removed_and_result_capped() {
        let dir = TempDir::new();
        let log = DebugLog::new(dir.0.clone());
        log.write("n", "s", &format!("a\r\nb\0c{}", "x".repeat(1000)));
        let text = fs::read_to_string(dir.0.join(FILE)).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains(" | abcxxx"));
        assert!(text.len() < 20 + 300 + 20);
    }

    #[test]
    fn disabled_writes_nothing() {
        let dir = TempDir::new();
        let log = DebugLog::new(dir.0.clone());
        log.set_enabled(false);
        log.write("n", "s", "r");
        assert!(!dir.0.exists());
        log.set_enabled(true);
        log.write("n", "s", "r");
        assert!(dir.0.join(FILE).is_file());
    }

    #[test]
    fn cap_rotates_and_keeps_two_files() {
        let dir = TempDir::new();
        let mut log = DebugLog::new(dir.0.clone());
        log.max_bytes = 200;
        for i in 0..40 {
            log.write("net", "step", &format!("line {i:03}"));
        }
        let mut names: Vec<String> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![OLD_FILE.to_string(), FILE.to_string()]);
        for name in [FILE, OLD_FILE] {
            assert!(fs::metadata(dir.0.join(name)).unwrap().len() <= 200, "{name}");
        }
        // The newest line is in the current file; the oldest ones are gone.
        let cur = fs::read_to_string(dir.0.join(FILE)).unwrap();
        let old = fs::read_to_string(dir.0.join(OLD_FILE)).unwrap();
        assert!(cur.contains("line 039"));
        assert!(!cur.contains("line 000") && !old.contains("line 000"));
        assert!(old.contains("line 03"));
    }

    #[test]
    fn write_takes_only_network_step_and_result() {
        // A signature change that adds a secret field breaks this line.
        let _: fn(&DebugLog, &str, &str, &str) = DebugLog::write;
        let _: fn(&Trace, &str, &str) = Trace::step;
    }

    #[test]
    fn trace_adds_elapsed_milliseconds() {
        let dir = TempDir::new();
        let log = Arc::new(DebugLog::new(dir.0.clone()));
        let t = Trace::new(Some(log), "net");
        t.step("start", "tls yes");
        t.step("tcp connected", "");
        let text = fs::read_to_string(dir.0.join(FILE)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains(" net start | tls yes t=") && lines[0].ends_with("ms"));
        assert!(lines[1].contains(" net tcp connected | t="));
        // No log: no panic, nothing written.
        Trace::new(None, "net").step("start", "x");
    }
}
