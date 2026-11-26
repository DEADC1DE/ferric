// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State, WindowEvent};
use tokio::sync::{mpsc, Mutex};

use ferric_core::server_list::{rows_from_text, text_with_rows, ServerRow};
use ferric_core::{
    apply_server_meta, delete_blow_ini_key, is_python_script, list_blow_ini_entries, load_server_meta, parse_blow_ini,
    parse_connectd_mrc, save_blow_ini_key, AppState, BlowEntry, BufferState, ConnectConfig,
    IrcClient, IrcEvent, NetworkState, ScriptContent, ScriptError, ScriptFileInfo, ScriptStore,
};
use ferric_core::crypto::dh1080::{self, Dh1080Keypair, Dh1080Kind};
use ferric_core::debuglog::DebugLog;
use ferric_core::identd;
use ferric_core::logging::ChatLog;
use ferric_core::mrc::interp::{
    script_events, Effect, EchoTarget, MenuAddress, MenuNode, RunResult, Runtime, ScriptContext, ScriptEvent,
    WINDOW_KINDS,
};
use ferric_core::proxy::{ProxyConfig, ProxyKind};

mod python_runtime;
use python_runtime::{DialogOp, PythonRuntime};
mod script_irc;
use script_irc::{send_effects, SendBudget};
mod perf;
use perf::{spawn_perf_logger, BackendPerfReport, PerfTelemetry};

#[derive(Clone)]
struct AppConfigPaths {
    blow_ini: PathBuf,
    connectd_mrc: PathBuf,
    /// Ferric project folder: logs\ and notes.txt live here.
    project_dir: PathBuf,
    app_config_dir: PathBuf,
}

impl AppConfigPaths {
    /// Folder the script editor works in: the folder of the loaded connectd.mrc (Scripts\).
    /// Falls back to the blow.ini folder if connectd.mrc has no parent.
    fn script_dir(&self) -> PathBuf {
        self.connectd_mrc
            .parent()
            .or_else(|| self.blow_ini.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn push_unique(roots: &mut Vec<PathBuf>, path: Option<PathBuf>) {
    if let Some(path) = path {
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
}

fn find_config_file(roots: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    roots.iter().find_map(|root| {
        names
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
    })
}

fn resolve_config_paths(
    app_handle: &tauri::AppHandle,
) -> Result<AppConfigPaths, Box<dyn std::error::Error>> {
    let config_dir = app_handle.path().app_config_dir()?;
    fs::create_dir_all(&config_dir)?;
    let override_config_dir = std::env::var_os("FERRIC_CONFIG_DIR").map(PathBuf::from);
    let preferred_config_dir = override_config_dir
        .clone()
        .unwrap_or_else(|| config_dir.clone());
    fs::create_dir_all(&preferred_config_dir)?;

    let executable_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));

    // blow.ini: override, then the Ferric project folder, then the app config folder.
    // Never the working directory or parent folders, so a moved Ferric folder keeps
    // working and an old copy elsewhere is not picked up.
    let project_root = executable_dir.as_deref().map(|dir| {
        // Cargo layout target\release\app.exe: the project root is two levels up.
        if dir.ends_with("target/release") || dir.ends_with("target/debug") {
            dir.parent().and_then(Path::parent).unwrap_or(dir).to_path_buf()
        } else {
            dir.to_path_buf()
        }
    });
    let project_dir = project_root
        .clone()
        .unwrap_or_else(|| preferred_config_dir.clone());
    let mut blow_roots = Vec::new();
    push_unique(&mut blow_roots, override_config_dir.clone());
    push_unique(&mut blow_roots, project_root.clone());
    push_unique(&mut blow_roots, Some(config_dir.clone()));
    // With no blow.ini yet, the first save creates it in the project folder.
    let blow_ini = find_config_file(&blow_roots, &["blow.ini"]).unwrap_or_else(|| {
        project_root
            .clone()
            .unwrap_or_else(|| preferred_config_dir.clone())
            .join("blow.ini")
    });

    // connectd.mrc: same roots and order as blow.ini (override, project folder, app config).
    // It lives in the Scripts folder. Never the working directory, parent folders or resource
    // folder. A missing file defaults to Scripts\connectd.mrc in the project folder; the
    // Server manager creates it on the first save.
    let connectd_mrc = find_config_file(&blow_roots, &["Scripts/connectd.mrc", "connectd.mrc"])
        .unwrap_or_else(|| {
            let scripts_dir = project_root
                .unwrap_or_else(|| preferred_config_dir.clone())
                .join("Scripts");
            let _ = fs::create_dir_all(&scripts_dir);
            scripts_dir.join("connectd.mrc")
        });

    Ok(AppConfigPaths {
        blow_ini,
        connectd_mrc,
        project_dir,
        app_config_dir: preferred_config_dir,
    })
}

/// Status line shown when blow.ini exists but cannot be parsed.
const BLOW_UNREADABLE_NOTICE: &str = "blow.ini could not be read; FiSH keys not loaded";

/// Reads the app's own blow.ini. A missing file gives no keys. A read or parse error also
/// gives no keys, with `false` so the caller can show a status line; it never aborts a connect.
fn load_blow_keys(path: &Path) -> (HashMap<String, String>, bool) {
    if !path.is_file() {
        return (HashMap::new(), true);
    }
    match parse_blow_ini(path) {
        Ok(keys) => (keys, true),
        Err(_) => (HashMap::new(), false),
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create configuration directory: {}", error))?;
    }
    Ok(())
}

struct AppSharedState {
    app_state: Mutex<AppState>,
    clients: Mutex<HashMap<String, Arc<IrcClient>>>,
    connecting: Mutex<HashMap<String, u64>>,
    next_conn_id: AtomicU64,
    paths: AppConfigPaths,
    scripts: ScriptStore,
    script_runtime: Mutex<Runtime>,
    python_runtime: Mutex<PythonRuntime>,
    /// Send budget of the script IRC lines, per network. Never held across an await.
    script_budgets: std::sync::Mutex<HashMap<String, SendBudget>>,
    scripts_dirty: AtomicBool,
    exiting: AtomicBool,
    logging: std::sync::Mutex<LogSettings>,
    /// logs\connect-debug.log: connect steps, no secrets. On by default.
    debug_log: Arc<DebugLog>,
    identd: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Pending DH1080 exchanges. Memory only. Never hold this lock across an await.
    dh_pending: std::sync::Mutex<DhPending>,
    /// Rate limit for incoming DH1080_INIT lines. Memory only.
    dh_init_rate: std::sync::Mutex<DhInitRate>,
    /// Cooldown for auto-initiated DH1080 exchanges. Memory only.
    auto_dh_cooldown: std::sync::Mutex<HashMap<(String, String), Instant>>,
    /// The Options setting "Allow self-signed certificates", as the UI last sent it. Every connect
    /// reads it. Starts on, which is the Options default.
    allow_self_signed: AtomicBool,
    /// The Options setting "DH1080 key exchange on open". Starts on, which is the Options default.
    query_auto_keyx: AtomicBool,
    /// The Options setting "Minimize to system tray".
    minimize_to_tray: AtomicBool,
    python_autoloaded: AtomicBool,
    pub perf: Arc<PerfTelemetry>,
}

/// Chat logging switches. Everything is off until `set_logging` turns it on. Lines go to a writer
/// thread (`LogWriter`), so a slow disk never holds an IRC task.
struct LogSettings {
    enabled: bool,
    writer: LogWriter,
    channel: bool,
    query: bool,
    status: bool,
}

/// Lines that may wait for the disk. Past this, a line is dropped and counted: the queue never
/// grows without limit.
const LOG_QUEUE_LINES: usize = 4096;

struct LogJob {
    network: String,
    target: String,
    line: String,
}

/// Handle to the thread that appends log lines. Submitting never blocks.
struct LogWriter {
    tx: std::sync::mpsc::SyncSender<LogJob>,
    dropped: AtomicU64,
}

impl LogWriter {
    /// Starts the writer thread. It ends when the last handle is dropped.
    fn start(mut log: ChatLog) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<LogJob>(LOG_QUEUE_LINES);
        log.set_enabled(true); // the on/off choice is made by `LogSettings`, before a line is queued
        let _ = std::thread::Builder::new()
            .name("ferric-log".into())
            .spawn(move || {
                for job in rx {
                    let _ = log.append(&job.network, &job.target, &job.line);
                }
            });
        Self {
            tx,
            dropped: AtomicU64::new(0),
        }
    }

    /// Queues one line. A full queue, or a writer thread that did not start, drops it.
    fn submit(&self, network: &str, target: &str, line: &str) {
        let job = LogJob {
            network: network.to_string(),
            target: target.to_string(),
            line: line.to_string(),
        };
        if self.tx.try_send(job).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

const MAX_NOTES_BYTES: usize = 64 * 1024;

/// Words that keep an own message out of the log (case-insensitive substring).
const LOG_BLOCKED_WORDS: [&str; 7] = [
    "identify", "password", "pass", "ghost", "recover", "register", "oper",
];

/// True when an own sent line may be logged: not a command and no credential word.
fn own_line_loggable(text: &str) -> bool {
    if text.trim_start().starts_with('/') {
        return false;
    }
    let lower = text.to_lowercase();
    !LOG_BLOCKED_WORDS.iter().any(|word| lower.contains(word))
}

fn format_log_line(sender: &str, text: &str, is_action: bool) -> String {
    if is_action {
        format!("* {} {}", sender, text)
    } else {
        format!("<{}> {}", sender, text)
    }
}

fn is_channel_name(target: &str) -> bool {
    matches!(target.chars().next(), Some('#' | '&' | '+' | '!'))
}

#[derive(Clone, Copy)]
enum LogKind {
    Channel,
    Query,
    Status,
}

/// Append one line if logging is on and the flag for `kind` is set. Errors are
/// dropped on purpose: a full disk or a bad name must never stop the app.
fn log_line(state: &AppSharedState, network: &str, target: &str, line: &str, kind: LogKind) {
    // Key exchange lines carry public values; never write them.
    if line.to_ascii_uppercase().contains("DH1080_") {
        return;
    }
    let Ok(settings) = state.logging.lock() else {
        return;
    };
    let wanted = match kind {
        LogKind::Channel => settings.channel,
        LogKind::Query => settings.query,
        LogKind::Status => settings.status,
    };
    if settings.enabled && wanted {
        settings.writer.submit(network, &target.to_lowercase(), line);
    }
}

/// Log an incoming chat message (own sends are logged in `send_message`).
/// Events waiting between the connection tasks and the event loop. A full queue drops
/// the newest events and so bounds memory when a server or a script floods.
const EVENT_QUEUE: usize = 2048;

fn log_incoming(state: &AppSharedState, own_nick: &str, event: &IrcEvent) {
    let IrcEvent::Message {
        network,
        target,
        sender,
        text,
        is_notice,
        is_action,
        ..
    } = event
    else {
        return;
    };
    // Our own lines come back from a bouncer or with echo-message. The same filter as the send
    // path applies, so a private line with a password in it is not written to the log.
    if sender.eq_ignore_ascii_case(own_nick) && !own_line_loggable(text) {
        return;
    }
    let line = format_log_line(sender, text, *is_action);
    if target == "status" {
        log_line(state, network, "status", &line, LogKind::Status);
    } else if is_channel_name(target) {
        log_line(state, network, target, &line, LogKind::Channel);
    } else if *is_notice {
        // Private notices (services, server) belong to status, not a query window.
        log_line(state, network, "status", &line, LogKind::Status);
    } else {
        // Our own private message, echoed back by a bouncer or echo-message, belongs to the
        // window of the person we sent it to, not to a window named after us.
        let peer = if sender.eq_ignore_ascii_case(own_nick) { target } else { sender };
        log_line(state, network, peer, &line, LogKind::Query);
    }
}

/// Letters, digits, '-', '_', '.', at most 32 bytes. Empty is allowed (identd falls back).
fn valid_identd_username(name: &str) -> bool {
    name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn build_proxy(
    kind: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
) -> Result<Option<ProxyConfig>, String> {
    let host = match host.map(|h| h.trim().to_string()) {
        Some(h) if !h.is_empty() => h,
        _ => return Ok(None),
    };
    // The host goes into feed URLs and onto the connect line: only plain host characters.
    if !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        return Err("The proxy host may contain only letters, digits, dots, hyphens, underscores and colons".to_string());
    }
    let kind = match kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("socks4") => ProxyKind::Socks4,
        Some("socks5") => ProxyKind::Socks5,
        Some("http") | Some("http_connect") => ProxyKind::HttpConnect,
        _ => return Err("Unknown proxy type (use socks4, socks5 or http)".to_string()),
    };
    let port = port
        .filter(|p| *p != 0)
        .ok_or_else(|| "Proxy port is required".to_string())?;
    Ok(Some(ProxyConfig {
        kind,
        host,
        port,
        username: username.filter(|u| !u.is_empty()),
    }))
}

async fn reserve_connection(state: &AppSharedState, network: &str) -> Option<u64> {
    let mut connecting = state.connecting.lock().await;
    let clients = state.clients.lock().await;
    if clients.contains_key(network) || connecting.contains_key(network) {
        return None;
    }
    let token = state.next_conn_id.fetch_add(1, Ordering::Relaxed);
    connecting.insert(network.to_string(), token);
    Some(token)
}

/// Stores a finished connection. A network closed while it was connecting has lost its
/// reservation; its client is shut down instead of being stored, and `false` is returned.
async fn store_connection(state: &AppSharedState, network: String, token: u64, client: Arc<IrcClient>) -> bool {
    let mut connecting = state.connecting.lock().await;
    if connecting.get(&network).copied() != Some(token) {
        client.shutdown();
        return false;
    }
    connecting.remove(&network);
    let mut clients = state.clients.lock().await;
    clients.insert(network, client);
    true
}

async fn release_connection(state: &AppSharedState, network: &str, token: u64) -> bool {
    let mut connecting = state.connecting.lock().await;
    if connecting.get(network).copied() == Some(token) {
        connecting.remove(network);
        true
    } else {
        false
    }
}

async fn remove_connection_if_current(
    state: &AppSharedState,
    network: &str,
    expected: &Arc<IrcClient>,
) {
    let mut clients = state.clients.lock().await;
    let is_current = clients
        .get(network)
        .map(|current| Arc::ptr_eq(current, expected))
        .unwrap_or(false);
    if is_current {
        clients.remove(network);
    }
}

/// False once `close_network` (or a newer connection) replaced this client. Its late events
/// must not re-create the closed network in the shared state.
/// Whether `expected` is still its network's connection. A closed network's client is retired at once,
/// and a shut-down one is never current; both are flags, so no client-map lock is taken per line.
async fn is_current_connection(expected: &Arc<IrcClient>) -> bool {
    !expected.is_retired() && !expected.is_shut_down()
}

/// Drops the network's client and removes it from the shared state. The UI sends QUIT first.
/// The blow.ini keys stay: there is no saved FiSH network name list to edit.
#[tauri::command]
async fn close_network(
    network: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let removed = state.clients.lock().await.remove(&network);
    state.connecting.lock().await.remove(&network);
    {
        let mut app_st = state.app_state.lock().await;
        app_st.remove_network(&network);
        app_st.mark_dirty();
    }
    if let Ok(mut pending) = state.dh_pending.lock() {
        pending.retain(|(net, _), _| net != &network);
    }
    // A closed network takes its online timers and its send budget with it.
    state.script_runtime.lock().await.remove_network_timers(&network);
    if let Ok(mut budgets) = state.script_budgets.lock() {
        budgets.remove(&network);
    }
    if let Some(client) = removed {
        // Not current from now on, so the event loop does not revive the network from its own
        // Disconnected event while the QUIT is still on its way.
        client.retire();
        retire_client(client);
    }
    Ok(())
}

/// How long a closed network's socket may stay open so a queued QUIT can still be sent.
const CLOSE_GRACE: Duration = Duration::from_secs(3);

/// Ends the connection tasks of a client that left the client map. Without this a closed
/// network whose server stays silent keeps its tasks, socket and event loop until exit.
fn retire_client(client: Arc<IrcClient>) {
    tokio::spawn(async move {
        tokio::time::sleep(CLOSE_GRACE).await;
        client.shutdown();
    });
}

#[tauri::command]
async fn connect_network(
    config: ConnectConfig,
    proxy_kind: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let network_id = config.network.clone();
    let mut config = ConnectConfig {
        allow_self_signed: state.allow_self_signed.load(Ordering::Relaxed),
        ..config
    };
    let proxy = build_proxy(proxy_kind, proxy_host, proxy_port, proxy_username)?;
    if let Err(error) = pin_certificate(&state.paths, &mut config, proxy.as_ref(), proxy_password.as_deref().filter(|p| !p.is_empty())).await {
        run_lifecycle_event(state.inner().as_ref(), Some(&app_handle), Some(&network_id), "connectfail", error.clone()).await;
        return Err(error);
    }
    let (blow_keys, blow_ok) = load_blow_keys(&state.paths.blow_ini);
    let Some(conn_token) = reserve_connection(state.inner().as_ref(), &network_id).await else {
        return Err(format!("Network '{}' is already connected or connecting", network_id));
    };
    let (event_tx, mut event_rx) = mpsc::channel::<IrcEvent>(EVENT_QUEUE);

    let mut client = IrcClient::new(config.clone(), event_tx);
    client.set_proxy(proxy, proxy_password.filter(|p| !p.is_empty()));
    client.set_debug_log(Some(Arc::clone(&state.debug_log)));
    for (target, key) in &blow_keys {
        client.set_fish_key(target, key).await;
    }
    if let Err(error) = client.connect().await {
        release_connection(state.inner().as_ref(), &network_id, conn_token).await;
        run_lifecycle_event(state.inner().as_ref(), Some(&app_handle), Some(&network_id), "connectfail", error.clone()).await;
        return Err(error);
    }

    let client_arc = Arc::new(client);
    if !store_connection(state.inner().as_ref(), network_id.clone(), conn_token, Arc::clone(&client_arc)).await {
        return Err("The connection was closed while it was being set up".to_string());
    }
    if !blow_ok {
        dh1080_status(state.inner().as_ref(), &app_handle, &network_id, "", BLOW_UNREADABLE_NOTICE.to_string()).await;
    }

    let state_clone = Arc::clone(&state);
    let app_handle_clone = app_handle.clone();
    let client_for_cleanup = Arc::clone(&client_arc);
    let network_for_cleanup = network_id.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if !is_current_connection(&client_for_cleanup).await {
                break;
            }
            if intercept_dh1080(state_clone.as_ref(), &app_handle_clone, &client_for_cleanup, &event).await {
                continue;
            }
            let disconnected = matches!(&event, IrcEvent::Disconnected { .. });
            process_event(state_clone.as_ref(), Some(&app_handle_clone), &network_for_cleanup, &event).await;
            emit_to_ui(&app_handle_clone, &event);
            if disconnected {
                break;
            }
        }
        remove_connection_if_current(state_clone.as_ref(), &network_for_cleanup, &client_for_cleanup).await;
    });

    Ok(format!("Connected to {}", network_id))
}

