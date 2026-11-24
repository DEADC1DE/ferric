// python_runtime.rs - Manages the background Python worker subprocess for Ferric.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use ferric_core::mrc::interp::{
    channel_ok, name_ok, nick_ok, query_ok, raw_refusal, targets_ok, EchoTarget, Effect, HaltStatus,
    IrcEffect, Limits, MenuAddress, MenuNode, NodeKind, RunResult, ScriptContext, ScriptEvent,
    MAX_LINE_BYTES,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const HOST_SCRIPT_SRC: &str = include_str!("python_host.py");
const RPC_TIMEOUT: Duration = Duration::from_millis(1000);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonMenuSpec {
    pub id: String,
    pub window: String,
    pub label: String,
    pub prompts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogOp {
    pub op: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
}

pub struct PythonRuntime {
    child: Option<Child>,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: Option<Lines<BufReader<ChildStdout>>>,
    project_dir: PathBuf,
    app_config_dir: PathBuf,
    custom_python_path: Option<String>,
    enabled: bool,
    loaded_scripts: HashSet<String>,
    subscribed_events: HashSet<String>,
    subscribed_aliases: HashSet<String>,
    registered_menus: Vec<PythonMenuSpec>,
    pending_dialog_ops: Vec<DialogOp>,
    stderr_tail: Arc<std::sync::Mutex<VecDeque<String>>>,
    perf: Option<Arc<crate::perf::PerfTelemetry>>,
}

impl PythonRuntime {
    pub fn new(project_dir: PathBuf, app_config_dir: PathBuf) -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            project_dir,
            app_config_dir,
            custom_python_path: None,
            enabled: true,
            loaded_scripts: HashSet::new(),
            subscribed_events: HashSet::new(),
            subscribed_aliases: HashSet::new(),
            registered_menus: Vec::new(),
            pending_dialog_ops: Vec::new(),
            stderr_tail: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(64))),
            perf: None,
        }
    }

    pub fn with_perf(mut self, perf: Arc<crate::perf::PerfTelemetry>) -> Self {
        self.perf = Some(perf);
        self
    }

    pub fn take_dialog_ops(&mut self) -> Vec<DialogOp> {
        std::mem::take(&mut self.pending_dialog_ops)
    }

    fn record_dialog_ops(&mut self, val: &serde_json::Value) {
        let ops = parse_dialog_ops(val);
        if !ops.is_empty() {
            self.pending_dialog_ops.extend(ops);
        }
    }

    pub fn set_custom_python_path(&mut self, path: Option<String>) -> bool {
        if self.custom_python_path != path {
            self.custom_python_path = path;
            self.shutdown();
            self.clear_registrations();
            true
        } else {
            false
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.shutdown();
                self.clear_registrations();
            }
            true
        } else {
            false
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    #[allow(dead_code)]
    pub fn has_loaded_scripts(&self) -> bool {
        !self.loaded_scripts.is_empty()
    }

    #[allow(dead_code)]
    pub fn is_script_loaded(&self, name: &str) -> bool {
        self.loaded_scripts.iter().any(|s| s.eq_ignore_ascii_case(name))
    }

    pub fn has_event(&self, event_name: &str) -> bool {
        self.enabled && (self.subscribed_events.contains("*") || self.subscribed_events.contains(&event_name.to_lowercase()))
    }

    pub fn has_alias(&self, name: &str) -> bool {
        self.enabled && self.subscribed_aliases.contains(&name.to_lowercase())
    }

    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.stdin = None;
        self.stdout = None;
    }

    fn write_host_script(&self) -> Result<PathBuf, String> {
        let dest = self.app_config_dir.join("python_host.py");
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&dest, HOST_SCRIPT_SRC)
            .map_err(|e| format!("Cannot write python_host.py: {e}"))?;
        Ok(dest)
    }

    fn resolve_python_binary(&self) -> Result<PathBuf, String> {
        // 1. Explicit setting or environment variable
        if let Some(ref p) = self.custom_python_path {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                let candidate = PathBuf::from(trimmed);
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
        if let Ok(p) = std::env::var("FERRIC_PYTHON") {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                let candidate = PathBuf::from(trimmed);
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }

        // 2. Portable Python in project dir
        #[cfg(target_os = "windows")]
        let portable = self.project_dir.join("python").join("python.exe");
        #[cfg(not(target_os = "windows"))]
        let portable = self.project_dir.join("python").join("bin").join("python3");

        if portable.exists() {
            return Ok(portable);
        }

        // 3. System Python in PATH
        let candidates = if cfg!(target_os = "windows") {
            vec!["python.exe", "python3.exe", "python", "py", "python3"]
        } else {
            vec!["python3", "python"]
        };

        for candidate in candidates {
            let mut check_cmd = std::process::Command::new(candidate);
            if candidate == "py" {
                check_cmd.arg("-3");
            }
            check_cmd.arg("--version");
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                check_cmd.creation_flags(0x08000000);
            }
            if let Ok(output) = check_cmd.output() {
                if output.status.success() {
                    let out = String::from_utf8_lossy(&output.stdout);
                    let err = String::from_utf8_lossy(&output.stderr);
                    if out.contains("Python 3.") || err.contains("Python 3.") {
                        return Ok(PathBuf::from(candidate));
                    }
                }
            }
        }

        Err("No Python interpreter found. Install Python 3 or specify a path in Options / FERRIC_PYTHON.".to_string())
    }

    async fn ensure_started(&mut self) -> Result<(), String> {
        if !self.enabled {
            return Err("Python scripting is disabled in options.".to_string());
        }

        if let Some(ref mut child) = self.child {
            if let Ok(None) = child.try_wait() {
                if self.stdin.is_some() && self.stdout.is_some() {
                    return Ok(());
                }
            }
        }

        self.shutdown();

        let host_script = self.write_host_script()?;
        let python_bin = self.resolve_python_binary()?;

        let mut cmd = Command::new(&python_bin);
        if python_bin == std::path::Path::new("py")
            || python_bin.file_name().and_then(|f| f.to_str()) == Some("py")
            || python_bin.file_name().and_then(|f| f.to_str()) == Some("py.exe")
        {
            cmd.arg("-3");
        }
        cmd.arg("-u")
            .arg("-X")
            .arg("utf8")
            .arg(&host_script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(target_os = "windows")]
        {
            // CREATE_NO_WINDOW = 0x08000000
            cmd.creation_flags(0x08000000);
        }

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn Python process ({:?}): {e}", python_bin))?;

        let stdin = child.stdin.take().ok_or("Failed to capture child stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to capture child stdout")?;
        let stderr = child.stderr.take();

        if let Some(stderr) = stderr {
            let tail = Arc::clone(&self.stderr_tail);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut lock) = tail.lock() {
                        if lock.len() >= 64 {
                            lock.pop_front();
                        }
                        lock.push_back(line);
                    }
                }
            });
        }

        self.child = Some(child);
        self.stdin = Some(BufWriter::new(stdin));
        self.stdout = Some(BufReader::new(stdout).lines());

        // Ping check
        let ping_req = serde_json::json!({ "cmd": "ping" });
        let resp = self.send_request(&ping_req).await?;
        if resp.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            self.shutdown();
            return Err("Python worker failed ping handshake.".to_string());
        }

        Ok(())
    }

    async fn send_request(&mut self, req: &serde_json::Value) -> Result<serde_json::Value, String> {
        let t0 = std::time::Instant::now();
        let line = serde_json::to_string(req).map_err(|e| e.to_string())? + "\n";

        let stdin = self.stdin.as_mut().ok_or("Python worker stdin not connected")?;
        let stdout = self.stdout.as_mut().ok_or("Python worker stdout not connected")?;

        let send_and_recv = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
            let resp_line = stdout.next_line().await?.ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Python worker closed output stream")
            })?;
            serde_json::from_str::<serde_json::Value>(&resp_line)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        };

        let res = match tokio::time::timeout(RPC_TIMEOUT, send_and_recv).await {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(io_err)) => {
                let recent_stderr = self.get_recent_stderr();
                self.shutdown();
                self.clear_registrations();
                Err(format!("Python worker communication error: {io_err}. {recent_stderr}"))
            }
            Err(_) => {
                let recent_stderr = self.get_recent_stderr();
                self.shutdown();
                self.clear_registrations();
                Err(format!("Python script worker timed out (>{:?}) and was stopped. {recent_stderr}", RPC_TIMEOUT))
            }
        };

        if let Some(ref perf) = self.perf {
            perf.record_python_rpc(t0.elapsed().as_micros() as u64);
        }

        res
    }

    fn get_recent_stderr(&self) -> String {
        if let Ok(lock) = self.stderr_tail.lock() {
            if !lock.is_empty() {
                let lines: Vec<&str> = lock.iter().map(|s| s.as_str()).collect();
                return format!("Recent stderr: {}", lines.join(" | "));
            }
        }
        String::new()
    }

    fn clear_registrations(&mut self) {
        self.loaded_scripts.clear();
        self.subscribed_events.clear();
        self.subscribed_aliases.clear();
        self.registered_menus.clear();
    }

    pub async fn load(&mut self, name: &str, content: &str) -> Result<(Vec<String>, RunResult), String> {
        self.ensure_started().await?;

        let req = serde_json::json!({
            "cmd": "load",
            "name": name,
            "content": content,
        });

        let resp = self.send_request(&req).await?;
        let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

        if !ok {
            let notices = parse_notices(&resp);
            return Err(if !notices.is_empty() {
                notices.join("\n")
            } else {
                resp.get("error").and_then(|v| v.as_str()).unwrap_or("Failed to load python script").to_string()
            });
        }

        self.loaded_scripts.insert(name.to_lowercase());

        self.subscribed_events = resp
            .get("events")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                    .collect()
            })
            .unwrap_or_default();

        self.subscribed_aliases = resp
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                    .collect()
            })
            .unwrap_or_default();

        self.registered_menus = parse_menus(&resp);

        self.record_dialog_ops(&resp);
        let mut result = convert_json_to_run_result(&resp);
        let notices = std::mem::take(&mut result.notices);

        Ok((notices, result))
    }

    pub async fn unload(&mut self, name: &str) -> Result<(bool, RunResult), String> {
        let key = name.to_lowercase();
        if !self.loaded_scripts.contains(&key) {
            return Ok((false, RunResult::default()));
        }

        let mut result = RunResult::default();
        if self.child.is_some() {
            let req = serde_json::json!({
                "cmd": "unload",
                "name": name,
            });
            if let Ok(resp) = self.send_request(&req).await {
                self.subscribed_events = resp
                    .get("events")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                            .collect()
                    })
                    .unwrap_or_default();

                self.subscribed_aliases = resp
                    .get("aliases")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                            .collect()
                    })
                    .unwrap_or_default();

                self.registered_menus = parse_menus(&resp);
                self.record_dialog_ops(&resp);
                result = convert_json_to_run_result(&resp);
            }
        }

        self.loaded_scripts.remove(&key);

        // If no more python scripts are loaded, shut down worker to save RAM
        if self.loaded_scripts.is_empty() {
            self.shutdown();
            self.clear_registrations();
        }

        Ok((true, result))
    }

    pub async fn dispatch_event(&mut self, event: &ScriptEvent, context: &ScriptContext) -> RunResult {
        if !self.enabled || self.child.is_none() {
            return RunResult::default();
        }

        let event_type = match event {
            ScriptEvent::Text { .. } => "text",
            ScriptEvent::Action { .. } => "action",
            ScriptEvent::Notice { .. } => "notice",
            ScriptEvent::Join { .. } => "join",
            ScriptEvent::Part { .. } => "part",
            ScriptEvent::Quit { .. } => "quit",
            ScriptEvent::Nick { .. } => "nick",
            ScriptEvent::CtcpReply { .. } => "ctcpreply",
            ScriptEvent::Connect { .. } => "connect",
            ScriptEvent::Disconnect { .. } => "disconnect",
            ScriptEvent::Error { .. } => "error",
            ScriptEvent::Kick { .. } => "kick",
            ScriptEvent::Topic { .. } => "topic",
            ScriptEvent::Mode { .. } => "mode",
            ScriptEvent::UserMode { .. } => "usermode",
            ScriptEvent::ModeChange { event, .. } => *event,
        };

        if !self.has_event(event_type) {
            return RunResult::default();
        }

        let event_val = match serde_json::to_value(event) {
            Ok(serde_json::Value::Object(map)) => {
                if map.len() == 1 {
                    if let Some((_, serde_json::Value::Object(inner))) = map.into_iter().next() {
                        serde_json::Value::Object(inner)
                    } else {
                        serde_json::Value::Object(serde_json::Map::new())
                    }
                } else {
                    serde_json::Value::Object(map)
                }
            }
            Ok(v) => v,
            Err(_) => serde_json::Value::Object(serde_json::Map::new()),
        };

        let req = serde_json::json!({
            "cmd": "event",
            "event_type": event_type,
            "event": event_val,
            "context": context,
        });

        match self.send_request(&req).await {
            Ok(resp) => {
                self.record_dialog_ops(&resp);
                convert_json_to_run_result(&resp)
            }
            Err(err) => RunResult {
                effects: Vec::new(),
                notices: vec![err],
                halt_status: HaltStatus::default(),
            },
        }
    }

    #[allow(dead_code)]
    pub async fn run_alias(&mut self, name: &str, args: &[String], context: &ScriptContext) -> RunResult {
        self.run_alias_with_state(name, args, context, None).await
    }

    pub async fn run_alias_with_state(
        &mut self,
        name: &str,
        args: &[String],
        context: &ScriptContext,
        channel_state: Option<&serde_json::Value>,
    ) -> RunResult {
        if !self.enabled || self.child.is_none() {
            return RunResult::default();
        }

        let mut req = serde_json::json!({
            "cmd": "alias",
            "name": name,
            "args": args,
            "context": context,
        });
        if let Some(cs) = channel_state {
            req["channel_state"] = cs.clone();
        }

        match self.send_request(&req).await {
            Ok(resp) => {
                self.record_dialog_ops(&resp);
                convert_json_to_run_result(&resp)
            }
            Err(err) => RunResult {
                effects: Vec::new(),
                notices: vec![err],
                halt_status: HaltStatus::default(),
            },
        }
    }

    pub async fn dispatch_dialog_event(
        &mut self,
        dialog_id: &str,
        event: &serde_json::Value,
        context: &ScriptContext,
        channel_state: Option<&serde_json::Value>,
    ) -> RunResult {
        if !self.enabled || self.child.is_none() {
            return RunResult::default();
        }

        let mut req = serde_json::json!({
            "cmd": "dialog_event",
            "id": dialog_id,
            "event": event,
            "context": context,
        });
        if let Some(cs) = channel_state {
            req["channel_state"] = cs.clone();
        }

        match self.send_request(&req).await {
            Ok(resp) => {
                self.record_dialog_ops(&resp);
                convert_json_to_run_result(&resp)
            }
            Err(err) => RunResult {
                effects: Vec::new(),
                notices: vec![err],
                halt_status: HaltStatus::default(),
            },
        }
    }

    pub async fn menu_nodes(&mut self, window: &str, context: &ScriptContext, args: &[String]) -> Vec<MenuNode> {
        if !self.enabled || self.child.is_none() {
            return Vec::new();
        }

        let win = window.trim().to_ascii_lowercase();
        if !self.registered_menus.iter().any(|m| m.window == win || m.window == "*") {
            return Vec::new();
        }

        let req = serde_json::json!({
            "cmd": "menu_nodes",
            "window": window,
            "context": context,
            "args": args,
        });

        let Ok(resp) = self.send_request(&req).await else {
            return Vec::new();
        };

        let Some(nodes_arr) = resp.get("nodes").and_then(|v| v.as_array()) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for item in nodes_arr {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else { continue };
            let label = item.get("label").and_then(|v| v.as_str()).unwrap_or("");
            let prompts: Vec<String> = item
                .get("prompts")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
                .unwrap_or_default();

            out.push(MenuNode {
                kind: NodeKind::Command,
                label: label.to_string(),
                address: Some(MenuAddress {
                    version: 0,
                    menu: 0,
                    path: Vec::new(),
                    python_id: Some(id.to_string()),
                }),
                prompts,
                children: Vec::new(),
            });
        }

        out
    }

    #[allow(dead_code)]
    pub async fn run_menu_item(
        &mut self,
        id: &str,
        context: &ScriptContext,
        args: &[String],
        answers: Vec<String>,
    ) -> Result<RunResult, String> {
        self.run_menu_item_with_state(id, context, args, answers, None).await
    }

    pub async fn run_menu_item_with_state(
        &mut self,
        id: &str,
        context: &ScriptContext,
        args: &[String],
        answers: Vec<String>,
        channel_state: Option<&serde_json::Value>,
    ) -> Result<RunResult, String> {
        if !self.enabled || self.child.is_none() {
            return Err("Python runtime is not active".to_string());
        }

        let mut req = serde_json::json!({
            "cmd": "run_menu_item",
            "id": id,
            "context": context,
            "args": args,
            "answers": answers,
        });
        if let Some(cs) = channel_state {
            req["channel_state"] = cs.clone();
        }

        let resp = self.send_request(&req).await?;
        self.record_dialog_ops(&resp);
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let notices = parse_notices(&resp);
            let err_msg = notices.into_iter().next().unwrap_or_else(|| format!("Menu item '{id}' not found"));
            return Err(err_msg);
        }
        Ok(convert_json_to_run_result(&resp))
    }
}

