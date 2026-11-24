use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use serde::Serialize;
use tokio::time::interval;

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const FILE: &str = "perf.log";
const OLD_FILE: &str = "perf.1.log";

#[derive(Default)]
pub struct PerfTelemetry {
    pub get_state_calls: AtomicU64,
    pub get_state_snapshot_calls: AtomicU64,
    pub get_state_snapshot_total_duration_us: AtomicU64,
    pub get_state_total_duration_us: AtomicU64,
    pub get_state_last_duration_us: AtomicU64,
    pub get_state_max_duration_us: AtomicU64,
    pub get_state_last_payload_bytes: AtomicUsize,
    pub get_state_last_lock_wait_us: AtomicU64,

    pub python_rpc_calls: AtomicU64,
    pub python_rpc_total_duration_us: AtomicU64,
    pub python_rpc_last_duration_us: AtomicU64,
    pub python_rpc_max_duration_us: AtomicU64,
}

#[derive(Serialize, Clone, Debug)]
pub struct BackendPerfReport {
    pub get_state_calls: u64,
    pub get_state_snapshot_calls: u64,
    pub get_state_snapshot_avg_ms: f64,
    pub get_state_avg_ms: f64,
    pub get_state_last_ms: f64,
    pub get_state_max_ms: f64,
    pub get_state_last_lock_wait_ms: f64,
    pub get_state_last_payload_bytes: usize,

    pub python_rpc_calls: u64,
    pub python_rpc_avg_ms: f64,
    pub python_rpc_last_ms: f64,
    pub python_rpc_max_ms: f64,
}

impl PerfTelemetry {
    pub fn record_get_state_snapshot(&self, lock_wait_us: u64, duration_us: u64, payload_bytes: usize) {
        self.get_state_calls.fetch_add(1, Ordering::Relaxed);
        self.get_state_snapshot_calls.fetch_add(1, Ordering::Relaxed);
        self.get_state_snapshot_total_duration_us.fetch_add(duration_us, Ordering::Relaxed);
        self.get_state_total_duration_us.fetch_add(duration_us, Ordering::Relaxed);
        self.get_state_last_duration_us.store(duration_us, Ordering::Relaxed);
        self.get_state_last_lock_wait_us.store(lock_wait_us, Ordering::Relaxed);
        self.get_state_last_payload_bytes.store(payload_bytes, Ordering::Relaxed);
        self.get_state_max_duration_us.fetch_max(duration_us, Ordering::Relaxed);
    }

    pub fn record_get_state_noop(&self, lock_wait_us: u64, duration_us: u64) {
        self.get_state_calls.fetch_add(1, Ordering::Relaxed);
        self.get_state_total_duration_us.fetch_add(duration_us, Ordering::Relaxed);
        self.get_state_last_duration_us.store(duration_us, Ordering::Relaxed);
        self.get_state_last_lock_wait_us.store(lock_wait_us, Ordering::Relaxed);
        self.get_state_max_duration_us.fetch_max(duration_us, Ordering::Relaxed);
    }

    pub fn record_python_rpc(&self, duration_us: u64) {
        self.python_rpc_calls.fetch_add(1, Ordering::Relaxed);
        self.python_rpc_total_duration_us.fetch_add(duration_us, Ordering::Relaxed);
        self.python_rpc_last_duration_us.store(duration_us, Ordering::Relaxed);
        self.python_rpc_max_duration_us.fetch_max(duration_us, Ordering::Relaxed);
    }