#[tauri::command]
async fn load_connectd(
    proxy_kind: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    let proxy = build_proxy(proxy_kind, proxy_host, proxy_port, proxy_username)?;
    let proxy_password = proxy_password.filter(|p| !p.is_empty());

    let mut servers = parse_connectd_mrc(&state.paths.connectd_mrc)?;
    apply_server_meta(&mut servers, &load_server_meta(server_meta_path(&state.paths)));
    let (blow_keys, blow_ok) = load_blow_keys(&state.paths.blow_ini);
    let blow_keys = Arc::new(blow_keys);

    let mut net_ids = Vec::new();

    // Populate new networks while preserving live buffers on repeated loads.
    {
        let mut app_st = state.app_state.lock().await;
        for (place, s) in servers.iter().enumerate() {
            net_ids.push(s.network_id.clone());
            if !app_st.networks.contains_key(&s.network_id) {
                let mut buffers = BTreeMap::new();
                buffers.insert(
                    "status".to_string(),
                    BufferState {
                        name: "status".to_string(),
                        is_channel: false,
                        topic: Some(format!("Connecting to {}:{}...", s.host, s.port)),
                        users: BTreeMap::new(),
                        messages: Vec::new(),
                        unread_count: 0,
                        highlight_count: 0,
                        topic_set_by: None,
                        topic_history: Vec::new(),
                        channel_flags: String::new(),
                        channel_key: None,
                        channel_limit: None,
                        bans: Vec::new(),
                        excepts: Vec::new(),
                        invites: Vec::new(),
                        messages_mode: String::new(),
                        lists_mode: String::new(),
                        ..Default::default()
                    },
                );

                app_st.networks.insert(
                    s.network_id.clone(),
                    NetworkState {
                        id: s.network_id.clone(),
                        server: s.host.clone(),
                        nick: s.nick.clone(),
                        connected: false,
                        active_buffer: "status".to_string(),
                        buffers,
                        label: s.label.clone(),
                        order: Some(place),
                        enabled: s.enabled,
                    },
                );
            } else if let Some(net) = app_st.networks.get_mut(&s.network_id) {
                net.label = s.label.clone();
                net.order = Some(place);
                net.enabled = s.enabled;
            }
        }

        if app_st.active_network.is_none() && !net_ids.is_empty() {
            app_st.active_network = Some(net_ids[0].clone());
        }
        app_st.mark_dirty();
    }

    let mut pending_servers = Vec::new();
    for server in servers {
        // A disabled server keeps its sidebar entry, shown as offline, but is not connected.
        if server.enabled {
            if let Some(conn_token) = reserve_connection(state.inner().as_ref(), &server.network_id).await {
                pending_servers.push((server, conn_token));
            }
        }
    }

    // Launch each new network connection with a small stagger to avoid bouncer flood limits.
    for (idx, (server, conn_token)) in pending_servers.into_iter().enumerate() {
        let net_id = server.network_id.clone();
        let config = ConnectConfig {
            allow_self_signed: state.allow_self_signed.load(Ordering::Relaxed),
            ..ConnectConfig::from(server)
        };
        let blow_keys_clone = Arc::clone(&blow_keys);
        let proxy_clone = proxy.clone();
        let proxy_password_clone = proxy_password.clone();
        let state_clone = Arc::clone(&state);
        let app_handle_clone = app_handle.clone();

        tokio::spawn(async move {
            if idx > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(idx as u64 * 120)).await;
            }

            let mut config = config;
            if let Err(e) = pin_certificate(&state_clone.paths, &mut config, proxy_clone.as_ref(), proxy_password_clone.as_deref()).await {
                if !release_connection(state_clone.as_ref(), &net_id, conn_token).await {
                    return;
                }
                let msg = format!("Connection failed: {}", e);
                let err_ev = IrcEvent::Error {
                    network: net_id.clone(),
                    message: msg.clone(),
                };
                {
                    let mut app_st = state_clone.app_state.lock().await;
                    app_st.handle_event(&err_ev);
                }
                let _ = app_handle_clone.emit("irc-event", &err_ev);
                run_lifecycle_event(state_clone.as_ref(), Some(&app_handle_clone), Some(&net_id), "connectfail", msg).await;
                return;
            }
            let (event_tx, mut event_rx) = mpsc::channel::<IrcEvent>(EVENT_QUEUE);
            let mut client = IrcClient::new(config, event_tx);
            client.set_proxy(proxy_clone, proxy_password_clone);
            client.set_debug_log(Some(Arc::clone(&state_clone.debug_log)));

            for (target, key) in blow_keys_clone.iter() {
                client.set_fish_key(target, key).await;
            }

            match client.connect().await {
                Ok(()) => {
                    let client_arc = Arc::new(client);
                    // false: the network was closed while it connected. store_connection has shut the
                    // client down, and the loop below sees that it is no longer current and ends.
                    let still_current = store_connection(state_clone.as_ref(), net_id.clone(), conn_token, Arc::clone(&client_arc)).await;
                    if still_current && !blow_ok {
                        dh1080_status(state_clone.as_ref(), &app_handle_clone, &net_id, "", BLOW_UNREADABLE_NOTICE.to_string()).await;
                    }
                    let client_for_cleanup = Arc::clone(&client_arc);

                    while let Some(event) = event_rx.recv().await {
                        if !is_current_connection(&client_for_cleanup).await {
                            break;
                        }
                        if intercept_dh1080(state_clone.as_ref(), &app_handle_clone, &client_for_cleanup, &event).await {
                            continue;
                        }
                        let disconnected = matches!(&event, IrcEvent::Disconnected { .. });
                        process_event(state_clone.as_ref(), Some(&app_handle_clone), &net_id, &event).await;
                        emit_to_ui(&app_handle_clone, &event);
                        if disconnected {
                            break;
                        }
                    }
                    remove_connection_if_current(
                        state_clone.as_ref(),
                        &net_id,
                        &client_for_cleanup,
                    ).await;
                }
                Err(e) => {
                    if !release_connection(state_clone.as_ref(), &net_id, conn_token).await {
                        return;
                    }
                    let msg = format!("Connection failed: {}", e);
                    let err_ev = IrcEvent::Error {
                        network: net_id.clone(),
                        message: msg.clone(),
                    };
                    {
                        let mut app_st = state_clone.app_state.lock().await;
                        app_st.handle_event(&err_ev);
                    }
                    let _ = app_handle_clone.emit("irc-event", &err_ev);
                    run_lifecycle_event(state_clone.as_ref(), Some(&app_handle_clone), Some(&net_id), "connectfail", msg).await;
                }
            }
        });
    }

    Ok(net_ids)
}

/// One message we sent, as state, the log and the UI record it.
struct SentLine<'a> {
    network: &'a str,
    target: &'a str,
    nick: &'a str,
    /// The text as it is shown: a CTCP ACTION without its \x01 wrapper.
    text: String,
    is_action: bool,
    is_encrypted: bool,
}

/// Records a message we sent: state first, then the log, then the UI when `app` is given.
async fn record_own_message(state: &AppSharedState, app: Option<&tauri::AppHandle>, sent: SentLine<'_>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let event = IrcEvent::Message {
        sender_address: None,
        network: sent.network.to_string(),
        target: sent.target.to_string(),
        sender: sent.nick.to_string(),
        text: sent.text.clone(),
        spans: Vec::new(),
        is_notice: false,
        is_action: sent.is_action,
        is_encrypted: sent.is_encrypted,
        timestamp: now,
        sender_host: None,
    };
    state.app_state.lock().await.handle_event(&event);
    if own_line_loggable(&sent.text) {
        let kind = if is_channel_name(sent.target) {
            LogKind::Channel
        } else {
            LogKind::Query
        };
        log_line(
            state,
            sent.network,
            sent.target,
            &format_log_line(sent.nick, &sent.text, sent.is_action),
            kind,
        );
    }
    if let Some(app) = app {
        let _ = app.emit("irc-event", &event);
    }
}

#[tauri::command]
async fn send_message(
    network: String,
    target: String,
    text: String,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    if target.trim().eq_ignore_ascii_case("status") {
        return Err("Not on a channel or query. Use /msg <nick> <text>.".to_string());
    }
    // The map lock covers the lookup only; the send and the logging below run without it.
    let client = state.clients.lock().await.get(&network).cloned();
    let Some(client) = client else {
        return Err(format!("Network '{}' not found", network));
    };
    let nick = client.current_nick();
    let (plain_text, is_enc) = client.send_privmsg(&target, &text).await?;
    // A CTCP ACTION (/me, the Slap item) echoes as an action, not as raw \x01 text.
    let action_body: Option<String> = plain_text
        .strip_prefix("\x01ACTION ")
        .and_then(|rest| rest.strip_suffix('\x01'))
        .map(str::to_string);
    let (plain_text, is_action) = match action_body {
        Some(body) => (body, true),
        None => (plain_text, false),
    };
    let sent = SentLine {
        network: &network,
        target: &target,
        nick: &nick,
        text: plain_text,
        is_action,
        is_encrypted: is_enc,
    };
    record_own_message(state.inner().as_ref(), Some(&app_handle), sent).await;
    Ok(())
}

#[tauri::command]
async fn send_raw(
    network: String,
    raw: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let clients = state.clients.lock().await;
    if let Some(client) = clients.get(&network) {
        client.send_raw(&raw)?;
        Ok(())
    } else {
        Err(format!("Network '{}' not found", network))
    }
}

/// The Options setting for self-signed certificates. Applied to every connect from now on.
#[tauri::command]
fn set_allow_self_signed(value: bool, state: State<'_, Arc<AppSharedState>>) {
    state.allow_self_signed.store(value, Ordering::Relaxed);
}

/// The Options setting for auto DH1080 key exchange on opening queries.
#[tauri::command]
fn set_query_auto_keyx(value: bool, state: State<'_, Arc<AppSharedState>>) {
    state.query_auto_keyx.store(value, Ordering::Relaxed);
}