pub fn parse_dialog_ops(val: &serde_json::Value) -> Vec<DialogOp> {
    let Some(arr) = val.get("dialog_ops").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(op) = item.get("op").and_then(|v| v.as_str()) else { continue };
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else { continue };
        let clean_op = op.trim().to_ascii_lowercase();
        if clean_op != "open" && clean_op != "update" && clean_op != "close" {
            continue;
        }
        let clean_id = id.trim();
        if clean_id.is_empty() || clean_id.len() > 64 || !clean_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let title = item.get("title").and_then(|v| v.as_str()).map(|t| safe_truncate_str(t, 256).to_string());
        let spec = item.get("spec").cloned();
        if let Some(ref s) = spec {
            if let Ok(json_str) = serde_json::to_string(s) {
                if json_str.len() > 524_288 {
                    continue;
                }
            }
        }
        out.push(DialogOp {
            op: clean_op,
            id: clean_id.to_string(),
            title,
            spec,
        });
    }
    out
}

fn parse_notices(val: &serde_json::Value) -> Vec<String> {
    val.get("notices")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn parse_menus(val: &serde_json::Value) -> Vec<PythonMenuSpec> {
    val.get("menus")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| serde_json::from_value::<PythonMenuSpec>(m.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

fn convert_json_to_run_result(val: &serde_json::Value) -> RunResult {
    let limits = Limits::default();
    let mut effects = Vec::new();
    let mut notices = parse_notices(val);

    let mut echo_count = 0;
    let mut irc_count = 0;

    let Some(eff_array) = val.get("effects").and_then(|v| v.as_array()) else {
        return RunResult {
            effects,
            notices,
            halt_status: HaltStatus::default(),
        };
    };

    for item in eff_array {
        let Some(typ) = item.get("type").and_then(|v| v.as_str()) else {
            continue;
        };

        match typ {
            "echo" => {
                if echo_count >= limits.max_echo_lines {
                    continue;
                }
                echo_count += 1;
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let text = safe_truncate_str(text, limits.max_string_bytes);
                let target_str = item.get("target").and_then(|v| v.as_str()).unwrap_or("default");
                let target = match target_str {
                    "active" | "-a" => EchoTarget::Active,
                    "status" | "-s" => EchoTarget::Status,
                    _ => EchoTarget::Default,
                };
                effects.push(Effect::EchoTarget {
                    target,
                    text: text.to_string(),
                });
            }
            "msg" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if !targets_ok(target) || text.len() > limits.max_string_bytes || text.contains(['\r', '\n', '\0']) {
                    notices.push(format!("Python msg: invalid target or text: {target}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Msg {
                    target: target.to_string(),
                    text: text.to_string(),
                }));
            }
            "notice" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if !targets_ok(target) || text.len() > limits.max_string_bytes || text.contains(['\r', '\n', '\0']) {
                    notices.push(format!("Python notice: invalid target or text: {target}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Notice {
                    target: target.to_string(),
                    text: text.to_string(),
                }));
            }
            "action" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if !targets_ok(target) || text.len() > limits.max_string_bytes || text.contains(['\r', '\n', '\0']) {
                    notices.push(format!("Python action: invalid target or text: {target}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Action {
                    target: target.to_string(),
                    text: text.to_string(),
                }));
            }
            "join" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                let key = item.get("key").and_then(|v| v.as_str()).map(str::to_string);
                if !channel_ok(channel) {
                    notices.push(format!("Python join: invalid channel name: {channel}"));
                    continue;
                }
                if let Some(ref k) = key {
                    if !name_ok(k) {
                        notices.push(format!("Python join: invalid key: {k}"));
                        continue;
                    }
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Join {
                    channel: channel.to_string(),
                    key,
                }));
            }
            "part" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                let reason = item.get("reason").and_then(|v| v.as_str()).map(str::to_string);
                if !channel_ok(channel) {
                    notices.push(format!("Python part: invalid channel name: {channel}"));
                    continue;
                }
                if let Some(ref r) = reason {
                    if r.len() > limits.max_string_bytes || r.contains(['\r', '\n', '\0']) {
                        notices.push(format!("Python part: invalid reason: {r}"));
                        continue;
                    }
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Part {
                    channel: channel.to_string(),
                    reason,
                }));
            }
            "kick" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
                let nick = item.get("nick").and_then(|v| v.as_str()).unwrap_or("");
                let reason = item.get("reason").and_then(|v| v.as_str()).map(str::to_string);
                if !channel_ok(channel) || !nick_ok(nick) {
                    notices.push(format!("Python kick: invalid channel or nick: {channel} {nick}"));
                    continue;
                }
                if let Some(ref r) = reason {
                    if r.len() > limits.max_string_bytes || r.contains(['\r', '\n', '\0']) {
                        notices.push(format!("Python kick: invalid reason: {r}"));
                        continue;
                    }
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Kick {
                    channel: channel.to_string(),
                    nick: nick.to_string(),
                    reason,
                }));
            }
            "mode" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                let modes = item.get("modes").and_then(|v| v.as_str()).unwrap_or("");
                if !name_ok(target) || modes.trim().is_empty() || modes.contains(['\r', '\n', '\0']) {
                    notices.push(format!("Python mode: invalid target or modes: {target}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Mode {
                    target: target.to_string(),
                    modes: modes.to_string(),
                }));
            }
            "nick" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let nick = item.get("nick").and_then(|v| v.as_str()).unwrap_or("");
                if !nick_ok(nick) {
                    notices.push(format!("Python nick: invalid nick: {nick}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Nick(nick.to_string())));
            }
            "whois" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let nick = item.get("nick").and_then(|v| v.as_str()).unwrap_or("");
                if !nick_ok(nick) {
                    notices.push(format!("Python whois: invalid nick: {nick}"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Whois(nick.to_string())));
            }
            "query" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let nick = item.get("nick").and_then(|v| v.as_str()).unwrap_or("");
                if !query_ok(nick) {
                    notices.push(format!("Python query: invalid nick: {nick}"));
                    continue;
                }
                let text = item.get("text").and_then(|v| v.as_str()).map(str::to_string);
                if let Some(ref t) = text {
                    if t.len() > limits.max_string_bytes || t.contains(['\r', '\n', '\0']) {
                        notices.push(format!("Python query: invalid text: {t}"));
                        continue;
                    }
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Query {
                    nick: nick.to_string(),
                    text,
                }));
            }
            "raw" => {
                if irc_count >= limits.max_outbound {
                    continue;
                }
                let line = item.get("line").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(reason) = raw_refusal(line) {
                    notices.push(format!("Python raw rejected: {reason}"));
                    continue;
                }
                if line.len() > MAX_LINE_BYTES {
                    notices.push(format!("Python raw rejected: line exceeds {MAX_LINE_BYTES} bytes"));
                    continue;
                }
                irc_count += 1;
                effects.push(Effect::Irc(IrcEffect::Raw(line.to_string())));
            }
            _ => {}
        }
    }

    RunResult {
        effects,
        notices,
        halt_status: HaltStatus::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_json_utf8_truncation() {
        let limits = Limits::default();
        // Construct string where byte limits.max_string_bytes falls inside a 4-byte emoji: 🦀 (4 bytes)
        let mut text = "a".repeat(limits.max_string_bytes - 2);
        text.push_str("🦀🦀"); // spans across the boundary

        let json = serde_json::json!({
            "effects": [
                { "type": "echo", "text": text }
            ]
        });

        let res = convert_json_to_run_result(&json);
        assert_eq!(res.effects.len(), 1);
        if let Effect::EchoTarget { text: out_text, .. } = &res.effects[0] {
            assert!(out_text.len() <= limits.max_string_bytes);
            // Verify valid UTF-8 and no panic
            assert!(!out_text.is_empty());
        } else {
            panic!("Expected EchoTarget");
        }
    }

    #[test]
    fn test_convert_json_query_and_reasons_validation() {
        let limits = Limits::default();

        // 1. query counts against max_outbound
        let mut effects = Vec::new();
        for i in 0..(limits.max_outbound + 10) {
            effects.push(serde_json::json!({
                "type": "query",
                "nick": format!("nick{}", i)
            }));
        }
        let json = serde_json::json!({ "effects": effects });
        let res = convert_json_to_run_result(&json);
        assert_eq!(res.effects.len(), limits.max_outbound);

        // 2. CRLF in part reason, kick reason, query text
        let bad_json = serde_json::json!({
            "effects": [
                { "type": "part", "channel": "#lobby", "reason": "bye\r\nQUIT :pwn" },
                { "type": "kick", "channel": "#lobby", "nick": "target", "reason": "bye\0evil" },
                { "type": "query", "nick": "friend", "text": "hello\nsecret" }
            ]
        });
        let bad_res = convert_json_to_run_result(&bad_json);
        assert!(bad_res.effects.is_empty());
        assert_eq!(bad_res.notices.len(), 3);
    }

    #[test]
    fn test_convert_json_raw_refusal() {
        let json = serde_json::json!({
            "effects": [
                { "type": "raw", "line": "PASS secret" },
                { "type": "raw", "line": "OPER admin secret" },
                { "type": "raw", "line": "AUTHENTICATE PLAIN" },
                { "type": "raw", "line": "PRIVMSG #test :hello\r\nQUIT" },
                { "type": "raw", "line": "NOTICE #test :valid line" }
            ]
        });
        let res = convert_json_to_run_result(&json);
        assert_eq!(res.effects.len(), 1); // Only the valid notice
        assert_eq!(res.notices.len(), 4);
    }

    #[tokio::test]
    async fn test_python_lifecycle_and_effects() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-test-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric

@ferric.alias("greet")
def handle_greet(args, ctx):
    name = args[0] if args else "world"
    ferric.echo(f"Hello, {name}!")

@ferric.on("text")
def handle_text(ev, ctx):
    text = ev.get("text", "")
    if "ping" in text:
        ferric.msg(ctx.get("chan") or ctx.get("target") or "#lobby", "pong")
"##;

        let res = rt.load("test.py", py_code).await;
        if let Err(e) = res {
            eprintln!("Skipping test: python interpreter not available: {e}");
            return;
        }

        assert!(rt.has_alias("greet"));
        assert!(rt.has_event("text"));

        let ctx = ScriptContext {
            network: Some("freenode".into()),
            chan: Some("#lobby".into()),
            target: Some("#lobby".into()),
            me: Some("testbot".into()),
            ..Default::default()
        };
        let run_res = rt.run_alias("greet", &["Alice".into()], &ctx).await;
        assert_eq!(run_res.effects.len(), 1);
        match &run_res.effects[0] {
            Effect::EchoTarget { text, .. } => assert_eq!(text, "Hello, Alice!"),
            _ => panic!("Expected Effect::EchoTarget, got {:?}", run_res.effects[0]),
        }

        let ev = ScriptEvent::Text {
            network: "freenode".into(),
            target: "#lobby".into(),
            nick: "bob".into(),
            text: "ping please".into(),
            address: None,
            me: "testbot".into(),
        };
        let ev_res = rt.dispatch_event(&ev, &ctx).await;
        assert_eq!(ev_res.effects.len(), 1);
        match &ev_res.effects[0] {
            Effect::Irc(IrcEffect::Msg { target, text }) => {
                assert_eq!(target, "#lobby");
                assert_eq!(text, "pong");
            }
            _ => panic!("Expected Effect::Irc(Msg)"),
        }

        let unload_res = rt.unload("test.py").await;
        assert!(unload_res.is_ok());
        assert!(!rt.has_alias("greet"));
        assert!(!rt.has_event("text"));
        assert!(!rt.is_running());
    }

    #[tokio::test]
    async fn test_python_multi_script_unload_aliases() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-multi-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code_a = r##"