    pub fn report(&self) -> BackendPerfReport {
        let gs_calls = self.get_state_calls.load(Ordering::Relaxed);
        let gs_snap_calls = self.get_state_snapshot_calls.load(Ordering::Relaxed);
        let gs_snap_total = self.get_state_snapshot_total_duration_us.load(Ordering::Relaxed);
        let gs_total = self.get_state_total_duration_us.load(Ordering::Relaxed);
        let gs_last = self.get_state_last_duration_us.load(Ordering::Relaxed);
        let gs_max = self.get_state_max_duration_us.load(Ordering::Relaxed);
        let gs_lock = self.get_state_last_lock_wait_us.load(Ordering::Relaxed);
        let gs_bytes = self.get_state_last_payload_bytes.load(Ordering::Relaxed);

        let py_calls = self.python_rpc_calls.load(Ordering::Relaxed);
        let py_total = self.python_rpc_total_duration_us.load(Ordering::Relaxed);
        let py_last = self.python_rpc_last_duration_us.load(Ordering::Relaxed);
        let py_max = self.python_rpc_max_duration_us.load(Ordering::Relaxed);

        BackendPerfReport {
            get_state_calls: gs_calls,
            get_state_snapshot_calls: gs_snap_calls,
            get_state_snapshot_avg_ms: if gs_snap_calls > 0 { (gs_snap_total as f64 / gs_snap_calls as f64) / 1000.0 } else { 0.0 },
            get_state_avg_ms: if gs_calls > 0 { (gs_total as f64 / gs_calls as f64) / 1000.0 } else { 0.0 },
            get_state_last_ms: gs_last as f64 / 1000.0,
            get_state_max_ms: gs_max as f64 / 1000.0,
            get_state_last_lock_wait_ms: gs_lock as f64 / 1000.0,
            get_state_last_payload_bytes: gs_bytes,

            python_rpc_calls: py_calls,
            python_rpc_avg_ms: if py_calls > 0 { (py_total as f64 / py_calls as f64) / 1000.0 } else { 0.0 },
            python_rpc_last_ms: py_last as f64 / 1000.0,
            python_rpc_max_ms: py_max as f64 / 1000.0,
        }
    }
}

fn append_perf_line(log_dir: &PathBuf, line: &str) {
    let _ = std::fs::create_dir_all(log_dir);
    let path = log_dir.join(FILE);
    let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if len > 0 && len + line.len() as u64 > MAX_LOG_BYTES {
        let old = log_dir.join(OLD_FILE);
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, &old);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

pub fn spawn_perf_logger(perf: Arc<PerfTelemetry>, log_dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(Duration::from_secs(60));
        // Skip initial immediate tick
        tick.tick().await;

        let mut prev_gs_calls = 0u64;
        let mut prev_gs_snaps = 0u64;
        let mut prev_py_calls = 0u64;

        loop {
            tick.tick().await;
            let rep = perf.report();
            let gs_delta = rep.get_state_calls.saturating_sub(prev_gs_calls);
            let gs_snap_delta = rep.get_state_snapshot_calls.saturating_sub(prev_gs_snaps);
            let py_delta = rep.python_rpc_calls.saturating_sub(prev_py_calls);
            prev_gs_calls = rep.get_state_calls;
            prev_gs_snaps = rep.get_state_snapshot_calls;
            prev_py_calls = rep.python_rpc_calls;

            let ts = ferric_core::logging::timestamp_utc(SystemTime::now());
            let line = format!(
                "[{}] get_state: calls_min={} (snaps={}), snap_avg={:.2}ms, last={:.2}ms, max={:.2}ms, lock_wait={:.2}ms, last_payload={}B | py_rpc: calls_min={}, avg={:.2}ms, last={:.2}ms, max={:.2}ms\n",
                ts,
                gs_delta,
                gs_snap_delta,
                rep.get_state_snapshot_avg_ms,
                rep.get_state_last_ms,
                rep.get_state_max_ms,
                rep.get_state_last_lock_wait_ms,
                rep.get_state_last_payload_bytes,
                py_delta,
                rep.python_rpc_avg_ms,
                rep.python_rpc_last_ms,
                rep.python_rpc_max_ms,
            );

            let dir = log_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                append_perf_line(&dir, &line);
            }).await;
        }
    });
}