fn restore_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_skip_taskbar(false);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn set_minimize_to_tray(
    app: tauri::AppHandle,
    value: bool,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    state.minimize_to_tray.store(value, Ordering::Relaxed);
    if value {
        if app.tray_by_id("main").is_some() {
            return Ok(());
        }
        let show_item = MenuItemBuilder::with_id("show", "Show Ferric")
            .build(&app)
            .map_err(|e| e.to_string())?;
        let quit_item = MenuItemBuilder::with_id("quit", "Quit")
            .build(&app)
            .map_err(|e| e.to_string())?;
        let menu = MenuBuilder::new(&app)
            .items(&[&show_item, &quit_item])
            .build()
            .map_err(|e| e.to_string())?;

        let mut builder = TrayIconBuilder::with_id("main")
            .tooltip("Ferric")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "show" => restore_main_window(app),
                "quit" => {
                    if let Some(state) = app.try_state::<Arc<AppSharedState>>() {
                        if state.scripts_dirty.load(Ordering::SeqCst) {
                            restore_main_window(app);
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.emit("script-close-requested", ());
                            }
                        } else if !state.exiting.load(Ordering::SeqCst) {
                            let state = state.inner().clone();
                            let handle = app.clone();
                            tauri::async_runtime::spawn(finish_exit(state, handle));
                        }
                    } else {
                        app.exit(0);
                    }
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    restore_main_window(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
        builder.build(&app).map_err(|e| e.to_string())?;
    } else {
        restore_main_window(&app);
        let _ = app.remove_tray_by_id("main");
    }
    Ok(())
}

#[tauri::command]
async fn set_python_settings(
    app: tauri::AppHandle,
    enabled: bool,
    path: Option<String>,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut autoload_results = Vec::new();
    {
        let mut py = state.python_runtime.lock().await;
        let path_changed = py.set_custom_python_path(path);
        let enabled_changed = py.set_enabled(enabled);
        if !enabled {
            state.python_autoloaded.store(false, Ordering::SeqCst);
        } else {
            let already_loaded = state.python_autoloaded.swap(true, Ordering::SeqCst);
            if !already_loaded || path_changed || enabled_changed {
                let script_dir = state.paths.script_dir();
                for name in read_loaded_list(&script_dir) {
                    if is_python_script(&name) {
                        if let Ok(script) = state.scripts.read(&name) {
                            if let Ok((notices, mut run_result)) = py.load(&name, &script.content).await {
                                run_result.notices.extend(notices);
                                autoload_results.push(run_result);
                            }
                        }
                    }
                }
            }
        }
        let ops = py.take_dialog_ops();
        drop(py);
        emit_dialog_ops(Some(&app), ops);
    }

    if !autoload_results.is_empty() {
        let net = state.app_state.lock().await.active_network.clone();
        if let Some(net) = net {
            for run_result in &autoload_results {
                deliver_script_result(state.as_ref(), Some(&app), &net, "status", run_result).await;
            }
        }
    }
    Ok(())
}

fn tls_pins_path(paths: &AppConfigPaths) -> std::path::PathBuf {
    paths.connectd_mrc.with_file_name("ferric-tls-pins.txt")
}

/// Trust on first use. The first connect to a server with self-signed certificates allowed reads
/// the fingerprint of its certificate and saves it in `ferric-tls-pins.txt`; later connects accept
/// only that certificate, so a changed certificate is refused by the handshake.
async fn trust_on_first_use(
    paths: &AppConfigPaths,
    host: &str,
    port: u16,
    proxy: Option<&ProxyConfig>,
    proxy_password: Option<&str>,
) -> Result<String, String> {
    let pins_path = tls_pins_path(paths);
    let key = ferric_core::tls_pin_key(host, port);
    if let Some(pin) = ferric_core::load_tls_pins(&pins_path).get(&key) {
        return Ok(pin.clone());
    }
    let fingerprint = ferric_core::connection::client::probe_tls_fingerprint(
        host,
        port,
        proxy.map(|cfg| (cfg, proxy_password)),
    )
    .await?;
    ferric_core::save_tls_pin(&pins_path, &key, &fingerprint)?;
    Ok(fingerprint)
}

/// Pins the certificate of a TLS connection that has no fingerprint of its own, when self-signed
/// certificates are allowed. Other connections are left as they are.
async fn pin_certificate(
    paths: &AppConfigPaths,
    config: &mut ConnectConfig,
    proxy: Option<&ProxyConfig>,
    proxy_password: Option<&str>,
) -> Result<(), String> {
    if !config.use_tls || !config.allow_self_signed || config.tls_fingerprint.is_some() {
        return Ok(());
    }
    config.tls_fingerprint = Some(trust_on_first_use(paths, &config.host, config.port, proxy, proxy_password).await?);
    Ok(())
}

/// The fingerprint of a server's certificate, for the user to compare with the one the
/// server admin published. The certificate itself is not trusted by this call.
#[tauri::command]
async fn tls_fingerprint(
    host: String,
    port: u16,
    proxy_kind: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
) -> Result<String, String> {
    // Through the same proxy as the connection, so the probe does not reveal the user's address.
    let proxy = build_proxy(proxy_kind, proxy_host, proxy_port, proxy_username)?;
    let password = proxy_password.filter(|p| !p.is_empty());
    let via = proxy.as_ref().map(|cfg| (cfg, password.as_deref()));
    ferric_core::connection::client::probe_tls_fingerprint(&host, port, via).await
}

#[tauri::command]
async fn set_fish_key(
    network: String,
    target: String,
    key: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let path = state.paths.blow_ini.clone();
    let net = if network.is_empty() { None } else { Some(network.as_str()) };
    store_fish_key(state.inner().as_ref(), &path, net, &target, &key).await
}

#[tauri::command]
async fn get_fish_keys(state: State<'_, Arc<AppSharedState>>) -> Result<Vec<BlowEntry>, String> {
    let path = state.paths.blow_ini.clone();
    if path.is_file() {
        list_blow_ini_entries(path)
    } else {
        Ok(Vec::new())
    }
}

#[tauri::command]
async fn get_buffer_fish_key(
    network: String,
    target: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<Option<String>, String> {
    let clients = state.clients.lock().await;
    if let Some(client) = clients.get(&network) {
        Ok(client.get_fish_key(&target).await)
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn save_fish_key_entry(
    network: Option<String>,
    target: String,
    key: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let path = state.paths.blow_ini.clone();
    store_fish_key(state.inner().as_ref(), &path, network.as_deref(), &target, &key).await
}

/// The blow.ini section for a key, and whether it is scoped to one network.
fn fish_section(network: Option<&str>, target: &str) -> (String, bool) {
    match network {
        Some(net) if !net.is_empty() && !target.contains(':') => (format!("{}:{}", net, target), true),
        _ => (target.to_string(), false),
    }
}

/// Gives every connection the keys blow.ini holds now, as a connect loads them. A key that was
/// deleted or changed then stops working in the running session too. An unreadable file changes nothing.
async fn sync_fish_keys_from_file(state: &AppSharedState, path: &Path) {
    static FISH_SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _sync_guard = FISH_SYNC_LOCK.lock().await;
    let path = path.to_path_buf();
    let (keys, readable) = tokio::task::spawn_blocking(move || load_blow_keys(&path))
        .await
        .unwrap_or_else(|_| (HashMap::new(), false));
    if !readable {
        return;
    }
    let clients: Vec<Arc<IrcClient>> = state.clients.lock().await.values().cloned().collect();
    for client in &clients {
        client.replace_fish_keys(keys.clone()).await;
    }
}

/// Saves a key to blow.ini and sets it in memory. Shared by the key manager, the UI and
/// the DH1080 exchange.
async fn store_fish_key(
    state: &AppSharedState,
    path: &Path,
    network: Option<&str>,
    target: &str,
    key: &str,
) -> Result<(), String> {
    let (section, scoped) = fish_section(network, target);
    // Lookups try "network:target" before "target", so the in-memory key must use the
    // same section name that was written to blow.ini. A scoped key goes to its own
    // network's client only; other networks never see it.
    // The clients lock covers only the in-memory update, never the file write below.
    {
        let clients = state.clients.lock().await;
        if scoped {
            if let Some(client) = network.and_then(|net| clients.get(net)) {
                client.set_fish_key(&section, key).await;
            }
        } else {
            for client in clients.values() {
                client.set_fish_key(&section, key).await;
            }
        }
    }
    // The file write runs on a blocking thread, not on the task that holds the IRC state.
    let blow_path = path.to_path_buf();
    let path = path.to_path_buf();
    let key = key.to_string();
    tokio::task::spawn_blocking(move || {
        ensure_parent_dir(&path)?;
        save_blow_ini_key(&path, &section, &key)
    })
    .await
    .map_err(|e| e.to_string())??;
    sync_fish_keys_from_file(state, &blow_path).await;
    Ok(())
}

// ---- DH1080 key exchange (query level, as FiSH 10) ----

const DH_PENDING_MAX: usize = 16;
const DH_PENDING_TTL: Duration = Duration::from_secs(300);

/// Pending exchanges keyed by (network, nick lowercase). Memory only, never written to disk.
type DhPending = HashMap<(String, String), (Dh1080Keypair, Instant)>;

const DH_INIT_PER_NICK: usize = 3;
const DH_INIT_PER_NETWORK: usize = 20;
const DH_INIT_WINDOW: Duration = Duration::from_secs(60);

/// Accepted incoming DH1080_INIT times per (network, nick) and per network.
#[derive(Default)]
struct DhInitRate {
    nicks: HashMap<(String, String), VecDeque<Instant>>,
    networks: HashMap<String, VecDeque<Instant>>,
}

impl DhInitRate {
    /// True when this INIT is within the limits (3 per nick and 20 per network per minute).
    /// Dropped lines are not counted. `now` is a parameter so tests can use fixed times.
    fn allow(&mut self, network: &str, nick: &str, now: Instant) -> bool {
        let fresh = |t: &Instant| now.saturating_duration_since(*t) < DH_INIT_WINDOW;
        self.nicks.retain(|_, times| {
            times.retain(fresh);
            !times.is_empty()
        });
        self.networks.retain(|_, times| {
            times.retain(fresh);
            !times.is_empty()
        });
        let key = dh_key(network, nick);
        let nick_count = self.nicks.get(&key).map_or(0, VecDeque::len);
        let net_count = self.networks.get(network).map_or(0, VecDeque::len);
        if nick_count >= DH_INIT_PER_NICK || net_count >= DH_INIT_PER_NETWORK {
            return false;
        }
        self.nicks.entry(key).or_default().push_back(now);
        self.networks.entry(network.to_string()).or_default().push_back(now);
        true
    }
}

fn dh_key(network: &str, nick: &str) -> (String, String) {
    (network.to_string(), nick.to_lowercase())
}

/// 1 to 32 characters: letters, digits and the IRC nick symbols `[]\`_^{|}-`.
/// No space, colon, comma or control character can pass.
fn valid_dh_nick(nick: &str) -> bool {
    (1..=32).contains(&nick.chars().count())
        && nick
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "[]\\`_^{|}-".contains(c))
}

/// Adds a pending entry. Drops entries older than the TTL first, then the oldest ones
/// until there is room. `now` is a parameter so tests can use fixed times.
fn dh_pending_insert(map: &mut DhPending, key: (String, String), pair: Dh1080Keypair, now: Instant) {
    map.retain(|_, (_, created)| now.saturating_duration_since(*created) < DH_PENDING_TTL);
    map.remove(&key);
    while map.len() >= DH_PENDING_MAX {
        let oldest = map.iter().min_by_key(|(_, (_, created))| *created).map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                map.remove(&k);
            }
            None => break,
        }
    }
    map.insert(key, (pair, now));
}

/// Removes and returns the pending keypair unless it is older than the TTL.
fn dh_pending_take(map: &mut DhPending, key: &(String, String), now: Instant) -> Option<Dh1080Keypair> {
    let (pair, created) = map.remove(key)?;
    (now.saturating_duration_since(created) < DH_PENDING_TTL).then_some(pair)
}

/// Shows one status line in the nick's query buffer when it is open, else in the status buffer (not logged).
async fn dh1080_status(state: &AppSharedState, app: &tauri::AppHandle, network: &str, nick: &str, text: String) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let event = {
        let mut app_st = state.app_state.lock().await;
        // Same key incoming private messages use (lowercased nick). Our own nick would route to key "*".
        let query_open = app_st
            .networks
            .get(network)
            .is_some_and(|n| !nick.eq_ignore_ascii_case(&n.nick) && n.buffers.contains_key(&nick.to_lowercase()));
        let event = IrcEvent::Message { sender_address: None,
            network: network.to_string(),
            target: if query_open { nick.to_string() } else { "status".to_string() },
            sender: "*".to_string(),
            spans: Vec::new(),
            text,
            is_notice: true,
            is_action: false,
            is_encrypted: false,
            timestamp: now,
            sender_host: None,
        };
        app_st.handle_event(&event);
        event
    };
    let _ = app.emit("irc-event", &event);
}

/// Handles a private DH1080 line. Returns true when the event is consumed: it then reaches
/// neither the chat nor the log. Channel lines and everything else return false.
async fn intercept_dh1080(
    state: &AppSharedState,
    app: &tauri::AppHandle,
    client: &IrcClient,
    event: &IrcEvent,
) -> bool {
    let IrcEvent::Message { network, target, sender, text, is_action, is_encrypted, .. } = event else {
        return false;
    };
    if *is_action || *is_encrypted || is_channel_name(target) {
        return false;
    }
    let Some(msg) = dh1080::parse_message(text) else {
        return false;
    };
    let known_nick = state.app_state.lock().await.networks.get(network).map(|n| n.nick.clone());
    let own_now = client.current_nick();
    let to_us = target.eq_ignore_ascii_case(&own_now)
        || known_nick.is_some_and(|n| target.eq_ignore_ascii_case(&n));
    if !to_us {
        return false;
    }
    if !valid_dh_nick(sender) {
        return true;
    }
    // Invalid peers must not consume the INIT quota: validate first, then rate-limit.
    if dh1080::check_peer_public(&msg.public).is_err() {
        return fail_dh1080(state, app, network, sender).await;
    }
    if msg.kind == Dh1080Kind::Init {
        let allowed = match state.dh_init_rate.lock() {
            Ok(mut rate) => rate.allow(network, sender, Instant::now()),
            Err(_) => false,
        };
        if !allowed {
            return true; // over the limit: drop silently
        }
        {
            let mut app_st = state.app_state.lock().await;
            ensure_query_buffer(&mut app_st, network, sender);
        }
    }
    // An ECB exchange must not replace a CBC key the user already has: that would weaken the
    // channel without a word. The exchange is dropped and the user is told.
    if !msg.cbc
        && client
            .get_fish_key(sender)
            .await
            .is_some_and(|key| ferric_core::crypto::fish::FishCipher::new(&key).is_cbc())
    {
        let line = format!("*** FiSH: {sender} offered an ECB key; keeping the CBC key you have");
        dh1080_status(state, app, network, sender, line).await;
        return true;
    }
    let pending = match msg.kind {
        Dh1080Kind::Init => None,
        Dh1080Kind::Finish => {
            let taken = match state.dh_pending.lock() {
                Ok(mut map) => dh_pending_take(&mut map, &dh_key(network, sender), Instant::now()),
                Err(_) => None,
            };
            match taken {
                Some(pair) => Some(pair),
                None => return true, // nothing pending: ignore the line
            }
        }
    };
    let peer = msg.public.clone();
    let computed = tokio::task::spawn_blocking(move || -> Result<(Dh1080Keypair, String), dh1080::Dh1080Error> {
        let pair = match pending {
            Some(pair) => pair,
            None => Dh1080Keypair::generate()?,
        };
        let key = pair.shared_key(&peer)?;
        Ok((pair, key))
    })
    .await;
    let (pair, key) = match computed {
        Ok(Ok(done)) => done,
        _ => return fail_dh1080(state, app, network, sender).await,
    };
    let stored = if msg.cbc { format!("cbc:{key}") } else { key };
    let blow_ini = state.paths.blow_ini.clone();
    if store_fish_key(state, &blow_ini, Some(network), sender, &stored).await.is_err() {
        return fail_dh1080(state, app, network, sender).await;
    }
    if msg.kind == Dh1080Kind::Init
        && client
            .send_raw(&format!("NOTICE {} :{}", sender, pair.finish_message(msg.cbc)))
            .is_err()
    {
        return fail_dh1080(state, app, network, sender).await;
    }
    let mode = if msg.cbc { "CBC" } else { "ECB" };
    if msg.kind == Dh1080Kind::Init {
        let line = format!("*** FiSH: Received DH1080 public key from {sender}, sending mine ...");
        dh1080_status(state, app, network, sender, line).await;
    }
    let line = format!("*** FiSH: Key for {sender} set to *censored* ({mode} Mode)");
    dh1080_status(state, app, network, sender, line).await;
    true
}

async fn fail_dh1080(state: &AppSharedState, app: &tauri::AppHandle, network: &str, nick: &str) -> bool {
    dh1080_status(state, app, network, nick, format!("*** FiSH: DH1080 key exchange with {nick} failed")).await;
    true
}

fn is_irc_service(nick: &str) -> bool {
    matches!(
        nick.to_ascii_lowercase().as_str(),
        "nickserv" | "chanserv" | "memoserv" | "operserv" | "botserv" | "hostserv" | "helpserv" | "saslserv" | "global" | "auth"
    )
}

fn is_valid_auto_keyx_target(target: &str, own_nick: &str) -> bool {
    valid_dh_nick(target)
        && !is_channel_name(target)
        && !target.eq_ignore_ascii_case("status")
        && !target.eq_ignore_ascii_case(own_nick)
        && !is_irc_service(target)
}