import ferric
@ferric.alias("cmd_a")
def handle_a(args, ctx):
    ferric.echo("A")
"##;

        let code_b = r##"
import ferric
@ferric.alias("cmd_b")
def handle_b(args, ctx):
    ferric.echo("B")
"##;

        if rt.load("a.py", code_a).await.is_err() {
            return;
        }
        if rt.load("b.py", code_b).await.is_err() {
            return;
        }

        assert!(rt.has_alias("cmd_a"));
        assert!(rt.has_alias("cmd_b"));

        // Unload a.py: cmd_a must be gone, cmd_b must stay!
        let _ = rt.unload("a.py").await;
        assert!(!rt.has_alias("cmd_a"));
        assert!(rt.has_alias("cmd_b"));

        // Unload b.py: both gone, worker shuts down
        let _ = rt.unload("b.py").await;
        assert!(!rt.has_alias("cmd_b"));
        assert!(!rt.is_running());
    }

    #[tokio::test]
    async fn test_python_print_and_caps() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-test2-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric

@ferric.alias("spammer")
def handle_spam(args, ctx):
    print("Normal python stdout debug line that should not corrupt JSON")
    for i in range(250):
        ferric.echo(f"line {i}")
"##;

        let res = rt.load("spam.py", py_code).await;
        if res.is_err() {
            return;
        }

        let ctx = ScriptContext::default();
        let run_res = rt.run_alias("spammer", &[], &ctx).await;
        assert!(run_res.effects.len() <= 200);

        let _ = rt.unload("spam.py").await;
    }

    #[tokio::test]
    async fn test_python_timeout_recovery() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-test3-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric
import time

@ferric.alias("hang")
def handle_hang(args, ctx):
    while True:
        time.sleep(0.1)

@ferric.alias("fine")
def handle_fine(args, ctx):
    ferric.echo("I am fine")
"##;

        let res = rt.load("hang.py", py_code).await;
        if res.is_err() {
            return;
        }

        let ctx = ScriptContext::default();
        let run_res = rt.run_alias("hang", &[], &ctx).await;
        assert!(run_res.notices.iter().any(|n| n.contains("timeout") || n.contains("timed out") || n.contains("terminated")));
        assert!(!rt.is_running());

        let res2 = rt.load("hang.py", py_code).await;
        assert!(res2.is_ok());
        let fine_res = rt.run_alias("fine", &[], &ctx).await;
        assert_eq!(fine_res.effects.len(), 1);

        let _ = rt.unload("hang.py").await;
    }

    #[tokio::test]
    async fn test_python_menu() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-test4-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric

@ferric.menu("channel", "Slap User")
def on_slap(ctx, args, answers):
    ferric.action(ctx.get("chan", "#lobby"), f"slaps {args[0] if args else 'someone'} around with a trout")
"##;

        let res = rt.load("menu.py", py_code).await;
        if res.is_err() {
            return;
        }

        let ctx = ScriptContext {
            chan: Some("#lobby".into()),
            ..Default::default()
        };
        let nodes = rt.menu_nodes("channel", &ctx, &["troll".into()]).await;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].label, "Slap User");

        let addr = nodes[0].address.as_ref().expect("Expected address");
        let py_id = addr.python_id.as_deref().expect("Expected python_id");
        let res = rt.run_menu_item(py_id, &ctx, &["troll".into()], Vec::new()).await.expect("run_menu_item failed");
        assert_eq!(res.effects.len(), 1);
        match &res.effects[0] {
            Effect::Irc(IrcEffect::Action { target, text }) => {
                assert_eq!(target, "#lobby");
                assert_eq!(text, "slaps troll around with a trout");
            }
            _ => panic!("Expected Effect::Irc(Action)"),
        }

        let _ = rt.unload("menu.py").await;
    }

    #[tokio::test]
    async fn test_python_invoke_fn_no_retry_on_exception() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-err-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric

@ferric.alias("crash")
def handle_crash(args, ctx):
    ferric.echo("called")
    raise ValueError("intentional error")
"##;

        let res = rt.load("crash.py", py_code).await;
        if res.is_err() {
            return;
        }

        let ctx = ScriptContext::default();
        let run_res = rt.run_alias("crash", &[], &ctx).await;
        // Verify handle_crash was only called once (single echo effect)
        assert_eq!(run_res.effects.len(), 1);
        assert!(run_res.notices.iter().any(|n| n.contains("intentional error")));

        let _ = rt.unload("crash.py").await;
    }

    #[tokio::test]
    async fn test_python_flat_event_and_reason_modes_param_matching() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-events-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let py_code = r##"
import ferric

@ferric.on("kick", match="*bad*")
def on_kick(ev, ctx):
    # Ensure event is flat dict and fields are accessible
    ferric.echo(f"kicked {ev['knick']} reason={ev['reason']}")

@ferric.on("mode", match="+o*")
def on_mode(ev, ctx):
    ferric.echo(f"mode on {ev['channel']}: {ev['modes']}")

@ferric.on("op", match="targetnick")
def on_op(ev, ctx):
    ferric.echo(f"op on {ev['param']}")
"##;

        let res = rt.load("evtest.py", py_code).await;
        if res.is_err() {
            return;
        }

        // 1. Kick event matching reason "*bad*"
        let ev_kick = ScriptEvent::Kick {
            nick: "admin".into(),
            knick: "spammer".into(),
            channel: "#test".into(),
            reason: Some("bad behavior".into()),
            network: "net".into(),
            me: "me".into(),
        };
        let ctx = ev_kick.context();
        let res_kick = rt.dispatch_event(&ev_kick, &ctx).await;
        assert_eq!(res_kick.effects.len(), 1);
        match &res_kick.effects[0] {
            Effect::EchoTarget { text, .. } => assert_eq!(text, "kicked spammer reason=bad behavior"),
            _ => panic!("Expected EchoTarget"),
        }

        // 2. Mode event matching modes "+o*"
        let ev_mode = ScriptEvent::Mode {
            nick: "admin".into(),
            channel: "#test".into(),
            modes: "+o targetnick".into(),
            network: "net".into(),
            me: "me".into(),
        };
        let ctx_mode = ev_mode.context();
        let res_mode = rt.dispatch_event(&ev_mode, &ctx_mode).await;
        assert_eq!(res_mode.effects.len(), 1);
        match &res_mode.effects[0] {
            Effect::EchoTarget { text, .. } => assert_eq!(text, "mode on #test: +o targetnick"),
            _ => panic!("Expected EchoTarget"),
        }

        // 3. ModeChange event matching param "targetnick"
        let ev_op = ScriptEvent::ModeChange {
            event: "op",
            nick: "admin".into(),
            channel: "#test".into(),
            param: "targetnick".into(),
            network: "net".into(),
            me: "me".into(),
        };
        let ctx_op = ev_op.context();
        let res_op = rt.dispatch_event(&ev_op, &ctx_op).await;
        assert_eq!(res_op.effects.len(), 1);
        match &res_op.effects[0] {
            Effect::EchoTarget { text, .. } => assert_eq!(text, "op on targetnick"),
            _ => panic!("Expected EchoTarget"),
        }

        let _ = rt.unload("evtest.py").await;
    }

    #[tokio::test]
    async fn test_python_reload_syntax_error_preserves_old() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-reload-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code_valid = r##"