async fn initiate_dh1080(
    state: &AppSharedState,
    app: &tauri::AppHandle,
    network: &str,
    nick: &str,
    cbc: bool,
) -> Result<(), String> {
    if !valid_dh_nick(nick) {
        return Err("Invalid nick: 1-32 letters, digits or []\\`_^{|}-".to_string());
    }
    let client = state
        .clients
        .lock()
        .await
        .get(network)
        .cloned()
        .ok_or_else(|| format!("Network '{}' not found", network))?;
    let pair = tokio::task::spawn_blocking(Dh1080Keypair::generate)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let line = format!("NOTICE {} :{}", nick, pair.init_message(cbc));
    let key = dh_key(network, nick);
    {
        let mut map = state.dh_pending.lock().map_err(|_| "DH1080 state unavailable".to_string())?;
        dh_pending_insert(&mut map, key.clone(), pair, Instant::now());
    }
    if let Err(error) = client.send_raw(&line) {
        if let Ok(mut map) = state.dh_pending.lock() {
            map.remove(&key);
        }
        return Err(error);
    }
    dh1080_status(
        state,
        app,
        network,
        nick,
        format!("*** FiSH: Sent my DH1080 public key to {nick}, waiting for reply ..."),
    )
    .await;
    Ok(())
}

pub(crate) async fn trigger_auto_dh1080(
    state: &AppSharedState,
    app_handle: &tauri::AppHandle,
    network: &str,
    target: &str,
) {
    if !state.query_auto_keyx.load(Ordering::Relaxed) {
        return;
    }
    let own_nick = {
        let app_st = state.app_state.lock().await;
        let Some(net) = app_st.networks.get(network) else {
            return;
        };
        net.nick.clone()
    };
    if !is_valid_auto_keyx_target(target, &own_nick) {
        return;
    }
    let client = {
        let clients = state.clients.lock().await;
        clients.get(network).cloned()
    };
    let Some(client) = client else {
        return;
    };
    // Keep existing keys: do not auto-overwrite a manually set or previously stored key
    if client.get_fish_key(target).await.is_some() {
        return;
    }
    let key = dh_key(network, target);
    {
        let mut map = match state.dh_pending.lock() {
            Ok(m) => m,
            Err(_) => return,
        };
        let now = Instant::now();
        map.retain(|_, (_, created)| now.saturating_duration_since(*created) < DH_PENDING_TTL);
        if map.contains_key(&key) {
            return;
        }
    }
    {
        let mut cooldowns = match state.auto_dh_cooldown.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let now = Instant::now();
        cooldowns.retain(|_, last_time| now.saturating_duration_since(*last_time) < Duration::from_secs(5));
        if cooldowns.contains_key(&key) {
            return;
        }
        cooldowns.insert(key, now);
    }
    let _ = initiate_dh1080(state, app_handle, network, target, true).await;
}

/// Starts a DH1080 exchange: remembers a keypair for (network, nick) and sends the INIT notice.
#[tauri::command]
async fn start_dh1080(
    network: String,
    nick: String,
    cbc: bool,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    initiate_dh1080(state.inner().as_ref(), &app_handle, &network, &nick, cbc).await
}

#[tauri::command]
async fn delete_fish_key_entry(
    section: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<bool, String> {
    // `section` is one exact blow.ini section ("network:target" or a legacy bare target).
    // The client lowercases it, so the same lowercase map entry goes from every client.
    let section = section.trim().to_string();
    if section.is_empty() {
        return Err("No key section given".to_string());
    }
    let path = state.paths.blow_ini.clone();
    // The map lock covers the copy of the client handles only, never the file write.
    let clients: Vec<Arc<IrcClient>> = state.clients.lock().await.values().cloned().collect();
    for client in &clients {
        client.remove_fish_key(&section).await;
    }
    let removed = tokio::task::spawn_blocking(move || delete_blow_ini_key(&path, &section))
        .await
        .map_err(|e| e.to_string())??;
    sync_fish_keys_from_file(state.inner().as_ref(), &state.paths.blow_ini).await;
    Ok(removed)
}

#[tauri::command]
async fn open_query(
    network: String,
    target: String,
    auto_keyx: Option<bool>,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let was_new = {
        let mut app_st = state.app_state.lock().await;
        open_query_buffer(&mut app_st, &network, &target)
            .ok_or_else(|| format!("Network '{}' not found", network))?
    };
    if was_new && auto_keyx.unwrap_or(true) {
        trigger_auto_dh1080(state.inner().as_ref(), &app_handle, &network, &target).await;
    }
    Ok(())
}

/// Ensures the query window of `target` on `network` exists without changing `active_buffer`.
/// Drops the oldest inactive query if the buffer cap is reached.
/// Returns Some(was_new) if network exists, None if network is unknown.
fn ensure_query_buffer(app_st: &mut AppState, network: &str, target: &str) -> Option<bool> {
    let net = app_st.networks.get_mut(network)?;
    let key = target.to_lowercase();
    let is_channel = is_channel_name(target);
    if !is_channel && key != "status" {
        net.make_room_for_query(&key);
    }
    let was_new = !net.buffers.contains_key(&key);
    net.buffers.entry(key.clone()).or_insert_with(|| BufferState {
        name: target.to_string(),
        is_channel,
        topic: if is_channel {
            None
        } else {
            Some(format!("Private conversation with {}", target))
        },
        ..Default::default()
    });
    if was_new {
        app_st.mark_dirty();
    }
    Some(was_new)
}

/// Opens the query window of `target` on `network` if it is not open yet, and makes it the
/// active buffer. Returns Some(was_new) when network exists, None when network is unknown.
fn open_query_buffer(app_st: &mut AppState, network: &str, target: &str) -> Option<bool> {
    let was_new = ensure_query_buffer(app_st, network, target)?;
    if let Some(net) = app_st.networks.get_mut(network) {
        let key = target.to_lowercase();
        if net.active_buffer != key {
            net.active_buffer = key;
            app_st.mark_dirty();
        }
    }
    Some(was_new)
}

/// Makes `buffer` the window the UI shows on `network`, and `network` the shown one. False when the
/// network is unknown; nothing changes then.
fn set_active_buffer_state(app_st: &mut AppState, network: &str, buffer: &str) -> bool {
    let key = buffer.to_lowercase();
    let Some(net) = app_st.networks.get_mut(network) else {
        return false;
    };
    net.active_buffer = key.clone();
    if let Some(buf) = net.buffers.get_mut(&key) {
        buf.unread_count = 0;
        buf.highlight_count = 0;
    }
    app_st.active_network = Some(network.to_string());
    app_st.mark_dirty();
    true
}

#[tauri::command]
async fn set_active_buffer(
    network: String,
    buffer: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut app_st = state.app_state.lock().await;
    if set_active_buffer_state(&mut app_st, &network, &buffer) {
        Ok(())
    } else {
        Err(format!("Network '{}' not found", network))
    }
}

#[tauri::command]
async fn close_buffer(
    network: String,
    buffer: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut app_st = state.app_state.lock().await;
    app_st.mark_dirty();
    if let Some(net) = app_st.networks.get_mut(&network) {
        let key = buffer.to_lowercase();
        if key != "status" {
            net.buffers.remove(&key);
            if net.active_buffer == key {
                net.active_buffer = "status".to_string();
            }
        }
        Ok(())
    } else {
        Err(format!("Network '{}' not found", network))
    }
}

/// `full` asks for every message again (the UI's first sync, or after it lost its copy); otherwise
/// the snapshot carries only the messages the UI has not been sent yet.
#[tauri::command]
async fn get_state(
    full: Option<bool>,
    seen_revision: Option<u64>,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<tauri::ipc::Response, String> {
    let t0 = Instant::now();
    let mut st = state.app_state.lock().await;
    let lock_wait_us = t0.elapsed().as_micros() as u64;

    let t1 = Instant::now();
    let res = st.ui_snapshot_if_changed(full.unwrap_or(false), seen_revision);
    drop(st);

    match res {
        Some(snapshot) => {
            let json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
            let duration_us = t1.elapsed().as_micros() as u64;
            state.perf.record_get_state_snapshot(lock_wait_us, duration_us, json.len());
            Ok(tauri::ipc::Response::new(tauri::ipc::InvokeResponseBody::Json(json)))
        }
        None => {
            let duration_us = t1.elapsed().as_micros() as u64;
            state.perf.record_get_state_noop(lock_wait_us, duration_us);
            Ok(tauri::ipc::Response::new(tauri::ipc::InvokeResponseBody::Json("null".to_string())))
        }
    }
}

#[tauri::command]
async fn get_backend_perf(
    state: State<'_, Arc<AppSharedState>>,
) -> Result<BackendPerfReport, String> {
    Ok(state.perf.report())
}

/// The newest messages of one buffer, for opening it. `limit` is the UI's scrollback setting.
#[tauri::command]
async fn get_buffer_history(
    network: String,
    buffer: String,
    limit: Option<usize>,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<Vec<ferric_core::state::manager::ChatMessage>, String> {
    let st = state.app_state.lock().await;
    Ok(st.buffer_history(&network, &buffer, limit.unwrap_or(2000)))
}

/// `/clear`: empties one buffer's scrollback in the backend too, so opening it again does not
/// bring the lines back.
#[tauri::command]
async fn clear_buffer_history(
    network: String,
    buffer: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut st = state.app_state.lock().await;
    st.clear_buffer_messages(&network, &buffer);
    Ok(())
}

/// Sends an event to the UI. Raw lines stay in the backend: the UI ignores them, and each
/// emitted event costs the UI a full state sync.
fn emit_to_ui(app: &tauri::AppHandle, event: &IrcEvent) {
    if !matches!(event, IrcEvent::Raw { .. }) {
        let _ = app.emit("irc-event", event);
    }
}

/// One incoming event, in order: state update, scripts, then the log. The caller emits
/// the event to the UI afterwards, so script output is already in state when the UI sees it.
/// `app` is None in tests; then the script's own messages are recorded without an emit.
async fn process_event(state: &AppSharedState, app: Option<&tauri::AppHandle>, network: &str, event: &IrcEvent) {
    if let IrcEvent::Disconnected { network: gone, .. } = event {
        state.script_runtime.lock().await.remove_network_timers(gone);
    }
    let own_nick = {
        let mut app_st = state.app_state.lock().await;
        app_st.handle_event(event);
        app_st.networks.get(network).map(|net| net.nick.clone()).unwrap_or_default()
    };
    // logon: the connection is registered. It runs just before connect, which is the closest
    // point the event loop sees to the end of the login.
    if let IrcEvent::Connected { .. } = event {
        run_lifecycle_event(state, app, Some(network), "logon", String::new()).await;
    }
    if let IrcEvent::Error { message, .. } = event {
        if message.starts_with("Connection failed") {
            run_lifecycle_event(state, app, Some(network), "connectfail", message.clone()).await;
        }
    }
    run_script_event(state, app, network, &own_nick, event).await;
    log_incoming(state, &own_nick, event);
}

/// Runs the loaded scripts for one event: sends their IRC lines and writes their echo into chat.
/// The runtime and app state locks are never held at the same time.
async fn run_script_event(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    own_nick: &str,
    event: &IrcEvent,
) {
    let mapped = script_events(event, own_nick);
    if mapped.is_empty() {
        return;
    }
    let default_buf = event_buffer(event);
    dispatch_script_events(state, app, network, &default_buf, &mapped).await;
}

/// Runs the script events of one or more mapped events for a network, in both runtimes.
async fn dispatch_script_events(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    default_buf: &str,
    mapped: &[ScriptEvent],
) {
    // One IRC event can fire several script events (a MODE line with OP and BAN changes). They
    // run in order under one lock, and their output is handled as one result.
    let result = {
        let mut runtime = state.script_runtime.lock().await;
        let mut all = RunResult::default();
        for ev in mapped {
            let one = runtime.dispatch_event(ev, None);
            all.effects.extend(one.effects);
            all.notices.extend(one.notices);
        }
        all
    };
    deliver_script_result(state, app, network, default_buf, &result).await;

    // Python event dispatch
    let (py_result, py_ops) = {
        let mut py = state.python_runtime.lock().await;
        if py.is_enabled() {
            let mut all = RunResult::default();
            for ev in mapped {
                let ctx = ev.context();
                let py_res = py.dispatch_event(ev, &ctx).await;
                all.effects.extend(py_res.effects);
                all.notices.extend(py_res.notices);
            }
            let ops = py.take_dialog_ops();
            (all, ops)
        } else {
            (RunResult::default(), Vec::new())
        }
    };
    emit_dialog_ops(app, py_ops);
    if !py_result.effects.is_empty() || !py_result.notices.is_empty() {
        deliver_script_result(state, app, network, default_buf, &py_result).await;
    }
}

/// Runs the scripts for a lifecycle event that is not an IRC line: start, exit, logon,
/// connectfail, appactive. Output goes to the network's status window; without a network
/// there is no window, so only the scripts run.
async fn run_lifecycle_event(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: Option<&str>,
    event: &'static str,
    text: String,
) {
    let me = match network {
        Some(net) => state.app_state.lock().await.networks.get(net).map(|n| n.nick.clone()),
        None => None,
    };
    let mapped = [ScriptEvent::Lifecycle {
        event,
        network: network.map(String::from),
        me,
        text,
    }];
    dispatch_script_events(state, app, network.unwrap_or(""), "status", &mapped).await;
}

/// Sends the IRC lines of a result and writes its echo and notices into chat. Notices go to
/// `default`; echo lines go to their own window (`-a`, `-s`) or to `default`.
async fn deliver_script_result(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    default: &str,
    result: &RunResult,
) {
    send_effects(state, app, network, &result.effects).await;
    if !result.effects.iter().any(Effect::is_echo) && result.notices.is_empty() {
        return;
    }
    let mut app_st = state.app_state.lock().await;
    let active = app_st.active_network.as_deref().and_then(|net| {
        app_st.networks.get(net).map(|n| (net, n.active_buffer.as_str()))
    });
    let lines = place_script_output(result, network, default, active);
    for line in lines {
        if !app_st.push_local_message(&line.network, &line.buffer, line.text.clone()) {
            app_st.push_local_message(&line.network, "status", line.text);
        }
    }
}

/// Runs the `load` or `unload` handlers of one script on the active network. Without an
/// active network there is no window to write to, so nothing runs.
async fn run_lifecycle(state: &AppSharedState, app: Option<&tauri::AppHandle>, script: &str, event: &str) {
    let (network, me) = {
        let app_st = state.app_state.lock().await;
        let Some(network) = app_st.active_network.clone() else {
            return;
        };
        let me = app_st.networks.get(&network).map(|n| n.nick.clone()).unwrap_or_default();
        (network, me)
    };
    let context = ScriptContext::new().with_network(network.clone()).with_me(me);
    let result = state.script_runtime.lock().await.dispatch_lifecycle(script, event, &context);
    deliver_script_result(state, app, &network, "status", &result).await;
}

/// Starts the timer driver: every 100 ms, which is the timer floor, it runs the due timers.
fn spawn_timer_driver(state: Arc<AppSharedState>, app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            run_due_timers(&state, Some(&app)).await;
        }
    });
}

/// Runs the timers that are due and writes what they produced to their network's status window.
/// The app state lock is released before the runtime lock is taken.
async fn run_due_timers(state: &AppSharedState, app: Option<&tauri::AppHandle>) {
    // This runs every 100 ms: without a timer there is nothing to build.
    if !state.script_runtime.lock().await.has_timers() {
        return;
    }
    let nicks: HashMap<String, String> = {
        let app_st = state.app_state.lock().await;
        app_st.networks.iter().map(|(id, net)| (id.clone(), net.nick.clone())).collect()
    };
    let runs = state
        .script_runtime
        .lock()
        .await
        .fire_timers(&|network: &str| nicks.get(network).cloned());
    for run in runs {
        if let Some(network) = run.network.clone() {
            deliver_script_result(state, app, &network, "status", &run.result).await;
        }
    }
}

/// The chat buffer an event's script output belongs to: the channel, or the sender's
/// query window. Quit has no channel, so its output goes to status.
fn event_buffer(event: &IrcEvent) -> String {
    match event {
        IrcEvent::Message { target, .. } if is_channel_name(target) => target.to_lowercase(),
        IrcEvent::Message { sender, .. } => sender.to_lowercase(),
        IrcEvent::Join { channel, .. } | IrcEvent::Part { channel, .. } => channel.to_lowercase(),
        _ => "status".to_string(),
    }
}

/// One line of script output with the buffer it goes to.
#[derive(Debug, PartialEq, Eq)]
struct ScriptLine {
    network: String,
    buffer: String,
    text: String,
}

/// Places one trigger's output. `-s` lines go to status and `-a` lines to `active`
/// (network, buffer), or to status of `network` when there is no active buffer.
/// Other echo lines and notices go to `default`.
fn place_script_output(
    result: &RunResult,
    network: &str,
    default: &str,
    active: Option<(&str, &str)>,
) -> Vec<ScriptLine> {
    let line = |net: &str, buffer: &str, text: &str| ScriptLine {
        network: net.to_string(),
        buffer: buffer.to_string(),
        text: text.to_string(),
    };
    let mut lines: Vec<ScriptLine> = result
        .effects
        .iter()
        .filter(|effect| effect.is_echo())
        .map(|effect| match effect.target() {
            EchoTarget::Default => line(network, default, effect.text()),
            EchoTarget::Status => line(network, "status", effect.text()),
            EchoTarget::Active => match active {
                Some((net, buffer)) => line(net, buffer, effect.text()),
                // No active window: the caller's window, (not the status window).
                None => line(network, default, effect.text()),
            },
        })
        .collect();
    lines.extend(result.notices.iter().map(|notice| line(network, default, notice)));
    lines
}

// Script editor commands. All rules live in ferric_core::script_files; these
// wrappers only hand the shared store to it.

#[tauri::command]
async fn list_script_files(
    state: State<'_, Arc<AppSharedState>>,
) -> Result<Vec<ScriptFileInfo>, ScriptError> {
    state.scripts.list()
}

#[tauri::command]
async fn read_script_file(
    name: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<ScriptContent, ScriptError> {
    state.scripts.read(&name)
}

// Scripts are loaded only by explicit user action; the editor folder is not an
// inventory of enabled scripts.
#[tauri::command]
async fn load_script_aliases(
    name: String,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    if is_python_script(&name) {
        let script = state.scripts.read(&name).map_err(|e| e.message)?;
        let mut py = state.python_runtime.lock().await;
        let (mut notices, run_result) = py.load(&name, &script.content).await?;
        let ops = py.take_dialog_ops();
        drop(py);
        emit_dialog_ops(Some(&app_handle), ops);
        if let Err(e) = remember_loaded(&state.paths.script_dir(), &name) {
            notices.push(format!("{name} is loaded, but it will not load at the next start: {e}"));
        }
        let net = state.app_state.lock().await.active_network.clone();
        if let Some(net) = net {
            deliver_script_result(state.inner().as_ref(), Some(&app_handle), &net, "status", &run_result).await;
        }
        return Ok(notices);
    }
    let (loaded, mut notices) = {
        let mut runtime = state.script_runtime.lock().await;
        load_aliases(&state.scripts, &mut runtime, &name)?
    };
    if let Err(e) = remember_loaded(&state.paths.script_dir(), &loaded) {
        notices.push(format!("{loaded} is loaded, but it will not load at the next start: {e}"));
    }
    // Only LOAD runs here. The rules for /load and /unload say START
    // may follow under gates that are not established, so START is not sent unconditionally.
    run_lifecycle(state.inner().as_ref(), Some(&app_handle), &loaded, "load").await;
    Ok(notices)
}

/// The file, next to connectd.mrc, that lists the scripts to load at start: one name per line.
/// It is `.txt`, so the Scripts Editor does not list it as a script.
const LOADED_SCRIPTS_FILE: &str = "loaded-scripts.txt";

fn read_loaded_list(dir: &Path) -> Vec<String> {
    let file = dir.join(LOADED_SCRIPTS_FILE);
    if !file.is_file() {
        let defaults = ["services.mrc", "channelcentral.py"];
        return defaults
            .iter()
            .filter(|name| dir.join(name).is_file())
            .map(|s| s.to_string())
            .collect();
    }
    fs::read_to_string(file)
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn write_loaded_list(dir: &Path, names: &[String]) -> Result<(), String> {
    let mut text = names.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    fs::write(dir.join(LOADED_SCRIPTS_FILE), text).map_err(|e| format!("{LOADED_SCRIPTS_FILE}: {e}"))
}

/// Adds `name` to the scripts loaded at start, unless it is listed already (names are not case-sensitive).
fn remember_loaded(dir: &Path, name: &str) -> Result<(), String> {
    let mut names = read_loaded_list(dir);
    if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        names.push(name.to_string());
        write_loaded_list(dir, &names)?;
    }
    Ok(())
}

fn forget_loaded(dir: &Path, name: &str) -> Result<(), String> {
    let mut names = read_loaded_list(dir);
    let before = names.len();
    names.retain(|n| !n.eq_ignore_ascii_case(name));
    if names.len() != before {
        write_loaded_list(dir, &names)?;
    }
    Ok(())
}

/// The name the runtime keeps the script under, and the notices of loading it.
fn load_aliases(store: &ScriptStore, runtime: &mut Runtime, name: &str) -> Result<(String, Vec<String>), String> {
    let script = store.read(name).map_err(|e| e.message)?;
    let (accepted, notices) = runtime.load_accepted(&script.name, &script.content);
    // A script that was not taken is an error: it is not remembered and runs no load handler.
    if !accepted {
        return Err(notices.join("; "));
    }
    Ok((script.name, notices))
}

#[tauri::command]
async fn unload_script_aliases(
    name: String,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<bool, String> {
    if is_python_script(&name) {
        let mut py = state.python_runtime.lock().await;
        let (unloaded, run_result) = py.unload(&name).await?;
        let ops = py.take_dialog_ops();
        drop(py);
        emit_dialog_ops(Some(&app_handle), ops);
        if unloaded {
            forget_loaded(&state.paths.script_dir(), &name)
                .map_err(|e| format!("unloaded, but it may load at the next start: {e}"))?;
            let net = state.app_state.lock().await.active_network.clone();
            if let Some(net) = net {
                deliver_script_result(state.inner().as_ref(), Some(&app_handle), &net, "status", &run_result).await;
            }
        }
        return Ok(unloaded);
    }
    // The unload handlers run while the script is still loaded.
    run_lifecycle(state.inner().as_ref(), Some(&app_handle), &name, "unload").await;
    let unloaded = state.script_runtime.lock().await.unload(&name);
    if unloaded {
        forget_loaded(&state.paths.script_dir(), &name)
            .map_err(|e| format!("unloaded, but it may load at the next start: {e}"))?;
    }
    Ok(unloaded)
}

fn snapshot_channel_state(app_state: &AppState, network: &str, channel: &str) -> Option<serde_json::Value> {
    let net = app_state.networks.get(network)?;
    let chan = net.buffers.get(&channel.to_lowercase())?;
    if !chan.is_channel {
        return None;
    }
    let my_user = chan.users.get(&net.nick.to_lowercase());
    serde_json::to_value(serde_json::json!({
        "network": network,
        "server": &net.server,
        "connected": net.connected,
        "channel": chan.name,
        "my_nick": net.nick,
        "my_prefix": my_user.and_then(|u| u.prefix).map(|c| c.to_string()),
        "my_prefix_modes": my_user.map(|u| &u.prefix_modes),
        "topic": chan.topic,
        "topic_set_by": chan.topic_set_by,
        "topic_history": chan.topic_history,
        "channel_flags": chan.channel_flags,
        "channel_key": chan.channel_key,
        "channel_limit": chan.channel_limit,
        "users": chan.users,
        "bans": chan.bans,
        "excepts": chan.excepts,
        "invites": chan.invites,
    })).ok()
}

fn emit_dialog_ops(app: Option<&tauri::AppHandle>, ops: Vec<DialogOp>) {
    if ops.is_empty() {
        return;
    }
    if let Some(app) = app {
        for op in ops {
            let _ = app.emit("script-dialog", op);
        }
    }
}

#[derive(serde::Serialize)]
struct ScriptAliasResult {
    effects: Vec<String>,
    notices: Vec<String>,
}

#[allow(dead_code)]
fn run_loaded_alias(
    runtime: &mut Runtime,
    name: &str,
    args: &[String],
    ctx: &ScriptContext,
) -> Option<ScriptAliasResult> {
    if !runtime.has_alias(name) {
        return None;
    }
    let result = runtime.run_alias_with_context(name, args, ctx);
    Some(ScriptAliasResult {
        effects: result.effects.iter().filter(|e| e.is_echo()).map(|e| e.text().to_string()).collect(),
        notices: result.notices,
    })
}

#[tauri::command]
async fn run_script_alias(
    name: String,
    args: Vec<String>,
    network: String,
    buffer: String,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<Option<ScriptAliasResult>, String> {
    let me = state.app_state.lock().await.networks.get(&network).map(|n| n.nick.clone()).unwrap_or_default();
    let mut ctx = ScriptContext::new().with_network(network.clone()).with_me(me);
    // The window the alias was run from is its target, as the active window is in the client.
    if buffer != "status" {
        ctx.target = Some(buffer.clone());
    }
    // In a channel window the channel is $chan.
    if is_channel_name(&buffer) {
        ctx = ctx.with_chan(buffer.clone());
    }
    let mut runtime = state.script_runtime.lock().await;
    if runtime.has_alias(&name) {
        let result = runtime.run_alias_with_context(&name, &args, &ctx);
        drop(runtime);
        // The output goes where its echo targets say (-a, -s, the window), as for the other script runs.
        deliver_script_result(state.inner().as_ref(), Some(&app_handle), &network, &buffer, &result).await;
        return Ok(Some(ScriptAliasResult { effects: Vec::new(), notices: Vec::new() }));
    }
    drop(runtime);

    let target_chan = if is_channel_name(&buffer) {
        Some(buffer.clone())
    } else if let Some(first) = args.first() {
        if is_channel_name(first) {
            Some(first.clone())
        } else {
            None
        }
    } else {
        None
    };

    let channel_state = if let Some(ref chan) = target_chan {
        let app_state = state.app_state.lock().await;
        snapshot_channel_state(&app_state, &network, chan)
    } else {
        None
    };

    let mut py = state.python_runtime.lock().await;
    if py.has_alias(&name) {
        let result = py.run_alias_with_state(&name, &args, &ctx, channel_state.as_ref()).await;
        let ops = py.take_dialog_ops();
        drop(py);
        emit_dialog_ops(Some(&app_handle), ops);
        deliver_script_result(state.inner().as_ref(), Some(&app_handle), &network, &buffer, &result).await;
        return Ok(Some(ScriptAliasResult { effects: Vec::new(), notices: Vec::new() }));
    }
    drop(py);

    Ok(None)
}

#[tauri::command]
async fn script_dialog_event(
    dialog_id: String,
    event: serde_json::Value,
    network: Option<String>,
    channel: Option<String>,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let channel_state = if let (Some(ref net), Some(ref chan)) = (&network, &channel) {
        let app_state = state.app_state.lock().await;
        snapshot_channel_state(&app_state, net, chan)
    } else {
        None
    };

    let me = if let Some(ref net) = network {
        state.app_state.lock().await.networks.get(net).map(|n| n.nick.clone()).unwrap_or_default()
    } else {
        String::new()
    };

    let mut ctx = ScriptContext::new();
    if let Some(ref net) = network {
        ctx = ctx.with_network(net.clone());
    }
    if !me.is_empty() {
        ctx = ctx.with_me(me);
    }
    if let Some(ref chan) = channel {
        ctx = ctx.with_chan(chan.clone());
        ctx.target = Some(chan.clone());
    }

    let mut py = state.python_runtime.lock().await;
    let result = py.dispatch_dialog_event(&dialog_id, &event, &ctx, channel_state.as_ref()).await;
    let ops = py.take_dialog_ops();
    drop(py);

    emit_dialog_ops(Some(&app_handle), ops);

    let net_str = network.as_deref().unwrap_or("");
    let chan_str = channel.as_deref().unwrap_or("");
    deliver_script_result(state.inner().as_ref(), Some(&app_handle), net_str, chan_str, &result).await;

    Ok(())
}

/// The context and the `$1`, `$2`, ... of a popup. The values come from the popup and stay data:
/// they are never parsed as commands.
async fn popup_context(
    state: &AppSharedState,
    network: &str,
    nick: &str,
    nicks: &[String],
    channel: &str,
) -> Result<(ScriptContext, Vec<String>), String> {
    for value in std::iter::once(nick).chain(nicks.iter().map(String::as_str)).chain(std::iter::once(channel)) {
        if value.len() > 512 || value.contains(['\r', '\n', '\0']) {
            return Err("a popup value is not a name".to_string());
        }
    }
    let me = state
        .app_state
        .lock()
        .await
        .networks
        .get(network)
        .map(|n| n.nick.clone())
        .ok_or_else(|| format!("Network '{}' not found", network))?;
    let mut ctx = ScriptContext::new().with_network(network).with_me(me);
    if !channel.is_empty() {
        ctx = ctx.with_chan(channel);
    }
    if !nicks.is_empty() {
        // $snicks: the selected nicks, joined with commas.
        ctx.snicks = Some(nicks.join(","));
    }
    if !nick.is_empty() {
        ctx.nick = Some(nick.to_string());
    }
    let args = if nicks.is_empty() {
        if nick.is_empty() {
            Vec::new()
        } else {
            vec![nick.to_string()]
        }
    } else {
        nicks.to_vec()
    };
    Ok((ctx, args))
}

/// The entries of a popup window kind (`status`, `channel`, `query`, `nicklist`, `menubar`)
/// from the loaded scripts, with their labels evaluated now.
#[tauri::command]
async fn script_menu_nodes(
    network: String,
    window: String,
    nick: String,
    nicks: Vec<String>,
    channel: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<Vec<MenuNode>, String> {
    if !WINDOW_KINDS.contains(&window.as_str()) {
        return Err("no popup menu for this window".to_string());
    }
    let (ctx, args) = popup_context(state.inner().as_ref(), &network, &nick, &nicks, &channel).await?;
    let mut nodes = state.script_runtime.lock().await.menu_nodes(&window, &ctx, &args);
    let mut py = state.python_runtime.lock().await;
    let py_nodes = py.menu_nodes(&window, &ctx, &args).await;
    nodes.extend(py_nodes);
    Ok(nodes)
}

/// Runs a command entry of a popup. Its output goes to the window the popup was opened in.
#[tauri::command]
async fn run_script_menu_item(
    network: String,
    buffer: String,
    address: MenuAddress,
    nick: String,
    nicks: Vec<String>,
    channel: String,
    answers: Vec<String>,
    state: State<'_, Arc<AppSharedState>>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    if answers.iter().any(|a| a.len() > 512 || a.contains(['\r', '\n', '\0'])) {
        return Err("a prompt answer is not a single line of at most 512 bytes".to_string());
    }
    let (mut ctx, args) = popup_context(state.inner().as_ref(), &network, &nick, &nicks, &channel).await?;
    // The window the entry was opened in is the target, so /say and /me work in a query window.
    if buffer != "status" {
        ctx.target = Some(buffer.clone());
    }

    let channel_state = if !channel.is_empty() {
        let app_state = state.app_state.lock().await;
        snapshot_channel_state(&app_state, &network, &channel)
    } else {
        None
    };

    if let Some(ref py_id) = address.python_id {
        let mut py = state.python_runtime.lock().await;
        let result = py.run_menu_item_with_state(py_id, &ctx, &args, answers, channel_state.as_ref()).await?;
        let ops = py.take_dialog_ops();
        drop(py);
        emit_dialog_ops(Some(&app_handle), ops);
        deliver_script_result(state.inner().as_ref(), Some(&app_handle), &network, &buffer, &result).await;
        return Ok(());
    }
    let result = state
        .script_runtime
        .lock()
        .await
        .run_menu_item(&address, &ctx, &args, answers)?;
    deliver_script_result(state.inner().as_ref(), Some(&app_handle), &network, &buffer, &result).await;
    Ok(())
}

#[tauri::command]
async fn write_script_file(
    name: String,
    content: String,
    expected_mtime: Option<u64>,
    backup: Option<bool>,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<u64, ScriptError> {
    state
        .scripts
        .write_with_backup(&name, &content, expected_mtime, backup.unwrap_or(true))
}

#[tauri::command]
async fn create_script_file(
    name: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<ScriptFileInfo, ScriptError> {
    state.scripts.create(&name)
}

/// The editor reports unsaved changes so closing the main window can ask first.
#[tauri::command]
fn set_scripts_dirty(dirty: bool, state: State<'_, Arc<AppSharedState>>) {
    state.scripts_dirty.store(dirty, Ordering::SeqCst);
}

/// Runs `exit` scripts once (bounded so a stuck runtime cannot block exit), then quits the app.
async fn finish_exit(state: Arc<AppSharedState>, app_handle: tauri::AppHandle) {
    if state.exiting.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        run_lifecycle_event(&state, Some(&app_handle), None, "exit", String::new()),
    )
    .await;
    app_handle.exit(0);
}

/// Called by the editor after the user chose to leave with unsaved changes.
#[tauri::command]
async fn exit_after_script_prompt(app_handle: tauri::AppHandle, state: State<'_, Arc<AppSharedState>>) -> Result<(), String> {
    finish_exit(state.inner().clone(), app_handle).await;
    Ok(())
}

/// Chat logging switches. Files go to logs\<network>\<target>.log in the project folder.
#[tauri::command]
fn set_logging(
    enabled: bool,
    channel: bool,
    query: bool,
    status: bool,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut settings = state
        .logging
        .lock()
        .map_err(|_| "Logging state unavailable".to_string())?;
    settings.enabled = enabled;
    settings.channel = channel;
    settings.query = query;
    settings.status = status;
    Ok(())
}

/// Connect debug log on or off. File: logs\connect-debug.log in the project folder.
#[tauri::command]
fn set_connect_debug(enabled: bool, state: State<'_, Arc<AppSharedState>>) {
    state.debug_log.set_enabled(enabled);
}

/// Start or stop the identd responder. Binds all interfaces so IRC servers can reach it.
#[tauri::command]
async fn set_identd(
    enabled: bool,
    port: u16,
    username: String,
    state: State<'_, Arc<AppSharedState>>,
) -> Result<(), String> {
    let mut slot = state.identd.lock().await;
    if !enabled {
        if let Some(handle) = slot.take() {
            handle.abort();
            let _ = handle.await;
        }
        return Ok(());
    }
    if port == 0 {
        return Err("Identd port must be 1-65535".to_string());
    }
    if !valid_identd_username(&username) {
        return Err(
            "Identd username: at most 32 characters, letters, digits, '-', '_' or '.'".to_string(),
        );
    }
    // Free the old listener first so the same port can be bound again.
    if let Some(handle) = slot.take() {
        handle.abort();
        let _ = handle.await;
    }
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|error| format!("Identd could not listen on port {}: {}", port, error))?;
    *slot = Some(tokio::spawn(async move {
        let _ = identd::serve(listener, username).await;
    }));
    Ok(())
}

/// notes.txt in the project folder; empty text when the file does not exist.
#[tauri::command]
fn read_notes(state: State<'_, Arc<AppSharedState>>) -> Result<String, String> {
    match fs::read_to_string(state.paths.project_dir.join("notes.txt")) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("Failed to read notes: {}", error)),
    }
}

/// Writes notes.txt through a temp file and a rename, so a crash keeps the old notes.
#[tauri::command]
fn write_notes(text: String, state: State<'_, Arc<AppSharedState>>) -> Result<(), String> {
    if text.len() > MAX_NOTES_BYTES {
        return Err("Notes are too large (64 KiB maximum)".to_string());
    }
    let dir = &state.paths.project_dir;
    fs::create_dir_all(dir).map_err(|e| format!("Failed to create the notes folder: {}", e))?;
    let target = dir.join("notes.txt");
    // A temp file per write, so two writes in one process never share a file.
    static NOTES_WRITES: AtomicU64 = AtomicU64::new(0);
    let seq = NOTES_WRITES.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!("notes.txt.{}.{seq}.tmp", std::process::id()));
    let result = fs::File::create(&temp)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|_| fs::rename(&temp, &target));
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(format!("Failed to save notes: {}", error));
    }
    Ok(())
}