import ferric
@ferric.alias("keepme")
def handle_valid(args, ctx):
    ferric.echo("still alive")
"##;

        let code_broken = r##"
this is totally invalid python code syntax !!!
"##;

        let load1 = rt.load("atomic.py", code_valid).await;
        if load1.is_err() {
            return;
        }
        assert!(rt.has_alias("keepme"));

        // Attempt reload with invalid syntax
        let load2 = rt.load("atomic.py", code_broken).await;
        assert!(load2.is_err());

        // Alias from valid script must still be present and working
        assert!(rt.has_alias("keepme"));
        let ctx = ScriptContext::default();
        let run_res = rt.run_alias("keepme", &[], &ctx).await;
        assert_eq!(run_res.effects.len(), 1);

        let _ = rt.unload("atomic.py").await;
    }

    #[tokio::test]
    async fn test_python_case_insensitive_load_unload() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-case-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = r##"
import ferric
@ferric.alias("mycase")
def handle_case(args, ctx):
    ferric.echo("case ok")
"##;

        if rt.load("MyScript.PY", code).await.is_err() {
            return;
        }
        assert!(rt.has_alias("mycase"));

        // Unload with lowercase
        let unload_res = rt.unload("myscript.py").await;
        assert!(unload_res.is_ok());
        assert!(!rt.has_alias("mycase"));
        assert!(!rt.is_running());
    }

    #[tokio::test]
    async fn test_python_menu_not_found() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-menuerr-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = "import ferric";
        if rt.load("empty.py", code).await.is_err() {
            return;
        }

        let ctx = ScriptContext::default();
        let res = rt.run_menu_item("nonexistent:999", &ctx, &[], Vec::new()).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("not found"));

        let _ = rt.unload("empty.py").await;
    }

    #[tokio::test]
    async fn test_python_load_clean_notices_and_unload_flag() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-clean-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = r##"
import ferric
@ferric.on_load
def init():
    ferric.echo("booted")
"##;
        let Ok((notices, run_res)) = rt.load("clean.py", code).await else {
            return;
        };
        assert!(notices.is_empty(), "clean load must not produce warning notices: {notices:?}");
        assert_eq!(run_res.effects.len(), 1);
        assert!(run_res.notices.is_empty());

        let (unloaded_missing, _) = rt.unload("does_not_exist.py").await.unwrap();
        assert!(!unloaded_missing);

        let (unloaded_ok, _) = rt.unload("clean.py").await.unwrap();
        assert!(unloaded_ok);
    }

    #[tokio::test]
    async fn test_python_on_load_rejected_effect_surfaces_notice() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-onload-err-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = r##"
import ferric
@ferric.on_load
def init():
    ferric.raw("PASS secret")
"##;
        let Ok((notices, run_res)) = rt.load("bad_onload.py", code).await else {
            return;
        };
        assert_eq!(notices.len(), 1, "expected raw rejection notice, got {notices:?}");
        assert!(notices[0].contains("PASS"));
        assert!(run_res.effects.is_empty());
        assert!(run_res.notices.is_empty());

        let _ = rt.unload("bad_onload.py").await;
    }

    #[tokio::test]
    async fn test_python_invoke_fn_keyword_only_param() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-kwonly-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = r##"