/// Fetches an RSS or Atom feed text over HTTP/HTTPS, through the configured proxy when there is one.
#[tauri::command]
async fn fetch_rss_feed(
    url: String,
    proxy_kind: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
) -> Result<String, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Invalid URL protocol, expected http or https".to_string());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|e| format!("Invalid URL: {e}"))?;
    let host = parsed.host_str().ok_or_else(|| "URL has no host".to_string())?;
    if is_local_host(host) {
        return Err("Feed host is a local or private address".to_string());
    }
    let proxy = build_proxy(proxy_kind, proxy_host, proxy_port, proxy_username)?;
    let password = proxy_password.filter(|p| !p.is_empty());
    let pinned = if proxy.is_none() {
        let port = parsed.port_or_known_default().unwrap_or(80);
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("Failed to resolve feed host: {e}"))?
            .collect();
        if addrs.iter().any(|a| is_local_host(&a.ip().to_string())) {
            return Err("Feed host resolves to a local or private address".to_string());
        }
        Some((host.to_string(), addrs))
    } else {
        None
    };
    let client = feed_client(proxy.as_ref(), password.as_deref(), pinned.as_ref())?;
    let mut res = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP error {}", res.status()));
    }
    if res.content_length().is_some_and(|n| n > RSS_MAX_BYTES as u64) {
        return Err(format!("Feed is larger than {} KiB", RSS_MAX_BYTES / 1024));
    }
    // Read the body in chunks and stop at the limit, so a huge file cannot fill memory.
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > RSS_MAX_BYTES {
            return Err(format!("Feed is larger than {} KiB", RSS_MAX_BYTES / 1024));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Largest RSS feed that is read, in bytes.
const RSS_MAX_BYTES: usize = 2 * 1024 * 1024;

/// The HTTP client for a feed read. Without a proxy, resolved public addresses are pinned per
/// request to prevent DNS-rebinding TOCTOU. With an HTTP proxy, traffic goes through that proxy.
/// SOCKS is not available to the feed client, so feeds are refused rather than sent around the proxy.
fn feed_client(
    proxy: Option<&ProxyConfig>,
    password: Option<&str>,
    pinned: Option<&(String, Vec<std::net::SocketAddr>)>,
) -> Result<reqwest::Client, String> {
    let mut builder = feed_client_builder();
    if let Some((host, addrs)) = pinned {
        if !addrs.is_empty() {
            builder = builder.resolve_to_addrs(host, addrs);
        }
    }
    if let Some(cfg) = proxy {
        if cfg.kind != ProxyKind::HttpConnect {
            return Err("Feeds cannot use a SOCKS proxy. Use an HTTP proxy, or turn the proxy off.".to_string());
        }
        let host = if cfg.host.contains(':') { format!("[{}]", cfg.host) } else { cfg.host.clone() };
        let mut route = reqwest::Proxy::all(format!("http://{host}:{}", cfg.port)).map_err(|e| e.to_string())?;
        if let (Some(user), Some(pass)) = (cfg.username.as_deref(), password) {
            route = route.basic_auth(user, pass);
        }
        builder = builder.proxy(route);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Settings shared by every feed client. Redirects are followed at most five times, and never to
/// a local or private address: a public feed must not be able to make the app read internal hosts.
fn feed_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent("Ferric/0.1.0 (RSS Reader)")
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if attempt.url().host_str().is_some_and(is_local_host) {
                attempt.error("a redirect to a local or private address was refused")
            } else {
                attempt.follow()
            }
        }))
}

/// Loopback, private, CGNAT, multicast, link-local and unspecified addresses, and `localhost` names.
fn is_local_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => local_v4(ip),
        Ok(std::net::IpAddr::V6(ip)) => {
            // ::ffff:127.0.0.1 and the other IPv4-in-IPv6 forms name an IPv4 address.
            if ip.to_ipv4().is_some_and(local_v4) {
                return true;
            }
            let first = ip.segments()[0];
            ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

fn local_v4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    let is_cgnat = o[0] == 100 && (o[1] & 0xc0) == 64;
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() || is_cgnat
}

#[cfg(test)]
mod local_host_tests {
    use super::is_local_host;

    #[test]
    fn local_addresses_are_refused_in_every_form() {
        for host in [
            "localhost",
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "[::1]",
            "[::ffff:127.0.0.1]",
            "[::ffff:169.254.169.254]",
            "[fd00::1]",
            "[fe80::1]",
        ] {
            assert!(is_local_host(host), "{host}");
        }
        for host in ["example.com", "8.8.8.8", "[2606:4700::1111]"] {
            assert!(!is_local_host(host), "{host}");
        }
    }
}

/// Whether a URL may be handed to the system opener: http or https, at most 2048
/// bytes, and no whitespace, control characters or quotes.
fn external_url_allowed(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://"))
        && url.len() <= 2048
        && url
            .chars()
            .all(|c| !c.is_control() && !c.is_whitespace() && !matches!(c, '"' | '\'' | '<' | '>' | '`'))
}

/// Opens a URL in the user's default browser safely.
#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    if !external_url_allowed(&url) {
        return Err("Only http and https URLs without spaces or control characters can be opened".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        // rundll32 receives the URL as one argument. cmd.exe would read & | ^ % in it as syntax.
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// View > Full Screen.
#[tauri::command]
fn set_fullscreen(enabled: bool, window: tauri::WebviewWindow) -> Result<(), String> {
    window.set_fullscreen(enabled).map_err(|e| e.to_string())
}

/// View > On Top.
#[tauri::command]
fn set_always_on_top(enabled: bool, window: tauri::WebviewWindow) -> Result<(), String> {
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn is_display_accessible() -> bool {
    // 1. Wayland detection (KDE Plasma / GNOME Wayland)
    if let Ok(wayland_display) = std::env::var("WAYLAND_DISPLAY") {
        if !wayland_display.is_empty() {
            if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
                if std::path::Path::new(&runtime_dir).join(&wayland_display).exists() {
                    return true;
                }
            } else if std::path::Path::new("/run/user").exists() {
                if let Ok(entries) = std::fs::read_dir("/run/user") {
                    for entry in entries.flatten() {
                        if entry.path().join(&wayland_display).exists() {
                            return true;
                        }
                    }
                }
            } else {
                return true;
            }
        }
    }

    // 2. X11 detection (Xorg / XWayland)
    if let Ok(display) = std::env::var("DISPLAY") {
        if display.is_empty() {
            return false;
        }
        if display.starts_with(':') {
            let num: String = display.chars().skip(1).take_while(|c| c.is_ascii_digit()).collect();
            let socket_path = format!("/tmp/.X11-unix/X{}", num);
            if std::path::Path::new(&socket_path).exists() {
                return true;
            }
        }
        if let Some(colon) = display.rfind(':') {
            let host = &display[..colon];
            let rest = &display[colon + 1..];
            let num: u16 = rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
            let port = 6000 + num;
            let target_host = if host.is_empty() || host == "localhost" { "127.0.0.1" } else { host };
            if let Ok(addr) = format!("{}:{}", target_host, port).parse() {
                if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200)).is_ok() {
                    return true;
                }
            }
        }
    }
    false
}

#[tokio::main]
async fn main() {
    #[cfg(target_os = "linux")]
    {
        if !is_display_accessible() {
            eprintln!("\n[Ferric] Kein erreichbarer Display-Server gefunden (DISPLAY oder WAYLAND_DISPLAY nicht verfügbar).");
            eprintln!("Ferric benötigt eine grafische Oberfläche (X11/Wayland).\n");
            eprintln!("Start-Optionen:");
            eprintln!("  1. Headless-Test (ohne Fenster):    xvfb-run ferric-app");
            eprintln!("  2. Remote via SSH:                  ssh -X user@host (mit lokalem X-Server)");
            eprintln!("  3. Sicher remote (Pixel-Isolation): Xpra oder VNC-Tunnel über SSH");
            eprintln!("  4. Lokaler Desktop (Monitor):       DISPLAY=:0 ferric-app\n");
            eprintln!("Sicherheitshinweis bei Remote-Betrieb:");
            eprintln!("  - Reines X11-Forwarding (ssh -X / -Y) erlaubt App-Zugriff auf lokale Tastatur/Zwischenablage.");
            eprintln!("  - Für sensible Umgebungen wird Xpra oder ein VNC-Tunnel über SSH empfohlen.\n");
            std::process::exit(1);
        }

        if let Ok(display) = std::env::var("DISPLAY") {
            let is_remote_ssh = std::env::var_os("SSH_CONNECTION").is_some()
                || std::env::var_os("SSH_CLIENT").is_some()
                || display.starts_with("localhost:")
                || display.starts_with("127.0.0.1:");
            if is_remote_ssh {
                eprintln!("[Ferric Sicherheitshinweis] SSH X11-Forwarding aktiv (DISPLAY={display}).");
                eprintln!("  X11-Forwarding erlaubt App-Zugriff auf lokale Tastatur und Zwischenablage.");
                eprintln!("  Für maximale Isolation wird Xpra oder ein VNC-Tunnel über SSH empfohlen.");
            }
        }
    }

    tauri::Builder::default()
        .setup(|app| {
            let paths = resolve_config_paths(app.handle())?;
            let log_dir = paths.project_dir.join("logs");
            // The scripts loaded in the last session load again, without a manual load.
            let script_dir = paths.script_dir();
            let scripts = ScriptStore::new(script_dir.clone());
            let mut runtime = Runtime::new();
            for name in read_loaded_list(&script_dir) {
                if !is_python_script(&name) {
                    let _ = load_aliases(&scripts, &mut runtime, &name);
                }
            }
            let perf = Arc::new(PerfTelemetry::default());
            let python_runtime = Mutex::new(PythonRuntime::new(
                paths.project_dir.clone(),
                paths.app_config_dir.clone(),
            ).with_perf(Arc::clone(&perf)));
            let state = Arc::new(AppSharedState {
                app_state: Mutex::new(AppState::new()),
                clients: Mutex::new(HashMap::new()),
                connecting: Mutex::new(HashMap::new()),
                next_conn_id: AtomicU64::new(1),
                scripts,
                script_runtime: Mutex::new(runtime),
                python_runtime,
                script_budgets: std::sync::Mutex::new(HashMap::new()),
                paths,
                scripts_dirty: AtomicBool::new(false),
                exiting: AtomicBool::new(false),
                allow_self_signed: AtomicBool::new(true),
                query_auto_keyx: AtomicBool::new(true),
                minimize_to_tray: AtomicBool::new(false),
                python_autoloaded: AtomicBool::new(false),
                debug_log: Arc::new(DebugLog::new(log_dir.clone())),
                logging: std::sync::Mutex::new(LogSettings {
                    enabled: false,
                    writer: LogWriter::start(ChatLog::new(log_dir.clone())),
                    channel: false,
                    query: false,
                    status: false,
                }),
                identd: Mutex::new(None),
                dh_pending: std::sync::Mutex::new(HashMap::new()),
                dh_init_rate: std::sync::Mutex::new(DhInitRate::default()),
                auto_dh_cooldown: std::sync::Mutex::new(HashMap::new()),
                perf: Arc::clone(&perf),
            });
            spawn_perf_logger(Arc::clone(&perf), log_dir);
            spawn_timer_driver(Arc::clone(&state), app.handle().clone());
            // start: once per launch, after the saved scripts are loaded.
            let start_state = Arc::clone(&state);
            let start_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_lifecycle_event(&start_state, Some(&start_handle), None, "start", String::new()).await;
            });
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                WindowEvent::CloseRequested { api, .. } => {
                    let dirty = window
                        .try_state::<Arc<AppSharedState>>()
                        .map(|state| state.scripts_dirty.load(Ordering::SeqCst))
                        .unwrap_or(false);
                    if dirty {
                        api.prevent_close();
                        let _ = window.emit("script-close-requested", ());
                    } else if let Some(state) = window.try_state::<Arc<AppSharedState>>() {
                        // exit: run off the UI thread so scripts cannot deadlock window close.
                        let state = state.inner().clone();
                        if !state.exiting.load(Ordering::SeqCst) {
                            api.prevent_close();
                            let handle = window.app_handle().clone();
                            tauri::async_runtime::spawn(finish_exit(state, handle));
                        }
                    }
                }
                WindowEvent::Focused(true) => {
                    // appactive: the application window got focus; the active network's scripts run.
                    if let Some(state) = window.try_state::<Arc<AppSharedState>>() {
                        let state = state.inner().clone();
                        let handle = window.app_handle().clone();
                        tauri::async_runtime::spawn(async move {
                            let network = state.app_state.lock().await.active_network.clone();
                            run_lifecycle_event(&state, Some(&handle), network.as_deref(), "appactive", String::new()).await;
                        });
                    }
                }
                WindowEvent::Resized(_) => {
                    if let Some(state) = window.try_state::<Arc<AppSharedState>>() {
                        if state.minimize_to_tray.load(Ordering::Relaxed)
                            && window.is_minimized().unwrap_or(false)
                        {
                            let _ = window.set_skip_taskbar(true);
                            let _ = window.hide();
                        }
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect_network,
            close_network,
            load_connectd,
            send_message,
            send_raw,
            set_fish_key,
            tls_fingerprint,
            set_allow_self_signed,
            set_query_auto_keyx,
            set_minimize_to_tray,
            set_python_settings,
            get_fish_keys,
            get_buffer_fish_key,
            save_fish_key_entry,
            delete_fish_key_entry,
            get_server_list,
            save_server_list,
            get_server_order,
            save_server_order,
            start_dh1080,
            open_query,
            close_buffer,
            set_active_buffer,
            get_state,
            get_backend_perf,
            get_buffer_history,
            clear_buffer_history,
            list_script_files,
            read_script_file,
            load_script_aliases,
            unload_script_aliases,
            run_script_alias,
            script_dialog_event,
            script_menu_nodes,
            run_script_menu_item,
            write_script_file,
            create_script_file,
            set_scripts_dirty,
            exit_after_script_prompt,
            set_fullscreen,
            set_always_on_top,
            set_logging,
            set_connect_debug,
            set_identd,
            read_notes,
            write_notes,
            fetch_rss_feed,
            open_external_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running Ferric application");
}

// ---- Server manager: the server lines of connectd.mrc ----

/// What the Server manager shows: the server rows and the modification time to pass back on save.
#[derive(serde::Serialize)]
struct ServerListView {
    file: String,
    rows: Vec<ServerRow>,
    mtime: Option<u64>,
}

fn connectd_file_name(paths: &AppConfigPaths) -> String {
    paths
        .connectd_mrc
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "connectd.mrc".to_string())
}

fn read_server_list(store: &ScriptStore, file: &str) -> Result<ServerListView, String> {
    match store.read(file) {
        Ok(content) => Ok(ServerListView {
            file: file.to_string(),
            rows: rows_from_text(&content.content),
            mtime: Some(content.mtime),
        }),
        Err(e) if e.code == "not_found" => Ok(ServerListView {
            file: file.to_string(),
            rows: Vec::new(),
            mtime: None,
        }),
        Err(e) => Err(e.message),
    }
}

/// Replaces the server lines of `file` with `rows`; every other line stays. A missing file is created.
/// `expected` is the modification time the list was read with; a file changed since is refused.
fn write_server_list(
    store: &ScriptStore,
    file: &str,
    rows: &[ServerRow],
    expected: Option<u64>,
) -> Result<ServerListView, String> {
    let current = match store.read(file) {
        Ok(content) => Some(content.content),
        Err(e) if e.code == "not_found" => None,
        Err(e) => return Err(e.message),
    };
    let text = text_with_rows(current.as_deref().unwrap_or(""), rows)?;
    let mtime = if current.is_some() {
        store.write(file, &text, expected).map_err(|e| e.message)?
    } else if expected.is_some() {
        return Err(format!("{file} was removed on disk; reload the list before saving."));
    } else {
        let created = store.create(file).map_err(|e| e.message)?;
        store.write(file, &text, Some(created.mtime)).map_err(|e| e.message)?
    };
    Ok(ServerListView {
        file: file.to_string(),
        rows: rows_from_text(&text),
        mtime: Some(mtime),
    })
}

/// The server order and network names (`ferric-servers.txt` next to connectd.mrc), in startup order.
fn server_meta_path(paths: &AppConfigPaths) -> std::path::PathBuf {
    paths.connectd_mrc.with_file_name("ferric-servers.txt")
}

#[tauri::command]
async fn get_server_order(state: State<'_, Arc<AppSharedState>>) -> Result<Vec<ferric_core::ServerMeta>, String> {
    let app = state.inner().clone();
    tokio::task::spawn_blocking(move || ferric_core::load_server_meta(server_meta_path(&app.paths)))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_server_order(
    state: State<'_, Arc<AppSharedState>>,
    entries: Vec<ferric_core::ServerMeta>,
) -> Result<(), String> {
    let app = state.inner().clone();
    let saved = entries.clone();
    tokio::task::spawn_blocking(move || ferric_core::save_server_meta(server_meta_path(&app.paths), &entries))
        .await
        .map_err(|e| e.to_string())??;
    // The sidebar follows the enabled flag at once, without a reload of connectd.mrc.
    let servers = parse_connectd_mrc(&state.paths.connectd_mrc)?;
    let mut app_st = state.app_state.lock().await;
    for server in servers {
        let key = ferric_core::server_meta_key(&server.host, server.port, &server.username);
        if let Some(meta) = saved.iter().find(|m| ferric_core::server_meta_key(&m.host, m.port, &m.ident) == key) {
            if let Some(net) = app_st.networks.get_mut(&server.network_id) {
                net.enabled = meta.enabled;
            }
        }
    }
    app_st.mark_dirty();
    Ok(())
}

#[tauri::command]
async fn get_server_list(state: State<'_, Arc<AppSharedState>>) -> Result<ServerListView, String> {
    let app = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let file = connectd_file_name(&app.paths);
        read_server_list(&app.scripts, &file)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn save_server_list(
    state: State<'_, Arc<AppSharedState>>,
    rows: Vec<ServerRow>,
    expected_mtime: Option<u64>,
) -> Result<ServerListView, String> {
    let app = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let file = connectd_file_name(&app.paths);
        write_server_list(&app.scripts, &file, &rows, expected_mtime)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod server_list_tests {
    use super::{read_server_list, write_server_list};
    use ferric_core::server_list::ServerRow;
    use ferric_core::ScriptStore;
    use std::time::Duration;

    fn row(host: &str) -> ServerRow {
        ServerRow {
            host: host.into(),
            port: 6667,
            use_tls: false,
            password: None,
            nick: "ferric".into(),
            altnick: "ferric_".into(),
            username: "ferric".into(),
            realname: "Ferric User".into(),
        }
    }

    #[test]
    fn saving_keeps_other_lines_and_refuses_a_file_changed_since_the_read() {
        let dir = std::env::temp_dir().join(format!("ferric_server_list_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = ScriptStore::new(dir.clone());

        let missing = read_server_list(&store, "connectd.mrc").unwrap();
        assert!(missing.rows.is_empty());
        assert_eq!(missing.mtime, None);
        // The first save creates the missing file.
        assert_eq!(write_server_list(&store, "connectd.mrc", &[row("a.org")], None).unwrap().rows.len(), 1);

        std::fs::write(dir.join("connectd.mrc"), "; mine\nserver -m a.org:6667 -i x y z@\n").unwrap();
        let view = read_server_list(&store, "connectd.mrc").unwrap();
        assert_eq!(view.rows.len(), 1);
        let stale = view.mtime;
        std::thread::sleep(Duration::from_millis(20));
        let out = write_server_list(&store, "connectd.mrc", &[row("b.org"), row("c.org")], stale).unwrap();
        assert_eq!(out.rows.len(), 2);
        let text = std::fs::read_to_string(dir.join("connectd.mrc")).unwrap();
        assert!(text.starts_with("; mine\nserver -m b.org:6667"), "{text}");
        assert_eq!(text.matches("server -m ").count(), 2);

        // The time read before the second save is now stale.
        assert!(write_server_list(&store, "connectd.mrc", &[], stale).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod loaded_scripts_tests {
    use super::*;

    #[test]
    fn loaded_scripts_are_remembered_once_forgotten_and_load_again() {
        let dir = std::env::temp_dir().join(format!("ferric_loaded_scripts_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hello.mrc"), "alias hi { echo hi }\n").unwrap();

        remember_loaded(&dir, "hello.mrc").unwrap();
        remember_loaded(&dir, "HELLO.mrc").unwrap();
        assert_eq!(read_loaded_list(&dir), ["hello.mrc"]);

        // The next start loads the listed script without a manual load.
        let mut runtime = Runtime::new();
        let store = ScriptStore::new(dir.clone());
        for name in read_loaded_list(&dir) {
            load_aliases(&store, &mut runtime, &name).unwrap();
        }
        assert!(run_loaded_alias(&mut runtime, "hi", &[], &ScriptContext::default()).is_some());

        forget_loaded(&dir, "Hello.MRC").unwrap();
        assert!(read_loaded_list(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_scripts_are_returned_when_list_file_is_missing() {
        let dir = std::env::temp_dir().join(format!("ferric_default_scripts_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("services.mrc"), "alias srv { echo srv }\n").unwrap();
        fs::write(dir.join("channelcentral.py"), "# py\n").unwrap();
        assert_eq!(read_loaded_list(&dir), ["services.mrc", "channelcentral.py"]);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod fish_scope_tests {
    use super::fish_section;

    #[test]
    fn a_network_key_is_scoped_to_that_network() {
        assert_eq!(fish_section(Some("libera"), "#chat"), ("libera:#chat".to_string(), true));
        assert_eq!(fish_section(None, "#chat"), ("#chat".to_string(), false));
        assert_eq!(fish_section(Some(""), "#chat"), ("#chat".to_string(), false));
        assert_eq!(fish_section(Some("libera"), "efnet:#chat"), ("efnet:#chat".to_string(), false));
    }
}

#[cfg(test)]
mod external_url_tests {
    use super::external_url_allowed;

    #[test]
    fn only_plain_http_urls_are_allowed() {
        assert!(external_url_allowed("https://example.com/?x=1&y=2"));
        assert!(external_url_allowed("http://example.com/a%20b"));
        assert!(!external_url_allowed("file:///C:/Windows/system.ini"));
        assert!(!external_url_allowed("https://example.com/ & calc.exe"));
        assert!(!external_url_allowed("https://example.com/\"x\""));
        assert!(!external_url_allowed("https://example.com/\n"));
        assert!(!external_url_allowed(&format!("https://e.com/{}", "a".repeat(2100))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_script_load_and_alias_invocation() {
        let dir = std::env::temp_dir().join(format!("ferric-script-test-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("test.mrc"), "alias greet { echo hello $1 }\n").unwrap();
        let store = ScriptStore::new(dir.clone());
        let mut runtime = Runtime::new();
        assert!(run_loaded_alias(&mut runtime, "greet", &[], &ScriptContext::default()).is_none());
        assert!(load_aliases(&store, &mut runtime, "../test.mrc").is_err());
        assert!(load_aliases(&store, &mut runtime, "test.mrc").unwrap().1.is_empty());
        let result = run_loaded_alias(&mut runtime, "GREET", &["Alice".into()], &ScriptContext::default()).unwrap();
        assert_eq!(result.effects, ["hello Alice"]);
        assert!(result.notices.is_empty());
        assert!(runtime.unload("test.mrc"));
        assert!(run_loaded_alias(&mut runtime, "greet", &[], &ScriptContext::default()).is_none());
        fs::remove_file(dir.join("test.mrc")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    fn test_shared_state() -> AppSharedState {
        let dir = std::env::temp_dir().join("ferric-event-test");
        AppSharedState {
            app_state: Mutex::new(AppState::new()),
            clients: Mutex::new(HashMap::new()),
            connecting: Mutex::new(HashMap::new()),
            next_conn_id: AtomicU64::new(1),
            paths: AppConfigPaths {
                blow_ini: dir.join("blow.ini"),
                connectd_mrc: dir.join("connectd.mrc"),
                project_dir: dir.clone(),
                app_config_dir: dir.clone(),
            },
            scripts: ScriptStore::new(dir.clone()),
            script_runtime: Mutex::new(Runtime::new()),
            python_runtime: Mutex::new(PythonRuntime::new(dir.clone(), dir.clone())),
            script_budgets: std::sync::Mutex::new(HashMap::new()),
            scripts_dirty: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            allow_self_signed: AtomicBool::new(true),
            query_auto_keyx: AtomicBool::new(true),
            minimize_to_tray: AtomicBool::new(false),
            python_autoloaded: AtomicBool::new(false),
            logging: std::sync::Mutex::new(LogSettings {
                enabled: false,
                writer: LogWriter::start(ChatLog::new(dir.clone())),
                channel: false,
                query: false,
                status: false,
            }),
            debug_log: Arc::new(DebugLog::new(dir)),
            identd: Mutex::new(None),
            dh_pending: std::sync::Mutex::new(HashMap::new()),
            dh_init_rate: std::sync::Mutex::new(DhInitRate::default()),
            auto_dh_cooldown: std::sync::Mutex::new(HashMap::new()),
            perf: Arc::new(PerfTelemetry::default()),
        }
    }

    fn incoming_message(sender: &str, target: &str, text: &str) -> IrcEvent {
        IrcEvent::Message { sender_address: None,
            network: "net".into(),
            target: target.into(),
            sender: sender.into(),
            text: text.into(),
            spans: Vec::new(),
            is_notice: false,
            is_action: false,
            is_encrypted: false,
            timestamp: 1,
            sender_host: None,
        }
    }

    #[test]
    fn script_output_goes_to_its_echo_target() {
        use ferric_core::mrc::interp::Effect;
        let result = RunResult {
            effects: vec![
                Effect::Echo("plain".into()),
                Effect::EchoTarget { target: EchoTarget::Status, text: "to status".into() },
                Effect::EchoTarget { target: EchoTarget::Active, text: "to active".into() },
            ],
            notices: vec!["notice".into()],
            ..Default::default()
        };
        let got: Vec<(String, String, String)> = place_script_output(&result, "net", "#chan", Some(("other", "query")))
            .into_iter()
            .map(|l| (l.network, l.buffer, l.text))
            .collect();
        let want = [
            ("net", "#chan", "plain"),
            ("net", "status", "to status"),
            ("other", "query", "to active"),
            ("net", "#chan", "notice"),
        ];
        assert_eq!(got, want.map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())));

        let no_active = place_script_output(&result, "net", "#chan", None);
        assert_eq!(no_active[2].network, "net");
        assert_eq!(no_active[2].buffer, "#chan");
    }

    #[test]
    fn event_buffer_is_channel_query_or_status() {
        assert_eq!(event_buffer(&incoming_message("Alice", "#Chan", "x")), "#chan");
        assert_eq!(event_buffer(&incoming_message("Alice", "Me", "x")), "alice");
        let quit = IrcEvent::Quit { address: None, network: "net".into(), nick: "bob".into(), message: None };
        assert_eq!(event_buffer(&quit), "status");
    }

    /// Scripts see the message first and their echo lands after it, in the channel buffer.
    #[tokio::test]
    async fn script_output_lands_after_the_message_that_caused_it() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("seen.mrc", "on *:TEXT:*:#:{ echo seen $nick }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        process_event(&state, None, "net", &incoming_message("alice", "#a", "hello")).await;

        let app = state.app_state.lock().await;
        let buffer = app.networks["net"].buffers.get("#a").expect("channel buffer");
        let texts: Vec<&str> = buffer.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["hello", "seen alice"]);
    }

    /// A kick reaches the scripts through process_event, with the kicked nick as $knick.
    #[tokio::test]
    async fn kick_reaches_the_scripts_with_the_kicked_nick() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("k.mrc", "on *:KICK:#:{ echo $knick was kicked by $nick }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        let kick = IrcEvent::Kick {
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            by: "op".into(),
            reason: None,
        };
        process_event(&state, None, "net", &kick).await;

        let app = state.app_state.lock().await;
        let status = &app.networks["net"].buffers["status"];
        assert!(status.messages.iter().any(|m| m.text == "bob was kicked by op"));
    }

    /// $snicks of a popup is the selected nicks, joined with commas.
    #[tokio::test]
    async fn popup_snicks_are_the_selected_nicks_joined_by_commas() {
        let state = test_shared_state();
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        let nicks = vec!["alice".to_string(), "bob".to_string()];
        let (ctx, args) = popup_context(&state, "net", "alice", &nicks, "#a").await.unwrap();
        assert_eq!(ctx.snicks.as_deref(), Some("alice,bob"));
        assert_eq!(args, nicks);
    }

    /// A popup value with a line break is refused before any script sees it.
    #[tokio::test]
    async fn popup_values_with_line_breaks_are_refused() {
        let state = test_shared_state();
        let nicks = vec!["a
quit".to_string()];
        assert!(popup_context(&state, "net", "a
quit", &nicks, "").await.is_err());
    }

    /// A timer made by an event runs once it is due, and writes to the network's status window.
    #[tokio::test]
    async fn a_due_timer_writes_its_output_to_the_status_window() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("tm.mrc", "on *:TEXT:*:#:{ timer -m tt 1 100 echo hi }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        process_event(&state, None, "net", &incoming_message("alice", "#a", "hello")).await;
        // The interval is 100 ms, so the run is due after a short wait.
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        run_due_timers(&state, None).await;

        let app = state.app_state.lock().await;
        let status = &app.networks["net"].buffers["status"];
        assert_eq!(status.messages.iter().filter(|m| m.text == "hi").count(), 1);
    }

    /// A disconnect stops the online timers of that network before they run.
    #[tokio::test]
    async fn a_disconnect_stops_the_online_timers_of_its_network() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("tm.mrc", "on *:TEXT:*:#:{ timer -m tt 1 100 echo hi }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        process_event(&state, None, "net", &incoming_message("alice", "#a", "hello")).await;
        process_event(&state, None, "net", &IrcEvent::Disconnected { network: "net".into(), reason: "bye".into() }).await;
        run_due_timers(&state, None).await;

        let app = state.app_state.lock().await;
        let status = &app.networks["net"].buffers["status"];
        assert!(!status.messages.iter().any(|m| m.text == "hi"));
    }

    /// The window the UI reports as shown is the one the backend sends lists for. An unknown
    /// network changes nothing, so the shown network stays the one it was.
    #[tokio::test]
    async fn reported_window_becomes_the_shown_one_for_a_known_network_only() {
        let state = test_shared_state();
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;

        let mut app = state.app_state.lock().await;
        assert!(set_active_buffer_state(&mut app, "net", "#Chan"));
        assert_eq!(app.active_network.as_deref(), Some("net"));
        assert_eq!(app.networks["net"].active_buffer, "#chan");

        assert!(!set_active_buffer_state(&mut app, "gone", "status"));
        assert_eq!(app.active_network.as_deref(), Some("net"));
    }

    /// `/query` from a script opens and activates the query window, even without a connection.
    #[tokio::test]
    async fn script_query_opens_the_window_and_activates_it() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("q.mrc", "on *:TEXT:*:#:{ query $nick hi }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        process_event(&state, None, "net", &incoming_message("alice", "#a", "hello")).await;

        let app = state.app_state.lock().await;
        let net = &app.networks["net"];
        assert!(net.buffers.contains_key("alice"));
        assert_eq!(net.active_buffer, "alice");
    }

    /// A script line on a network with no connection is dropped, with one status notice.
    #[tokio::test]
    async fn script_send_without_a_connection_is_one_notice() {
        let state = test_shared_state();
        state.script_runtime.lock().await.load("send.mrc", "on *:TEXT:*:#:{ msg $chan got it }");
        process_event(&state, None, "net", &IrcEvent::Connected { network: "net".into(), server: "irc.test".into() }).await;
        process_event(&state, None, "net", &incoming_message("alice", "#a", "hello")).await;

        let app = state.app_state.lock().await;
        let status = &app.networks["net"].buffers["status"];
        let notices: Vec<&str> = status
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .filter(|t| t.contains("not connected"))
            .collect();
        assert_eq!(notices.len(), 1, "{notices:?}");
    }

    /// Regression: the app build enables rustls' `ring` and `aws_lc_rs` features together.
    /// Without an explicit provider the connect task panicked and sent no event.
    #[tokio::test]
    async fn tls_connect_reports_a_stalled_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _stall = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let config = ferric_core::ConnectConfig {
            network: "test".into(),
            host: "127.0.0.1".into(),
            port,
            use_tls: true,
            nick: "tester".into(),
            username: "tester".into(),
            realname: "tester".into(),
            password: None,
            altnick: None,
            sasl: None,
            tls_fingerprint: None,
            allow_self_signed: false,
        };
        let mut client = ferric_core::IrcClient::new(config, tx);
        client.set_connect_timeout(std::time::Duration::from_millis(300));
        client.connect().await.unwrap();
        match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
            Ok(Some(ferric_core::IrcEvent::Error { message, .. })) => {
                assert!(message.contains("at tls handshake"), "{message}");
            }
            other => panic!("expected an error event, got {other:?}"),
        }
    }

    #[test]
    fn own_line_log_exclusion() {
        assert!(own_line_loggable("hello there"));
        assert!(!own_line_loggable("/msg nickserv hi"));
        assert!(!own_line_loggable("  /me waves"));
        for word in ["identify", "password", "pass", "ghost", "recover", "register", "oper"] {
            assert!(!own_line_loggable(&format!("x {} y", word)), "{word}");
            assert!(!own_line_loggable(&word.to_uppercase()), "{word}");
        }
        assert!(!own_line_loggable("IdEnTiFy hunter"));
    }

    #[test]
    fn dh1080_nick_validation() {
        assert!(valid_dh_nick("Alice"));
        assert!(valid_dh_nick("[a]\\`_^{|}-9"));
        assert!(valid_dh_nick(&"a".repeat(32)));
        assert!(!valid_dh_nick(""));
        assert!(!valid_dh_nick("bad name"));
        assert!(!valid_dh_nick("bad:name"));
        assert!(!valid_dh_nick("a,b"));
        assert!(!valid_dh_nick("a\r\nb"));
        assert!(!valid_dh_nick("#chan"));
        assert!(!valid_dh_nick(&"a".repeat(33)));
    }

    fn pending_key(i: usize) -> (String, String) {
        dh_key("net", &format!("nick{i}"))
    }

    #[test]
    fn dh1080_pending_keeps_at_most_16() {
        let t0 = Instant::now();
        let mut map = DhPending::new();
        for i in 0..20 {
            let pair = Dh1080Keypair::generate().unwrap();
            dh_pending_insert(&mut map, pending_key(i), pair, t0 + Duration::from_secs(i as u64));
        }
        assert_eq!(map.len(), DH_PENDING_MAX);
        assert!(!map.contains_key(&pending_key(0)));
        assert!(map.contains_key(&pending_key(19)));
    }

    #[test]
    fn dh1080_pending_drops_old_entries() {
        let t0 = Instant::now();
        let mut map = DhPending::new();
        dh_pending_insert(&mut map, pending_key(1), Dh1080Keypair::generate().unwrap(), t0);
        dh_pending_insert(&mut map, pending_key(2), Dh1080Keypair::generate().unwrap(), t0 + Duration::from_secs(10));
        // A new entry 301 s after the first drops it; the one 291 s old stays.
        dh_pending_insert(&mut map, pending_key(3), Dh1080Keypair::generate().unwrap(), t0 + Duration::from_secs(301));
        assert!(!map.contains_key(&pending_key(1)));
        assert!(map.contains_key(&pending_key(2)));
        assert!(map.contains_key(&pending_key(3)));
        // take: fresh entry works once, stale entry is refused.
        let now = t0 + Duration::from_secs(302);
        assert!(dh_pending_take(&mut map, &pending_key(3), now).is_some());
        assert!(dh_pending_take(&mut map, &pending_key(3), now).is_none());
        assert!(dh_pending_take(&mut map, &pending_key(2), t0 + Duration::from_secs(311)).is_none());
        assert!(map.is_empty());
    }

    #[test]
    fn dh1080_init_rate_limits() {
        let t0 = Instant::now();
        let mut rate = DhInitRate::default();
        // 3 per nick per minute, case-insensitive; dropped lines are not counted.
        assert!(rate.allow("net", "Alice", t0));
        assert!(rate.allow("net", "alice", t0 + Duration::from_secs(1)));
        assert!(rate.allow("net", "ALICE", t0 + Duration::from_secs(2)));
        assert!(!rate.allow("net", "alice", t0 + Duration::from_secs(3)));
        assert!(rate.allow("other", "alice", t0 + Duration::from_secs(3)));
        assert!(rate.allow("net", "alice", t0 + Duration::from_secs(61)));
        // 20 per network per minute.
        let mut rate = DhInitRate::default();
        for i in 0..20 {
            assert!(rate.allow("net", &format!("nick{i}"), t0), "{i}");
        }
        assert!(!rate.allow("net", "nick20", t0 + Duration::from_secs(30)));
        assert!(rate.allow("other", "nick20", t0 + Duration::from_secs(30)));
        assert!(rate.allow("net", "nick20", t0 + Duration::from_secs(60)));
    }

    #[test]
    fn identd_username_check() {
        assert!(valid_identd_username("ferric"));
        assert!(valid_identd_username("A-b_c.9"));
        assert!(valid_identd_username(&"a".repeat(32)));
        assert!(!valid_identd_username(&"a".repeat(33)));
        assert!(!valid_identd_username("bad name"));
        assert!(!valid_identd_username("bad:name"));
        assert!(!valid_identd_username("nick\r\n"));
        assert!(!valid_identd_username("\u{e9}"));
    }

    #[test]
    fn auto_keyx_target_validation() {
        assert!(is_valid_auto_keyx_target("Alice", "Bob"));
        assert!(is_valid_auto_keyx_target("cool_user", "Bob"));
        // Own nick
        assert!(!is_valid_auto_keyx_target("Bob", "Bob"));
        assert!(!is_valid_auto_keyx_target("bob", "BOB"));
        // Channel
        assert!(!is_valid_auto_keyx_target("#channel", "Bob"));
        assert!(!is_valid_auto_keyx_target("&channel", "Bob"));
        // Status
        assert!(!is_valid_auto_keyx_target("status", "Bob"));
        assert!(!is_valid_auto_keyx_target("STATUS", "Bob"));
        // Services
        assert!(!is_valid_auto_keyx_target("NickServ", "Bob"));
        assert!(!is_valid_auto_keyx_target("chanserv", "Bob"));
        assert!(!is_valid_auto_keyx_target("MemoServ", "Bob"));
        assert!(!is_valid_auto_keyx_target("OperServ", "Bob"));
        assert!(!is_valid_auto_keyx_target("Auth", "Bob"));
        // Invalid syntax
        assert!(!is_valid_auto_keyx_target("", "Bob"));
        assert!(!is_valid_auto_keyx_target("invalid name", "Bob"));
    }

    #[test]
    fn query_buffer_ensure_and_open_lifecycle() {
        let mut st = AppState::new();
        assert_eq!(ensure_query_buffer(&mut st, "net", "alice"), None);
        assert_eq!(open_query_buffer(&mut st, "net", "alice"), None);

        // Add network
        st.networks.insert("net".to_string(), NetworkState {
            id: "net".to_string(),
            server: "irc.test".to_string(),
            nick: "Bob".to_string(),
            connected: true,
            active_buffer: "status".to_string(),
            buffers: BTreeMap::new(),
            label: String::new(),
            order: None,
            enabled: true,
        });

        // ensure_query_buffer does NOT switch active_buffer
        assert_eq!(st.networks["net"].active_buffer, "status");
        assert_eq!(ensure_query_buffer(&mut st, "net", "alice"), Some(true));
        assert!(st.networks["net"].buffers.contains_key("alice"));
        assert_eq!(st.networks["net"].active_buffer, "status");

        // Calling again indicates it was NOT new
        assert_eq!(ensure_query_buffer(&mut st, "net", "alice"), Some(false));

        // open_query_buffer DOES switch active_buffer
        assert_eq!(open_query_buffer(&mut st, "net", "alice"), Some(false));
        assert_eq!(st.networks["net"].active_buffer, "alice");

        // New buffer via open_query_buffer
        assert_eq!(open_query_buffer(&mut st, "net", "charlie"), Some(true));
        assert!(st.networks["net"].buffers.contains_key("charlie"));
        assert_eq!(st.networks["net"].active_buffer, "charlie");
    }

    #[tokio::test]
    async fn connectfail_runs_for_connection_failure_errors_only() {
        let state = test_shared_state();
        state
            .script_runtime
            .lock()
            .await
            .load("cf.mrc", "on *:CONNECTFAIL:{ echo cf $1- }\n");

        let cert_notice = IrcEvent::Error {
            network: "net".into(),
            message: "Certificate checked (self-signed allowed)".into(),
        };
        process_event(&state, None, "net", &cert_notice).await;

        let conn_fail = IrcEvent::Error {
            network: "net".into(),
            message: "Connection failed to 127.0.0.1:6667 at tcp connect: refused".into(),
        };
        process_event(&state, None, "net", &conn_fail).await;

        let st = state.app_state.lock().await;
        let msgs: Vec<&str> = st.networks["net"].buffers["status"]
            .messages
            .iter()
            .map(|m| m.text.as_str())
            .collect();
        assert!(msgs.iter().any(|t| *t == "cf Connection failed to 127.0.0.1:6667 at tcp connect: refused"), "{msgs:?}");
        assert!(!msgs.iter().any(|t| t.starts_with("cf Certificate")), "{msgs:?}");
    }
}