import ferric
@ferric.alias("kwtest")
def handle_kw(args, *, debug=False):
    ferric.echo(f"ok:{len(args)}:{debug}")
"##;
        if rt.load("kw.py", code).await.is_err() {
            return;
        }
        let res = rt.run_alias("kwtest", &["x".into()], &ScriptContext::default()).await;
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(res.effects.len(), 1);
        match &res.effects[0] {
            Effect::EchoTarget { text, .. } => assert_eq!(text, "ok:1:False"),
            other => panic!("unexpected effect: {other:?}"),
        }
        let _ = rt.unload("kw.py").await;
    }

    #[tokio::test]
    async fn test_python_dialog_lifecycle_and_auto_close() {
        let temp_dir = std::env::temp_dir().join(format!("ferric-py-dialog-{}", std::process::id()));
        let mut rt = PythonRuntime::new(temp_dir.clone(), temp_dir.clone());

        let code = r##"
import ferric

@ferric.alias("opendlg")
def open_dlg(args, ctx):
    ferric.dialog_open("test_dlg", "Test Title", {"controls": [{"type": "button", "id": "btn1"}]})

@ferric.on_dialog("test_dlg")
def handle_dlg(ev, ctx):
    if ev.get("type") == "click" and ev.get("control") == "btn1":
        ferric.echo("button clicked")
        ferric.dialog_update("test_dlg", title="Updated Title")
    elif ev.get("type") == "close_btn":
        ferric.dialog_close("test_dlg")
"##;
        if rt.load("dlg_test.py", code).await.is_err() {
            return;
        }

        // Open dialog via alias
        let ctx = ScriptContext::default();
        let _ = rt.run_alias("opendlg", &[], &ctx).await;
        let ops = rt.take_dialog_ops();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op, "open");
        assert_eq!(ops[0].id, "test_dlg");
        assert_eq!(ops[0].title.as_deref(), Some("Test Title"));
        assert!(ops[0].spec.is_some());

        // Dispatch button click event
        let click_ev = serde_json::json!({ "type": "click", "control": "btn1" });
        let run_res = rt.dispatch_dialog_event("test_dlg", &click_ev, &ctx, None).await;
        assert_eq!(run_res.effects.len(), 1);
        let update_ops = rt.take_dialog_ops();
        assert_eq!(update_ops.len(), 1);
        assert_eq!(update_ops[0].op, "update");
        assert_eq!(update_ops[0].id, "test_dlg");
        assert_eq!(update_ops[0].title.as_deref(), Some("Updated Title"));

        // Unload script: must automatically emit close for "test_dlg"
        let (unloaded, _) = rt.unload("dlg_test.py").await.unwrap();
        assert!(unloaded);
        let close_ops = rt.take_dialog_ops();
        assert_eq!(close_ops.len(), 1);
        assert_eq!(close_ops[0].op, "close");
        assert_eq!(close_ops[0].id, "test_dlg");
    }
}
