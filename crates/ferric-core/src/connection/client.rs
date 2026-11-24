use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use rustls::client::danger::ServerCertVerifier;

use crate::connection::event::IrcEvent;
use crate::debuglog::{DebugLog, Trace};
use crate::crypto::fish::FishCipher;
use crate::protocol::{IrcMessage, IrcPrefix};

/// `user@host` of a prefix when the server gave both parts.
fn address_of(p: &IrcPrefix) -> Option<String> {
    match (&p.user, &p.host) {
        (Some(user), Some(host)) => Some(format!("{user}@{host}")),
        _ => None,
    }
}
use crate::proxy::{self, ProxyConfig};
use crate::text::{decode_lossy, is_channel_name_with, split_text};

/// Checks a server by its certificate fingerprint instead of the CA roots. With `pin`
/// set, only that certificate passes. The handshake signatures are checked either way.
/// `strict` makes the unpinned path run full webpki verification (chain of public
/// roots, validity period, host name); `trust_self_signed` additionally accepts a
/// self-signed certificate (the presented cert as its own anchor), so outright junk
/// is still refused. The fingerprint probe uses neither: it connects blind to read
/// the fingerprint the user is about to compare.
#[derive(Debug)]
struct PinnedCertificate {
    pin: Option<[u8; 32]>,
    strict: bool,
    trust_self_signed: bool,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
    provider: Arc<rustls::crypto::CryptoProvider>,
    roots: Arc<rustls::RootCertStore>,
}

impl PinnedCertificate {
    fn new(
        pin: Option<[u8; 32]>,
        strict: bool,
        trust_self_signed: bool,
        provider: Arc<rustls::crypto::CryptoProvider>,
    ) -> Self {
        let algorithms = provider.signature_verification_algorithms;
        Self { pin, strict, trust_self_signed, algorithms, provider, roots: Arc::new(tls_roots()) }
    }

    fn webpki_verify(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &ServerName<'_>,
        now: rustls::pki_types::UnixTime,
        roots: Arc<rustls::RootCertStore>,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(roots, self.provider.clone())
            .build()
            .map_err(|e| rustls::Error::General(e.to_string()))?;
        verifier.verify_server_cert(end_entity, intermediates, server_name, &[], now)
    }
}

impl rustls::client::danger::ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        match self.pin {
            Some(pin) => {
                let digest = <sha2::Sha256 as sha2::Digest>::digest(end_entity.as_ref());
                if digest.as_slice() == pin {
                    Ok(rustls::client::danger::ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General(
                        "the server certificate is not the pinned one: it changed since the first connect".into(),
                    ))
                }
            }
            None if !self.strict => Ok(rustls::client::danger::ServerCertVerified::assertion()),
            None => {
                let first = self.webpki_verify(end_entity, intermediates, server_name, now, self.roots.clone());
                if first.is_ok() || !self.trust_self_signed {
                    return first;
                }
                // Self-signed fallback: the presented certificate anchors itself.
                // Chain, validity period and host name are still checked.
                let mut anchored = rustls::RootCertStore::empty();
                anchored.add_parsable_certificates(std::iter::once(end_entity.clone()));
                let second = self.webpki_verify(
                    end_entity,
                    intermediates,
                    server_name,
                    now,
                    Arc::new(anchored),
                );
                if second.is_ok() { second } else { first }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A SHA-256 fingerprint of 64 hex digits, with optional ':' or spaces between them.
pub fn parse_fingerprint(text: &str) -> Result<[u8; 32], String> {
    let hex: String = text.chars().filter(|c| !matches!(c, ':' | ' ')).collect();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("a TLS fingerprint is 64 hex digits (SHA-256)".into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// The SHA-256 fingerprint of the certificate a server presents, as 64 lower-case hex
/// digits. The certificate is not checked. Compare it with a value obtained another way
/// before trusting the network. With `via`, the connection goes through that proxy, as the
/// network's own connection does.
pub async fn probe_tls_fingerprint(
    host: &str,
    port: u16,
    via: Option<(&ProxyConfig, Option<&str>)>,
) -> Result<String, String> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCertificate::new(None, false, false, provider)))
        .with_no_client_auth();
    let domain = ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid TLS server name: {e}"))?;
    let timeout = Duration::from_secs(10);
    let tcp = match via {
        Some((proxy_cfg, proxy_pass)) => {
            tokio::time::timeout(timeout, proxy::connect_via(proxy_cfg, proxy_pass, host, port)).await
        }
        None => tokio::time::timeout(timeout, TcpStream::connect((host, port))).await,
    }
    .map_err(|_| "the connection timed out".to_string())?
    .map_err(|e| e.to_string())?;
    let tls = tokio::time::timeout(timeout, TlsConnector::from(Arc::new(config)).connect(domain, tcp))
        .await
        .map_err(|_| "the TLS handshake timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let (_, connection) = tls.get_ref();
    let cert = connection
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| "the server sent no certificate".to_string())?;
    let digest = <sha2::Sha256 as sha2::Digest>::digest(cert.as_ref());
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// The SHA-256 of the certificate the peer of a TLS stream presented, as lower-case hex.
fn peer_fingerprint<S>(tls: &tokio_rustls::client::TlsStream<S>) -> Option<String> {
    let (_, connection) = tls.get_ref();
    let cert = connection.peer_certificates()?.first()?;
    let digest = <sha2::Sha256 as sha2::Digest>::digest(cert.as_ref());
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// The trust anchors used to check server certificates.
fn tls_roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// The bytes for one IRC line. A CR, LF or NUL inside the line would send a second
/// command, so such a line is refused. A CRLF at the end is accepted and not doubled.
/// The text of a WHOIS answer in the form the address book reads: `nick [user@host] name`,
/// `nick idle: N seconds`, `nick is on channel: ...`. Params: our nick first, then the target.
fn whois_line(msg: &IrcMessage) -> Option<String> {
    let p = |i: usize| msg.params.get(i).map(String::as_str).unwrap_or("");
    let nick = p(1);
    if nick.is_empty() {
        return None;
    }
    Some(match msg.command.as_str() {
        "301" => format!("{nick} is away: {}", p(2)),
        "311" => format!("{nick} [{}@{}] {}", p(2), p(3), p(5)),
        "312" => format!("{nick} using server {}", p(2)),
        "313" => format!("{nick} is an IRC operator"),
        "317" => format!("{nick} idle: {} seconds", p(2)),
        "319" => format!("{nick} is on channel: {}", p(2)),
        "330" => format!("{nick} is logged in as {}", p(2)),
        _ => return None,
    })
}

pub(crate) fn wire_line(line: &str) -> Result<String, String> {
    let body = line.strip_suffix("\r\n").unwrap_or(line);
    if body.contains(['\r', '\n', '\0']) {
        return Err("line contains CR, LF or NUL and would send more than one command".into());
    }
    Ok(format!("{body}\r\n"))
}

/// Account and password for SASL PLAIN. The password is not shown in debug output.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SaslAccount {
    pub account: String,
    pub password: String,
}

impl std::fmt::Debug for SaslAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaslAccount")
            .field("account", &self.account)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectConfig {
    pub network: String,
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub nick: String,
    pub username: String,
    pub realname: String,
    pub password: Option<String>,
    /// The nick to try when the first one is taken before registration (433), before the numbered
    /// alternates. Unset, only the numbered alternates are used.
    #[serde(default)]
    pub altnick: Option<String>,
    /// Account and password for SASL PLAIN, used when the server offers SASL. Unset, SASL is not used.
    #[serde(default)]
    pub sasl: Option<SaslAccount>,
    /// SHA-256 fingerprint of the server's certificate, for a network whose certificate is
    /// self-signed. When set, only that certificate is accepted (its chain is not checked);
    /// the handshake signatures still are. Unset, certificates must chain to the web PKI roots.
    #[serde(default)]
    pub tls_fingerprint: Option<String>,
    /// Accept a certificate that is not trusted (typically self-signed) when no fingerprint is
    /// pinned. The handshake signatures are still checked, but the identity is not: the host name
    /// and the issuer are not verified, so anyone who can reach the server can present a
    /// certificate. The fingerprint is reported in the status window after the connect.
    #[serde(default)]
    pub allow_self_signed: bool,
}

/// Looks up a FiSH key by "network:target", trying the UI/config network id first
/// (what the key manager saves under), then the server-advertised NETWORK name
///, then the bare target.
fn find_fish_key<'a>(
    keys: &'a std::collections::HashMap<String, String>,
    config_network: &str,
    isupport_network: &str,
    target: &str,
) -> Option<&'a String> {
    let target = target.to_lowercase();
    [config_network, isupport_network]
        .iter()
        .find_map(|net| keys.get(&format!("{}:{}", net, target).to_lowercase()))
        .or_else(|| keys.get(&target))
}

/// Aborts a spawned task when dropped, so a task cannot outlive the scope that owns it.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct IrcClient {
    config: ConnectConfig,
    /// Moved into the connection task by `connect`. The client keeps no sender of its own, so
    /// the event channel closes as soon as that task ends and the receiving loop can finish.
    event_tx: Option<mpsc::Sender<IrcEvent>>,
    /// Handle of the connection task (connect, TLS, read loop), set by `connect`.
    task: Option<tokio::task::AbortHandle>,
    writer_tx: Option<mpsc::Sender<String>>,
    fish_keys: Arc<Mutex<std::collections::HashMap<String, String>>>,
    fish_network: Arc<Mutex<String>>,
    /// Our nick as the server knows it: starts as the configured nick, updated from 001 and NICK.
    own_nick: Arc<std::sync::Mutex<String>>,
    /// Proxy and its password. Memory only; `connect` takes it, so the password
    /// is dropped once the TCP connection is made.
    proxy: Option<(ProxyConfig, Option<String>)>,
    debug_log: Option<Arc<DebugLog>>,
    connect_timeout: Duration,
    /// Set by `shutdown`. The app's event loop reads it instead of locking the client map per line.
    shut_down: std::sync::atomic::AtomicBool,
    /// Set by `retire` when the network is closed. Its connection is no longer current from that
    /// moment, while the QUIT sent during the close can still go out.
    retired: std::sync::atomic::AtomicBool,
}

/// Limit for DNS, TCP and TLS together.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// A failed connect: the step it stopped at and why.
struct ConnectFail {
    step: &'static str,
    reason: String,
}

enum Established {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// One entry of a channel list from 346 (invites), 348 (excepts) or 367 (bans):
/// `<client> <channel> <mask> [setter [time]]`.
fn channel_list_event(network: &str, params: &[String], list: char) -> Option<IrcEvent> {
    if params.len() < 3 {
        return None;
    }
    let set_by = params.get(3).cloned().unwrap_or_default();
    let set_at = params.get(4).and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
    Some(IrcEvent::ChannelList {
        network: network.to_string(),
        channel: params[1].clone(),
        list,
        mask: params[2].clone(),
        set_by,
        set_at,
    })
}

/// DNS, TCP (or proxy) and TLS. `step` always names the phase in progress, so the caller knows
/// where this future was dropped on a timeout. Each phase is logged to `trace`.
async fn establish(
    config: &ConnectConfig,
    proxy: Option<&(ProxyConfig, Option<String>)>,
    trace: &Trace,
    step: &std::sync::Mutex<&'static str>,
) -> Result<Established, ConnectFail> {
    // Registration values go onto their own IRC lines, so a line break in one would send a second
    // command. A fingerprint is a TLS pin: without TLS it would be ignored. Both are refused first.
    let registration = [
        config.password.as_deref().unwrap_or(""),
        config.nick.as_str(),
        config.username.as_str(),
        config.realname.as_str(),
    ];
    if registration.iter().any(|v| v.contains(['\r', '\n', '\0'])) {
        return Err(ConnectFail {
            step: "config",
            reason: "the password, nick, user name and real name must not contain line breaks".into(),
        });
    }
    if config.tls_fingerprint.is_some() && !config.use_tls {
        return Err(ConnectFail { step: "config", reason: "a certificate fingerprint needs TLS".into() });
    }
    let set = |name: &'static str| {
        if let Ok(mut s) = step.lock() {
            *s = name;
        }
    };

    let tcp = match proxy {
        Some((proxy_cfg, proxy_pass)) => {
            set("proxy connect");
            match proxy::connect_via(proxy_cfg, proxy_pass.as_deref(), &config.host, config.port).await {
                Ok(s) => {
                    trace.step("tcp connected", "via proxy");
                    s
                }
                Err(e) => {
                    trace.step("tcp failed", &format!("{:?} (via proxy)", e.kind()));
                    return Err(ConnectFail { step: "proxy connect", reason: e.to_string() });
                }
            }
        }
        None => {
            set("dns");
            let addrs: Vec<SocketAddr> =
                match tokio::net::lookup_host((config.host.as_str(), config.port)).await {
                    Ok(it) => it.collect(),
                    Err(e) => {
                        trace.step("dns failed", &format!("{:?}", e.kind()));
                        return Err(ConnectFail { step: "dns", reason: e.to_string() });
                    }
                };
            if addrs.is_empty() {
                trace.step("dns failed", "no addresses");
                return Err(ConnectFail { step: "dns", reason: "no addresses found".into() });
            }
            trace.step("dns resolved", &format!("{} addresses", addrs.len()));
            set("tcp connect");
            let mut last_err = None;
            let mut stream = None;
            for (i, a) in addrs.iter().enumerate() {
                let which = format!(
                    "addr {}/{} {}",
                    i + 1,
                    addrs.len(),
                    if a.is_ipv6() { "ipv6" } else { "ipv4" }
                );
                trace.step("tcp connecting", &which);
                match TcpStream::connect(a).await {
                    Ok(s) => {
                        trace.step("tcp connected", &which);
                        stream = Some(s);
                        break;
                    }
                    Err(e) => {
                        trace.step("tcp failed", &format!("{:?} ({which})", e.kind()));
                        last_err = Some(e);
                    }
                }
            }
            match stream {
                Some(s) => s,
                None => {
                    let reason = last_err.map(|e| e.to_string()).unwrap_or_default();
                    return Err(ConnectFail { step: "tcp connect", reason });
                }
            }
        }
    };

    if !config.use_tls {
        return Ok(Established::Plain(tcp));
    }

    set("tls handshake");
    // Name the provider. With reqwest's `ring` and rustls' default `aws_lc_rs` both enabled
    // (the app build), `ClientConfig::builder()` panics. The panic ended the connect task
    // without an event, so the UI showed "Connecting" forever.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let versions = match rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
    {
        Ok(v) => v,
        Err(e) => {
            trace.step("tls failed", &format!("config: {e}"));
            return Err(ConnectFail { step: "tls handshake", reason: e.to_string() });
        }
    };
    // Certificates are checked against the built-in web PKI roots, unless the network has a
    // pinned fingerprint for its self-signed certificate.
    let client_config = match config.tls_fingerprint.as_deref() {
        Some(text) => {
            let pin = match parse_fingerprint(text) {
                Ok(pin) => pin,
                Err(reason) => {
                    trace.step("tls failed", &reason);
                    return Err(ConnectFail { step: "tls handshake", reason });
                }
            };
            versions
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedCertificate::new(
                    Some(pin),
                    false,
                    false,
                    provider.clone(),
                )))
                .with_no_client_auth()
        }
        None if config.allow_self_signed => versions
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCertificate::new(
                None,
                true,
                true,
                provider.clone(),
            )))
            .with_no_client_auth(),
        None => versions
            .with_root_certificates(tls_roots())
            .with_no_client_auth(),
    };
    let connector = TlsConnector::from(Arc::new(client_config));
    let domain = match ServerName::try_from(config.host.as_str()) {
        Ok(d) => d.to_owned(),
        Err(e) => {
            trace.step("tls failed", &format!("invalid server name: {e}"));
            return Err(ConnectFail {
                step: "tls handshake",
                reason: format!("invalid TLS server name: {e}"),
            });
        }
    };
    match connector.connect(domain, tcp).await {
        Ok(s) => {
            trace.step("tls handshake ok", "");
            Ok(Established::Tls(Box::new(s)))
        }
        Err(e) => {
            trace.step("tls failed", &format!("{:?}: {}", e.kind(), e));
            Err(ConnectFail { step: "tls handshake", reason: e.to_string() })
        }
    }
}

impl IrcClient {
    pub fn new(config: ConnectConfig, event_tx: mpsc::Sender<IrcEvent>) -> Self {
        let fish_network = config.network.to_lowercase();
        let own_nick = Arc::new(std::sync::Mutex::new(config.nick.clone()));
        Self {
            config,
            event_tx: Some(event_tx),
            task: None,
            writer_tx: None,
            fish_keys: Arc::new(Mutex::new(std::collections::HashMap::new())),
            fish_network: Arc::new(Mutex::new(fish_network)),
            own_nick,
            proxy: None,
            debug_log: None,
            connect_timeout: CONNECT_TIMEOUT,
            shut_down: std::sync::atomic::AtomicBool::new(false),
            retired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Write the connect steps of the next `connect` to this log. The log has no secret fields.
    pub fn set_debug_log(&mut self, log: Option<Arc<DebugLog>>) {
        self.debug_log = log;
    }

    /// Replaces the 20 second limit for DNS, TCP and TLS together.
    pub fn set_connect_timeout(&mut self, timeout: Duration) {
        self.connect_timeout = timeout;
    }

    /// Route the next `connect` through a proxy. `None` means a direct connection.
    /// A separate setter (not a `ConnectConfig` field) keeps existing
    /// `ConnectConfig` struct literals compiling and the password out of any
    /// serialized or cloned config.
    pub fn set_proxy(&mut self, proxy: Option<ProxyConfig>, password: Option<String>) {
        self.proxy = proxy.map(|p| (p, password));
    }

    pub async fn set_fish_key(&self, target: &str, key: &str) {
        let mut keys = self.fish_keys.lock().await;
        keys.insert(target.to_lowercase(), key.to_string());
    }

    pub async fn get_fish_keys(&self) -> std::collections::HashMap<String, String> {
        self.fish_keys.lock().await.clone()
    }

    pub async fn get_fish_key(&self, target: &str) -> Option<String> {
        let network = self.fish_network.lock().await.clone();
        let keys = self.fish_keys.lock().await;
        find_fish_key(&keys, &self.config.network, &network, target).cloned()
    }

    pub async fn remove_fish_key(&self, target: &str) {
        self.fish_keys.lock().await.remove(&target.to_lowercase());
    }

    /// Replaces every FiSH key of this connection, e.g. with what blow.ini holds now.
    pub async fn replace_fish_keys(&self, keys: std::collections::HashMap<String, String>) {
        *self.fish_keys.lock().await = keys;
    }

    /// Stops the connection task and its writer task and closes the socket. Safe to call more
    /// than once, and after the connection has already ended.
    pub fn shutdown(&self) {
        self.shut_down.store(true, std::sync::atomic::Ordering::Release);
        if let Some(task) = &self.task {
            task.abort();
        }
    }

    /// Marks the connection as no longer current. The socket stays open until `shutdown`.
    pub fn retire(&self) {
        self.retired.store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether `retire` has been called.
    pub fn is_retired(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Whether `shutdown` has been called.
    pub fn is_shut_down(&self) -> bool {
        self.shut_down.load(std::sync::atomic::Ordering::Acquire)
    }

    pub async fn connect(&mut self) -> Result<(), String> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let network = self.config.network.clone();
        let event_tx = self
            .event_tx
            .take()
            .ok_or_else(|| "Connection was already started".to_string())?;
        let fish_keys = Arc::clone(&self.fish_keys);
        let fish_network = Arc::clone(&self.fish_network);
        let own_nick = Arc::clone(&self.own_nick);

        // Lines waiting for the socket. A full queue means the server has stopped reading: a send
        // fails instead of the queue growing without limit.
        let (writer_tx, writer_rx) = mpsc::channel::<String>(WRITER_QUEUE_LINES);
        self.writer_tx = Some(writer_tx.clone());

        let config_clone = self.config.clone();
        let proxy = self.proxy.take();
        let limit = self.connect_timeout;
        let trace = Trace::new(self.debug_log.clone(), &network);

        let task = tokio::spawn(async move {
            trace.step(
                "start",
                &format!(
                    "host {} port {} tls {} proxy {}",
                    config_clone.host,
                    config_clone.port,
                    yes_no(config_clone.use_tls),
                    yes_no(proxy.is_some())
                ),
            );
            let step = std::sync::Mutex::new("dns");
            let outcome = tokio::time::timeout(
                limit,
                establish(&config_clone, proxy.as_ref(), &trace, &step),
            )
            .await;
            // The proxy password is not needed past this point.
            drop(proxy);
            let established = match outcome {
                Ok(Ok(e)) => e,
                Ok(Err(fail)) => {
                    let _ = event_tx.try_send(IrcEvent::Error {
                        network: network.clone(),
                        message: format!(
                            "Connection failed to {} at {}: {}",
                            addr, fail.step, fail.reason
                        ),
                    });
                    return;
                }
                Err(_) => {
                    let at = step.lock().map(|s| *s).unwrap_or("unknown step");
                    let secs = limit.as_secs_f32();
                    trace.step(&format!("timeout at {at}"), &format!("after {secs}s"));
                    let _ = event_tx.try_send(IrcEvent::Error {
                        network: network.clone(),
                        message: format!(
                            "Connection failed to {} at {}: timed out after {}s",
                            addr, at, secs
                        ),
                    });
                    return;
                }
            };

            match established {
                Established::Tls(tls_stream) => {
                    // Accepted without a pin: show the fingerprint, so the user can pin it.
                    if config_clone.tls_fingerprint.is_none() && config_clone.allow_self_signed {
                        if let Some(fingerprint) = peer_fingerprint(&tls_stream) {
                            let _ = event_tx.try_send(IrcEvent::Error {
                                network: network.clone(),
                                message: format!(
                                    "Certificate checked (self-signed allowed, not chained to a public CA). SHA-256: {fingerprint}. Pin it in the network's TLS fingerprint to refuse any other certificate."
                                ),
                            });
                        }
                    }
                    let (reader, writer) = tokio::io::split(*tls_stream);
                    run_client_loops(
                        reader,
                        writer,
                        writer_rx,
                        writer_tx,
                        config_clone,
                        event_tx,
                        fish_keys,
                        fish_network,
                        own_nick,
                        trace,
                    )
                    .await;
                }
                Established::Plain(tcp_stream) => {
                    let (reader, writer) = tokio::io::split(tcp_stream);
                    run_client_loops(
                        reader,
                        writer,
                        writer_rx,
                        writer_tx,
                        config_clone,
                        event_tx,
                        fish_keys,
                        fish_network,
                        own_nick,
                        trace,
                    )
                    .await;
                }
            }
        });
        self.task = Some(task.abort_handle());

        Ok(())
    }

    pub fn nick(&self) -> &str {
        &self.config.nick
    }

    /// Our nick as the server knows it: the configured nick until 001 (or a later NICK)
    /// says otherwise.
    pub fn current_nick(&self) -> String {
        self.own_nick
            .lock()
            .map(|n| n.clone())
            .unwrap_or_else(|_| self.config.nick.clone())
    }

    /// Sends `text` to `target`, split into as many lines as needed (FiSH-encrypted when a key
    /// exists). Returns the original text for the local echo and whether it was encrypted.
    pub async fn send_privmsg(&self, target: &str, text: &str) -> Result<(String, bool), String> {
        self.send_text("PRIVMSG", target, text).await
    }

    /// Sends `text` as a NOTICE, with the same splitting and FiSH rules as `send_privmsg`.
    pub async fn send_notice(&self, target: &str, text: &str) -> Result<(String, bool), String> {
        self.send_text("NOTICE", target, text).await
    }

    async fn send_text(&self, verb: &str, target: &str, text: &str) -> Result<(String, bool), String> {
        let network = self.fish_network.lock().await.clone();
        let keys = self.fish_keys.lock().await;
        let key_opt = find_fish_key(&keys, &self.config.network, &network, target);
        let is_enc = key_opt.is_some();
        let lines = build_message_lines(verb, target, text, key_opt.map(String::as_str))?;
        drop(keys);

        for line in &lines {
            self.send_raw(line)?;
        }
        Ok((text.to_string(), is_enc))
    }

    pub fn send_raw(&self, line: &str) -> Result<(), String> {
        let wire = wire_line(line)?;
        if let Some(ref tx) = self.writer_tx {
            tx.try_send(wire).map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    "Send queue is full, the server is not reading".to_string()
                }
                mpsc::error::TrySendError::Closed(_) => {
                    "Failed to send line: connection closed".to_string()
                }
            })
        } else {
            Err("Not connected".into())
        }
    }
}

/// Lines that may wait for the socket before a send fails.
const WRITER_QUEUE_LINES: usize = 1024;
/// Longest line we build (without CRLF): CRLF included it stays under 510 bytes.
const MAX_LINE_BODY: usize = 509 - 2;
/// Plaintext bytes per unencrypted PRIVMSG line.
const PLAIN_CHUNK: usize = 400;
/// Upper bound for plaintext bytes per encrypted line, so relayed lines (with the sender's
/// prefix added by the server) still fit.
const FISH_CHUNK: usize = 200;
/// Longest raw line read from the server; the rest of a longer line is dropped.
const MAX_READ_LINE: usize = 8 * 1024;

/// Wire length of the FiSH encoding of `n` plaintext bytes.
fn fish_encoded_len(cbc: bool, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let blocks = (n + 7) / 8;
    if cbc {
        // "+OK *" + base64(IV + ciphertext) with padding
        5 + 4 * ((8 + blocks * 8 + 2) / 3)
    } else {
        // "+OK " + 12 characters per 8-byte block
        4 + 12 * blocks
    }
}

/// The text cut to at most `max` UTF-16 code units, at a character boundary.
fn truncate_utf16(text: &str, max: usize) -> &str {
    let mut units = 0usize;
    for (i, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &text[..i];
        }
    }
    text
}

/// The segments of a message: CRLF, a lone CR or LF, and U+FFFF end a segment, and the spaces after a
/// separator are skipped. An empty segment before a separator is kept; a trailing separator adds none
///.
fn segments_of(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        let found = rest.char_indices().find(|&(_, c)| c == '\r' || c == '\n' || c == '\u{FFFF}');
        let Some((i, c)) = found else {
            out.push(rest);
            break;
        };
        out.push(&rest[..i]);
        let mut next = i + c.len_utf8();
        if c == '\r' && rest[next..].starts_with('\n') {
            next += 1;
        }
        rest = rest[next..].trim_start_matches(' ');
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// Builds the complete PRIVMSG lines (no CRLF) for `text`: one per chunk, each encrypted when
/// `key` is given. Every line stays within `MAX_LINE_BODY` bytes. CR and LF in `text` end a
/// line, so text can never smuggle in extra commands.
#[cfg(test)]
fn build_privmsg_lines(target: &str, text: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    build_message_lines("PRIVMSG", target, text, key)
}

/// The lines for one PRIVMSG or NOTICE: `verb` is the command, and each line is encrypted when a key exists.
fn build_message_lines(verb: &str, target: &str, text: &str, key: Option<&str>) -> Result<Vec<String>, String> {
    // The text is cut to what the handler allows for the target: 0x2800 UTF-16 code units less the target's
    // length. The cut is not wrapped; the transport split below
    // only keeps each line within 512 bytes.
    let cap = 0x2800usize.saturating_sub(target.encode_utf16().count());
    if cap == 0 {
        return Err("Target name is too long to send a message".to_string());
    }
    let text = truncate_utf16(text, cap);
    let head = format!("{verb} {target} :");
    let budget = MAX_LINE_BODY.saturating_sub(head.len());
    let cipher = key.map(FishCipher::new);
    let limit = match &cipher {
        Some(c) => (1..=FISH_CHUNK)
            .rev()
            .find(|n| fish_encoded_len(c.is_cbc(), *n) <= budget),
        None => Some(PLAIN_CHUNK.min(budget)).filter(|n| *n > 0),
    }
    .filter(|n| *n >= 4)
    .ok_or_else(|| "Target name is too long to send a message".to_string())?;

    let mut chunks: Vec<String> = Vec::new();
    for segment in segments_of(text) {
        if segment.is_empty() {
            chunks.push(String::new());
        } else {
            chunks.extend(split_text(segment, limit));
        }
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }

    let mut lines = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let wire = match &cipher {
            Some(c) => c
                .encrypt(chunk)
                .map_err(|_| "FiSH encryption failed; message was not sent".to_string())?,
            None => chunk.clone(),
        };
        lines.push(format!("{}{}", head, wire));
    }
    Ok(lines)
}

/// Reads one line from `reader` into `buf` (reused between calls) and decodes it as UTF-8 with a
/// Windows-1252 fallback. At most `MAX_READ_LINE` bytes of a line are kept; the rest of a longer
/// line is read and dropped. Returns `Ok(None)` at end of stream.
async fn read_line_capped<R>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    buf.clear();
    let mut seen_any = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            break;
        }
        seen_any = true;
        let (used, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let room = MAX_READ_LINE.saturating_sub(buf.len());
        buf.extend_from_slice(&available[..used.min(room)]);
        reader.consume(used);
        if done {
            break;
        }
    }
    if !seen_any {
        return Ok(None);
    }
    while matches!(buf.last(), Some(b'\n' | b'\r')) {
        buf.pop();
    }
    Ok(Some(decode_lossy(buf)))
}

impl Drop for IrcClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn run_client_loops<R, W>(
    reader: R,
    mut writer: W,
    mut writer_rx: mpsc::Receiver<String>,
    writer_tx: mpsc::Sender<String>,
    config: ConnectConfig,
    event_tx: mpsc::Sender<IrcEvent>,
    fish_keys: Arc<Mutex<std::collections::HashMap<String, String>>>,
    fish_network: Arc<Mutex<String>>,
    own_nick: Arc<std::sync::Mutex<String>>,
    trace: Trace,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let network = config.network.clone();

    // Writer task. The guard aborts it when this function returns or is aborted, so it does not
    // wait on a channel that the client object may keep open, and the socket is released.
    let _writer_guard = AbortOnDrop(tokio::spawn(async move {
        while let Some(line) = writer_rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            let _ = writer.flush().await;
        }
    }));

    // Initial registration
    if let Some(ref pass) = config.password {
        let _ = writer_tx.try_send(format!("PASS {}\r\n", pass));
    }
    // Capability negotiation starts before NICK and USER: the server holds registration until CAP END.
    let _ = writer_tx.try_send("CAP LS 302\r\n".to_string());
    let _ = writer_tx.try_send(format!("NICK {}\r\n", config.nick));
    let _ = writer_tx.try_send(format!(
        "USER {} 0 * :{}\r\n",
        config.username, config.realname
    ));
    trace.step(
        "registration sent",
        if config.password.is_some() { "PASS NICK USER" } else { "NICK USER" },
    );

    let mut reader = BufReader::new(reader);
    let mut line_buf: Vec<u8> = Vec::with_capacity(1024);
    // CHANTYPES from 005; decides whether a PRIVMSG target is a channel.
    let mut chantypes: Option<String> = None;
    // Nick refusals (433) before registration: how many alternates were tried, and whether 001 came.
    let mut nick_attempt: u32 = 0;
    let mut registered = false;
    // IRCv3 capability negotiation and SASL: what the server offers, what is enabled, and progress.
    let mut cap_offered: std::collections::HashSet<String> = Default::default();
    let mut cap_enabled: std::collections::HashSet<String> = Default::default();
    let mut cap_ended = false;
    let mut sasl_active = false;

    let mut end_reason = "closed by remote".to_string();
    let mut first_line = true;
    loop {
        let line = match read_line_capped(&mut reader, &mut line_buf).await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(e) => {
                end_reason = format!("read error {:?}", e.kind());
                break;
            }
        };
        if first_line {
            first_line = false;
            trace.step("first line received", "");
        }

        if let Some(msg) = IrcMessage::parse(&line) {
            // The state reads the nick (001) and the server's capabilities (005) from these two only.
            if msg.command == "001" || msg.command == "005" {
                let _ = event_tx.try_send(IrcEvent::Raw {
                    network: network.clone(),
                    raw: line.clone(),
                });
            }
            match msg.command.as_str() {
                "PING" => {
                    let daemon = msg.params.first().cloned().unwrap_or_default();
                    // try_send: waiting for room here could stall the reader behind a full queue.
                    if let Ok(line) = wire_line(&format!("PONG :{daemon}")) {
                        let _ = writer_tx.try_send(line);
                    }
                }
                "CAP" => {
                    // IRCv3 capability negotiation. Params: our nick (or "*"), the subcommand, an
                    // optional "*" when more lines follow, and the list.
                    let sub = msg.params.get(1).map(|s| s.to_ascii_uppercase()).unwrap_or_default();
                    let list = msg.params.last().cloned().unwrap_or_default();
                    let names: Vec<String> = list
                        .split_whitespace()
                        .map(|c| c.split('=').next().unwrap_or("").to_string())
                        .collect();
                    let want_sasl = config.sasl.is_some();
                    match sub.as_str() {
                        "LS" | "NEW" => {
                            cap_offered.extend(names.iter().cloned());
                            let more = sub == "LS" && msg.params.len() >= 4 && msg.params[2] == "*";
                            if !more {
                                let request = capabilities_to_request(&cap_offered, &cap_enabled, want_sasl);
                                if !request.is_empty() {
                                    if let Ok(line) = wire_line(&format!("CAP REQ :{}", request.join(" "))) {
                                        let _ = writer_tx.try_send(line);
                                    }
                                } else if !cap_ended {
                                    if let Ok(line) = wire_line("CAP END") {
                                        let _ = writer_tx.try_send(line);
                                    }
                                    cap_ended = true;
                                }
                            }
                        }
                        "ACK" => {
                            cap_enabled.extend(names.iter().cloned());
                            if !registered && cap_enabled.contains("sasl") && config.sasl.is_some() {
                                sasl_active = true;
                                if let Ok(line) = wire_line("AUTHENTICATE PLAIN") {
                                    let _ = writer_tx.try_send(line);
                                }
                            } else if !cap_ended {
                                if let Ok(line) = wire_line("CAP END") {
                                    let _ = writer_tx.try_send(line);
                                }
                                cap_ended = true;
                            }
                        }
                        "NAK" => {
                            if !cap_ended {
                                if let Ok(line) = wire_line("CAP END") {
                                    let _ = writer_tx.try_send(line);
                                }
                                cap_ended = true;
                            }
                        }
                        "DEL" => {
                            for name in &names {
                                cap_enabled.remove(name);
                                cap_offered.remove(name);
                            }
                        }
                        _ => {}
                    }
                }
                "AUTHENTICATE" => {
                    // SASL PLAIN: the server asks for the payload with "+", and the client sends it.
                    if sasl_active && msg.params.first().map(String::as_str) == Some("+") {
                        if let Some(acc) = config.sasl.as_ref() {
                            let payload = base64_encode(format!("\0{}\0{}", acc.account, acc.password).as_bytes());
                            for line in sasl_chunks(&payload) {
                                if let Ok(wire) = wire_line(&format!("AUTHENTICATE {line}")) {
                                    let _ = writer_tx.try_send(wire);
                                }
                            }
                        }
                    }
                }
                "903" => {
                    // RPL_SASLSUCCESS: registration goes on.
                    sasl_active = false;
                    if !cap_ended {
                        if let Ok(line) = wire_line("CAP END") {
                            let _ = writer_tx.try_send(line);
                        }
                        cap_ended = true;
                    }
                }
                "904" | "905" | "906" | "907" => {
                    // SASL failed, was aborted, or is already done. Registration goes on without it.
                    sasl_active = false;
                    let text = format!("SASL: {}", error_text(&msg));
                    let _ = event_tx.try_send(status_notice(&network, text));
                    if !cap_ended {
                        if let Ok(line) = wire_line("CAP END") {
                            let _ = writer_tx.try_send(line);
                        }
                        cap_ended = true;
                    }
                }
                "352" => {
                    // RPL_WHOREPLY: one line per user of a WHO request, in the status window.
                    let text = who_line(&msg).unwrap_or_else(|| error_text(&msg));
                    let _ = event_tx.try_send(status_notice(&network, text));
                }
                "354" | "315" | "730" | "731" | "732" | "733" | "734" => {
                    // WHOX reply, end of a WHO list, and MONITOR replies: shown as the server sent them.
                    let _ = event_tx.try_send(status_notice(&network, error_text(&msg)));
                }
                "AWAY" | "ACCOUNT" | "CHGHOST" | "SETNAME" | "INVITE" => {
                    // IRCv3 notifications about other users (away-notify, account-notify, chghost,
                    // setname, invite-notify), shown in the status window.
                    let who = msg.prefix.as_ref().map(|p| p.nick.clone()).unwrap_or_else(|| "*".into());
                    let text = capability_notice(&msg.command, &who, &msg.params);
                    let _ = event_tx.try_send(status_notice(&network, text));
                }
                "401" | "403" | "404" | "442" | "471" | "472" | "473" | "474" | "475" | "476" | "477" | "478" | "479" | "480" | "481" | "482" => {
                    // Error replies about a nick, a channel or a command, shown in the status window.
                    let _ = event_tx.try_send(status_notice(&network, error_text(&msg)));
                }
                "433" => {
                    // ERR_NICKNAMEINUSE. Before registration the refused nick is replaced by the next
                    // alternate, so the connection can finish. After it, the error is shown in status.
                    let refused = msg.params.last().cloned().unwrap_or_else(|| "Nickname is already in use".into());
                    if !registered && nick_attempt < MAX_NICK_ATTEMPTS {
                        let candidate = nick_candidate(&config.nick, config.altnick.as_deref(), nick_attempt);
                        nick_attempt += 1;
                        if let Ok(line) = wire_line(&format!("NICK {candidate}")) {
                            let _ = writer_tx.try_send(line);
                        }
                    } else {
                        let text = if registered {
                            refused
                        } else {
                            "All alternative nicknames are in use. Choose one with /nick.".to_string()
                        };
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                        let _ = event_tx.try_send(IrcEvent::Message {
                            sender_address: None,
                            network: network.clone(),
                            target: "status".into(),
                            sender: "*".into(),
                            spans: Vec::new(),
                            text,
                            is_notice: true,
                            is_action: false,
                            is_encrypted: false,
                            timestamp: now,
                            sender_host: None,
                        });
                    }
                }
                "001" => {
                    // RPL_WELCOME
                    registered = true;
                    trace.step("001 received", "");
                    // The first parameter is the nick the server gave us.
                    if let Some(nick) = msg.params.first().filter(|n| !n.is_empty() && *n != "*") {
                        if let Ok(mut own) = own_nick.lock() {
                            *own = nick.clone();
                        }
                    }
                    let _ = event_tx.try_send(IrcEvent::Connected {
                        network: network.clone(),
                        server: config.host.clone(),
                    });
                    if let Some(welcome_text) = msg.params.get(1) {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                        let _ = event_tx.try_send(IrcEvent::Message {
                            sender_address: None,
                            network: network.clone(),
                            target: "status".into(),
                            sender: "*".into(),
                            text: welcome_text.clone(),
                            spans: Vec::new(),
                            is_notice: true,
                            is_action: false,
                            is_encrypted: false,
                            timestamp: now,
                            sender_host: None,
                        });
                    }
                }
                "301" | "311" | "312" | "313" | "317" | "319" | "330" => {
                    // WHOIS answers go to the status window as server notices (sender "*"); the address
                    // book reads them there.
                    if let Some(text) = whois_line(&msg) {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                        let _ = event_tx.try_send(IrcEvent::Message {
                            sender_address: None,
                            network: network.clone(),
                            target: "status".into(),
                            sender: "*".into(),
                            spans: Vec::new(),
                            text,
                            is_notice: true,
                            is_action: false,
                            is_encrypted: false,
                            timestamp: now,
                            sender_host: None,
                        });
                    }
                }
                "005" => {
                    if let Some(types) = msg.params.iter().skip(1).find_map(|param| {
                        let (key, value) = param.split_once('=')?;
                        (key.eq_ignore_ascii_case("CHANTYPES") && !value.is_empty() && value.len() <= 32)
                            .then(|| value.to_string())
                    }) {
                        chantypes = Some(types);
                    }
                    if let Some(name) = msg.params.iter().skip(1).find_map(|param| {
                        let (key, value) = param.split_once('=')?;
                        (key.eq_ignore_ascii_case("NETWORK") && !value.is_empty())
                            .then_some(value.to_lowercase())
                    }) {
                        *fish_network.lock().await = name;
                    }
                    let text = msg.params.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                    let _ = event_tx.try_send(IrcEvent::Message {
                            sender_address: None,
                        network: network.clone(),
                        target: "status".into(),
                        sender: "*".into(),
                        text: text.clone(),
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        timestamp: now,
                        sender_host: None,
                    });
                }
                "002" | "003" | "004" | "251" | "252" | "253" | "254" | "255" | "265" | "266" | "372" | "375" | "376" => {
                    let text = msg.params.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                    let _ = event_tx.try_send(IrcEvent::Message {
                            sender_address: None,
                        network: network.clone(),
                        target: "status".into(),
                        sender: "*".into(),
                        text: text.clone(),
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        timestamp: now,
                        sender_host: None,
                    });
                }
                cmd if cmd.starts_with('4') || cmd.starts_with('5') => {
                    let err_text = msg.params.join(" ");
                    let _ = event_tx.try_send(IrcEvent::Error {
                        network: network.clone(),
                        message: err_text,
                    });
                }
                "PRIVMSG" | "NOTICE" => {
                    if let Some(target) = msg.params.first() {
                        let text = msg.params.get(1).cloned().unwrap_or_default();
                        let sender = msg
                            .prefix
                            .as_ref()
                            .map(|p| p.nick.clone())
                            .unwrap_or_default();
                        let sender_host = msg.prefix.as_ref().and_then(|p| p.host.clone());
                        let sender_address = msg.prefix.as_ref().and_then(address_of);
                        let is_notice = msg.command == "NOTICE";

                        let mut is_action = false;
                        let mut display_text = text.clone();

                        // Check FiSH encryption
                        let mut is_encrypted = false;
                        let trimmed_wire = display_text.trim_start();
                        if trimmed_wire.starts_with("+OK") || trimmed_wire.starts_with("mcps ") {
                            // Only a line that may be encrypted needs the keys; plain text takes no lock.
                            let keys = fish_keys.lock().await;
                            let own = own_nick.lock().map(|n| n.clone()).unwrap_or_default();
                            let key_target = if is_channel_name_with(target, chantypes.as_deref()) {
                                target.to_lowercase()
                            } else if sender.eq_ignore_ascii_case(&own) {
                                // Our own private message, echoed back: its key is the one for the
                                // person we wrote to, not for ourselves.
                                target.to_lowercase()
                            } else {
                                sender.to_lowercase()
                            };
                            let active_fish_network = fish_network.lock().await.clone();
                            let key_opt = find_fish_key(&keys, &config.network, &active_fish_network, &key_target);
                            if let Some(k) = key_opt {
                                let cipher = FishCipher::new(k);
                                if let Ok(dec) = cipher.decrypt(trimmed_wire) {
                                    display_text = dec;
                                    is_encrypted = true;
                                }
                            }
                        }

                        // CTCP ACTION, checked after decryption: an encrypted action has its frame inside the cipher text.
                        if display_text.starts_with("\x01ACTION ") && display_text.ends_with('\x01') {
                            is_action = true;
                            display_text = display_text[8..display_text.len() - 1].to_string();
                        }

                        let spans = Vec::new();

                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        // server-time: the server's time of the message when it sent one.
                        let now = msg.tags.get("time").and_then(|t| parse_server_time(t)).unwrap_or(now);
                        // echo-message: our own line comes back from the server. It is already in the
                        // chat from the send, so it is not added a second time.
                        let own_echo = {
                            let own = own_nick.lock().map(|n| n.clone()).unwrap_or_default();
                            is_own_echo(cap_enabled.contains("echo-message"), &sender, &own)
                        };
                        if !own_echo {
                            let _ = event_tx.try_send(IrcEvent::Message {
                                network: network.clone(),
                                target: target.clone(),
                                sender,
                                text: display_text,
                                spans,
                                is_notice,
                                is_action,
                                is_encrypted,
                                timestamp: now,
                                sender_host,
                                sender_address,
                            });
                        }
                    }
                }
                "JOIN" => {
                    if let Some(chan) = msg.params.first() {
                        let host = msg.prefix.as_ref().and_then(|p| p.host.clone());
                        let address = msg.prefix.as_ref().and_then(address_of);
                        let nick = msg.prefix.map(|p| p.nick).unwrap_or_default();
                        let _ = event_tx.try_send(IrcEvent::Join {
                            network: network.clone(),
                            channel: chan.clone(),
                            nick,
                            host,
                            address,
                        });
                    }
                }
                "KICK" => {
                    // KICK <channel> <nick> [reason], sent by the operator in the prefix.
                    if let (Some(chan), Some(victim)) = (msg.params.first(), msg.params.get(1)) {
                        let by = msg.prefix.as_ref().map(|p| p.nick.clone()).unwrap_or_default();
                        let _ = event_tx.try_send(IrcEvent::Kick {
                            network: network.clone(),
                            channel: chan.clone(),
                            nick: victim.clone(),
                            by,
                            reason: msg.params.get(2).cloned(),
                        });
                    }
                }
                "PART" => {
                    if let Some(chan) = msg.params.first() {
                        let address = msg.prefix.as_ref().and_then(address_of);
                        let nick = msg.prefix.map(|p| p.nick).unwrap_or_default();
                        let message = msg.params.get(1).cloned();
                        let _ = event_tx.try_send(IrcEvent::Part {
                            network: network.clone(),
                            channel: chan.clone(),
                            nick,
                            message,
                            address,
                        });
                    }
                }
                "QUIT" => {
                    let address = msg.prefix.as_ref().and_then(address_of);
                    let nick = msg.prefix.map(|p| p.nick).unwrap_or_default();
                    let message = msg.params.first().cloned();
                    let _ = event_tx.try_send(IrcEvent::Quit {
                        network: network.clone(),
                        nick,
                        message,
                        address,
                    });
                }
                "NICK" => {
                    let host = msg.prefix.as_ref().and_then(|p| p.host.clone());
                    let address = msg.prefix.as_ref().and_then(address_of);
                    let old_nick = msg.prefix.map(|p| p.nick).unwrap_or_default();
                    let new_nick = msg.params.first().cloned().unwrap_or_default();
                    if let Ok(mut own) = own_nick.lock() {
                        if old_nick.eq_ignore_ascii_case(&own) {
                            *own = new_nick.clone();
                        }
                    }
                    let _ = event_tx.try_send(IrcEvent::NickChange {
                        network: network.clone(),
                        old_nick,
                        new_nick,
                        host,
                        address,
                    });
                }
                "TOPIC" | "332" => {
                    // 332 = RPL_TOPIC
                    let (chan, topic) = if msg.command == "332" {
                        (
                            msg.params.get(1).cloned().unwrap_or_default(),
                            msg.params.get(2).cloned().unwrap_or_default(),
                        )
                    } else {
                        (
                            msg.params.first().cloned().unwrap_or_default(),
                            msg.params.get(1).cloned().unwrap_or_default(),
                        )
                    };
                    // 332 has the server as prefix; only a TOPIC message names who set it.
                    let set_by = if msg.command == "332" {
                        None
                    } else {
                        msg.prefix.map(|p| p.nick)
                    };
                    let _ = event_tx.try_send(IrcEvent::Topic {
                        network: network.clone(),
                        channel: chan,
                        topic,
                        set_by,
                    });
                }
                "353" => {
                    // RPL_NAMREPLY: <client> <symbol> <channel> :[prefix]nick1 [prefix]nick2 ...
                    if msg.params.len() >= 4 {
                        let chan = msg.params[2].clone();
                        let names_str = msg.params[3].clone();
                        let names: Vec<String> = names_str
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect();
                        let _ = event_tx.try_send(IrcEvent::Names {
                            network: network.clone(),
                            channel: chan,
                            names,
                        });
                    }
                }
                "324" => {
                    // RPL_CHANNELMODEIS: <client> <channel> <modes> [params...]
                    if msg.params.len() >= 3 {
                        let _ = event_tx.try_send(IrcEvent::ChannelModes {
                            network: network.clone(),
                            channel: msg.params[1].clone(),
                            modes: msg.params[2].clone(),
                            params: msg.params[3..].to_vec(),
                        });
                    }
                }
                // 346/348/367: invite, except and ban list entries.
                // 347/349/368: the matching end of list.
                "346" | "348" | "367" => {
                    let list = match msg.command.as_str() {
                        "346" => 'I',
                        "348" => 'e',
                        _ => 'b',
                    };
                    if let Some(event) = channel_list_event(&network, &msg.params, list) {
                        let _ = event_tx.try_send(event);
                    }
                }
                "347" | "349" | "368" => {
                    let list = match msg.command.as_str() {
                        "347" => 'I',
                        "349" => 'e',
                        _ => 'b',
                    };
                    if msg.params.len() >= 2 {
                        let _ = event_tx.try_send(IrcEvent::ChannelListEnd {
                            network: network.clone(),
                            channel: msg.params[1].clone(),
                            list,
                        });
                    }
                }
                "MODE" => {
                    if msg.params.len() >= 2 {
                        let _ = event_tx.try_send(IrcEvent::Mode {
                            network: network.clone(),
                            target: msg.params[0].clone(),
                            modes: msg.params[1].clone(),
                            params: msg.params[2..].to_vec(),
                            by: msg.prefix.as_ref().map(|p| p.nick.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    trace.step("disconnected", &end_reason);
    let reason = if end_reason == "closed by remote" {
        "Connection closed by remote".to_string()
    } else {
        format!("Connection lost: {end_reason}")
    };
    let _ = event_tx.try_send(IrcEvent::Disconnected { network, reason });
}

#[cfg(test)]
mod tests {
    use super::{
        build_privmsg_lines, fish_encoded_len, find_fish_key, read_line_capped, ConnectConfig,
        IrcClient, FISH_CHUNK, MAX_READ_LINE,
    };
    use crate::text::is_channel_name_with;
    use crate::proxy::ProxyConfig;
    use crate::connection::event::IrcEvent;
    use std::collections::HashMap;
    use std::time::Duration;
    use tokio::sync::mpsc;

    fn local_client(port: u16) -> (IrcClient, mpsc::Receiver<IrcEvent>) {
        let (tx, rx) = mpsc::channel(256);
        let config = ConnectConfig {
            network: "test".into(),
            host: "127.0.0.1".into(),
            port,
            use_tls: false,
            nick: "tester".into(),
            username: "tester".into(),
            realname: "tester".into(),
            password: None,
            altnick: None,
            sasl: None,
            tls_fingerprint: None,
            allow_self_signed: false,
        };
        (IrcClient::new(config, tx), rx)
    }

    #[test]
    fn whois_answers_read_like_the_address_book_expects() {
        let parse = |raw: &str| crate::protocol::IrcMessage::parse(raw).unwrap();
        assert_eq!(
            super::whois_line(&parse(":srv 311 me Alice alice host * :Real Name")).unwrap(),
            "Alice [alice@host] Real Name"
        );
        assert_eq!(
            super::whois_line(&parse(":srv 317 me Alice 30 1700000000 :seconds idle, signon time")).unwrap(),
            "Alice idle: 30 seconds"
        );
        assert!(super::whois_line(&parse(":srv 318 me Alice :End of /WHOIS list")).is_none());
    }

    #[tokio::test]
    async fn a_line_break_in_registration_is_refused_before_connecting() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut client, mut rx) = local_client(listener.local_addr().unwrap().port());
        client.config.nick = "tester\r\nQUIT :bye".into();
        client.connect().await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert!(
            matches!(&event, IrcEvent::Error { message, .. } if message.contains("line breaks")),
            "{event:?}"
        );
        assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err());
    }

    #[tokio::test]
    async fn a_fingerprint_without_tls_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut client, mut rx) = local_client(listener.local_addr().unwrap().port());
        client.config.tls_fingerprint = Some("ab".repeat(32));
        client.connect().await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
        assert!(
            matches!(&event, IrcEvent::Error { message, .. } if message.contains("needs TLS")),
            "{event:?}"
        );
        assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err());
    }

    #[tokio::test]
    async fn shutdown_ends_the_connection_and_closes_the_event_channel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut client, mut rx) = local_client(listener.local_addr().unwrap().port());
        client.connect().await.unwrap();
        let (_socket, _) = listener.accept().await.unwrap();
        client.shutdown();
        let end = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(matches!(end, Ok(None)), "event channel must close after shutdown");
    }

    #[tokio::test]
    async fn failed_connection_closes_the_event_channel() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (mut client, mut rx) = local_client(port);
        client.connect().await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(matches!(first, Ok(Some(IrcEvent::Error { .. }))));
        let end = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
        assert!(matches!(end, Ok(None)), "channel must close once the task has ended");
    }

    fn temp_log_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ferric-client-test-{}-{tag}", std::process::id()))
    }

    async fn first_error(rx: &mut mpsc::Receiver<IrcEvent>) -> String {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(IrcEvent::Error { message, .. })) => message,
            other => panic!("expected an error event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stalled_tls_handshake_times_out_and_names_the_step() {
        // Accepts the TCP connection, never answers the TLS handshake.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _stall = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let dir = temp_log_dir("tls");
        let (mut client, mut rx) = local_client(port);
        client.config.use_tls = true;
        client.set_connect_timeout(Duration::from_millis(300));
        client.set_debug_log(Some(std::sync::Arc::new(crate::debuglog::DebugLog::new(dir.clone()))));
        client.connect().await.unwrap();
        let message = first_error(&mut rx).await;
        assert_eq!(
            message,
            format!("Connection failed to 127.0.0.1:{port} at tls handshake: timed out after 0.3s")
        );
        let log = std::fs::read_to_string(dir.join("connect-debug.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        for want in [
            " test start | host 127.0.0.1 port ",
            "tls yes proxy no",
            " test dns resolved | 1 addresses",
            " test tcp connected | addr 1/1 ipv4",
            " test timeout at tls handshake | after 0.3s",
        ] {
            assert!(log.contains(want), "missing {want:?} in {log:?}");
        }
        assert!(!log.contains("tls handshake ok"));
    }

    #[tokio::test]
    async fn stalled_proxy_times_out_and_log_has_no_secrets() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let _stall = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let dir = temp_log_dir("proxy");
        let (mut client, mut rx) = local_client(6667);
        client.config.password = Some("serverpw-s3cret".into());
        client.set_proxy(
            Some(ProxyConfig {
                kind: crate::proxy::ProxyKind::Socks5,
                host: "127.0.0.1".into(),
                port: proxy_port,
                username: Some("proxyuser".into()),
            }),
            Some("proxypw-s3cret".into()),
        );
        client.set_connect_timeout(Duration::from_millis(300));
        client.set_debug_log(Some(std::sync::Arc::new(crate::debuglog::DebugLog::new(dir.clone()))));
        client.connect().await.unwrap();
        let message = first_error(&mut rx).await;
        assert_eq!(
            message,
            "Connection failed to 127.0.0.1:6667 at proxy connect: timed out after 0.3s"
        );
        let log = std::fs::read_to_string(dir.join("connect-debug.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(log.contains("tls no proxy yes"), "{log:?}");
        assert!(log.contains(" test timeout at proxy connect | after 0.3s"), "{log:?}");
        for secret in ["s3cret", "proxyuser", "serverpw", "proxypw"] {
            assert!(!log.contains(secret), "{secret} leaked into {log:?}");
        }
    }

    #[tokio::test]
    async fn successful_connect_logs_each_step() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = temp_log_dir("ok");
        let (mut client, mut rx) = local_client(listener.local_addr().unwrap().port());
        client.set_debug_log(Some(std::sync::Arc::new(crate::debuglog::DebugLog::new(dir.clone()))));
        client.connect().await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.write_all(b":srv 001 tester :Welcome\r\n").await.unwrap();
        loop {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(IrcEvent::Connected { .. })) => break,
                Ok(Some(_)) => {}
                other => panic!("connection ended early: {other:?}"),
            }
        }
        drop(socket);
        while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            if matches!(ev, IrcEvent::Disconnected { .. }) {
                break;
            }
        }
        let log = std::fs::read_to_string(dir.join("connect-debug.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let steps = [
            "start |", "dns resolved |", "tcp connected |", "registration sent | NICK USER",
            "first line received |", "001 received |", "disconnected |",
        ];
        let mut at = 0;
        for want in steps {
            let found = log[at..].find(want).unwrap_or_else(|| panic!("missing {want:?} in {log:?}"));
            at += found;
        }
    }

    #[tokio::test]
    async fn connect_twice_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut client, _rx) = local_client(listener.local_addr().unwrap().port());
        client.connect().await.unwrap();
        assert!(client.connect().await.is_err());
    }

    #[test]
    fn fish_key_found_under_config_network_when_isupport_name_differs() {
        let mut keys = HashMap::new();
        keys.insert("net-one-user:#chan.one".to_string(), "k1".to_string());
        keys.insert("net-two:#chan.two".to_string(), "k2".to_string());
        let found = find_fish_key(&keys, "net-one-user", "net-two", "#chan.one");
        assert_eq!(found.map(String::as_str), Some("k1"));
        let found = find_fish_key(&keys, "net-one-user", "net-two", "#chan.two");
        assert_eq!(found.map(String::as_str), Some("k2"));
        assert!(find_fish_key(&keys, "net-one-user", "net-two", "#other").is_none());
    }
    #[tokio::test]
    async fn non_utf8_bytes_do_not_end_the_read_loop() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (mut client, mut rx) = local_client(listener.local_addr().unwrap().port());
        client.connect().await.unwrap();
        let (mut socket, _) = listener.accept().await.unwrap();
        socket
            .write_all(b":srv NOTICE tester :caf\xE9 \x80\r\n:srv 001 Assigned :Welcome\r\n")
            .await
            .unwrap();
        let mut raws = Vec::new();
        let mut texts = Vec::new();
        let mut connected = false;
        while !connected {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(IrcEvent::Raw { raw, .. })) => raws.push(raw),
                Ok(Some(IrcEvent::Message { text, .. })) => texts.push(text),
                Ok(Some(IrcEvent::Connected { .. })) => connected = true,
                Ok(Some(_)) => {}
                other => panic!("connection ended early: {other:?}"),
            }
        }
        // The NOTICE arrives decoded as a message; only the registration reply is also sent raw.
        assert_eq!(texts[0], "caf\u{e9} \u{20AC}");
        assert_eq!(raws[0], ":srv 001 Assigned :Welcome");
        // 001 also gives us the server-assigned nick.
        assert_eq!(client.current_nick(), "Assigned");
        assert_eq!(client.nick(), "tester");
    }

    #[tokio::test]
    async fn read_line_capped_limits_and_decodes() {
        let long = format!("{}\r\nnext\n", "x".repeat(MAX_READ_LINE * 3));
        let mut data = long.into_bytes();
        data.extend_from_slice(b"caf\xE9\nlast");
        let mut reader = tokio::io::BufReader::new(&data[..]);
        let mut buf = Vec::new();
        let first = read_line_capped(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(first.len(), MAX_READ_LINE);
        assert!(first.bytes().all(|b| b == b'x'));
        assert_eq!(read_line_capped(&mut reader, &mut buf).await.unwrap().unwrap(), "next");
        assert_eq!(read_line_capped(&mut reader, &mut buf).await.unwrap().unwrap(), "caf\u{e9}");
        assert_eq!(read_line_capped(&mut reader, &mut buf).await.unwrap().unwrap(), "last");
        assert!(read_line_capped(&mut reader, &mut buf).await.unwrap().is_none());
    }

    fn payload<'a>(line: &'a str, target: &str) -> &'a str {
        line.strip_prefix(&format!("PRIVMSG {} :", target)).expect("PRIVMSG prefix")
    }

    #[test]
    fn long_encrypted_message_is_split_under_the_line_limit() {
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(40);
        let text = text.trim_end().to_string();
        for key in ["plainkey123", "cbc:cbckey456"] {
            let lines = build_privmsg_lines("#chan", &text, Some(key)).unwrap();
            assert!(lines.len() > 1);
            let cipher = crate::crypto::fish::FishCipher::new(key);
            let mut joined = Vec::new();
            for line in &lines {
                assert!(line.len() + 2 < 510, "line of {} bytes", line.len());
                let plain = cipher.decrypt(payload(line, "#chan")).unwrap();
                assert!(!plain.is_empty());
                joined.push(plain);
            }
            assert_eq!(joined.join(" "), text, "key {key}");
        }
    }

    #[test]
    fn encoded_length_formula_matches_the_cipher() {
        for key in ["plainkey123", "cbc:cbckey456"] {
            let cipher = crate::crypto::fish::FishCipher::new(key);
            for n in 1..=FISH_CHUNK {
                let enc = cipher.encrypt(&"a".repeat(n)).unwrap();
                assert_eq!(enc.len(), fish_encoded_len(cipher.is_cbc(), n), "{key} n={n}");
            }
        }
    }

    #[test]
    fn long_plain_message_is_split_at_400_bytes() {
        let text = format!("{} tail", "word ".repeat(200));
        let lines = build_privmsg_lines("nick", &text, None).unwrap();
        assert!(lines.len() >= 3);
        for line in &lines {
            assert!(payload(line, "nick").len() <= 400);
            assert!(line.len() + 2 < 510);
        }
        let joined: Vec<&str> = lines.iter().map(|l| payload(l, "nick")).collect();
        assert_eq!(joined.join(" "), text);
        // Multi-byte text is never cut inside a character.
        let lines = build_privmsg_lines("nick", &"\u{e9}".repeat(900), None).unwrap();
        assert!(lines.iter().all(|l| payload(l, "nick").chars().all(|c| c == '\u{e9}')));
    }

    #[test]
    fn short_and_multiline_messages() {
        assert_eq!(build_privmsg_lines("#c", "hi", None).unwrap(), vec!["PRIVMSG #c :hi"]);
        // Empty segments before separators are forwarded; a trailing separator adds none; spaces after a
        // separator are skipped.
        assert_eq!(
            build_privmsg_lines("#c", "a\r\n\r\nb", None).unwrap(),
            vec!["PRIVMSG #c :a", "PRIVMSG #c :", "PRIVMSG #c :b"]
        );
        assert_eq!(
            build_privmsg_lines("#c", "\r\n  x\n", None).unwrap(),
            vec!["PRIVMSG #c :", "PRIVMSG #c :x"]
        );
        // The text is cut to 0x2800 minus the target's length: 10238 units for a two-unit target.
        let long = "a".repeat(10300);
        let sent: usize = build_privmsg_lines("#c", &long, None)
            .unwrap()
            .iter()
            .map(|l| l.len() - "PRIVMSG #c :".len())
            .sum();
        assert_eq!(sent, 0x2800 - 2);
        // A newline never lets text become a second command.
        let lines = build_privmsg_lines("#c", "a\r\nQUIT :x", None).unwrap();
        assert_eq!(lines, vec!["PRIVMSG #c :a", "PRIVMSG #c :QUIT :x"]);
    }

    #[test]
    fn find_fish_key_unaffected_by_channel_types() {
        // PRIVMSG key lookup uses the shared channel check.
        assert!(is_channel_name_with("&ops", None));
        assert!(!is_channel_name_with("&ops", Some("#")));
    }

}

/// The text of an error reply: everything after our own nick, as the server sent it.
fn error_text(msg: &IrcMessage) -> String {
    msg.params.iter().skip(1).cloned().collect::<Vec<_>>().join(" ")
}

/// One WHO reply (352) as a status line: `nick [user@host] flags realname`. None when the reply has
/// fewer parameters than the standard form.
fn who_line(msg: &IrcMessage) -> Option<String> {
    let p = &msg.params;
    if p.len() < 8 {
        return None;
    }
    let (user, host, nick, flags) = (&p[2], &p[3], &p[5], &p[6]);
    let realname = p[7].split_once(' ').map(|(_, real)| real).unwrap_or("");
    Some(format!("{nick} [{user}@{host}] {flags} {realname}"))
}

/// True when a message is our own line echoed back under echo-message: the sender is our nick and
/// the capability is on. Such a line is already in the chat, so it is not shown twice.
fn is_own_echo(echo_enabled: bool, sender: &str, own_nick: &str) -> bool {
    echo_enabled && !own_nick.is_empty() && sender.eq_ignore_ascii_case(own_nick)
}

/// The status line for an IRCv3 notification about another user.
fn capability_notice(command: &str, who: &str, params: &[String]) -> String {
    let arg = |i: usize| params.get(i).map(String::as_str).unwrap_or("");
    match command {
        "AWAY" if params.is_empty() => format!("{who} is back"),
        "AWAY" => format!("{who} is away: {}", arg(0)),
        "ACCOUNT" if arg(0) == "*" => format!("{who} logged out of its account"),
        "ACCOUNT" => format!("{who} is logged in as {}", arg(0)),
        "CHGHOST" => format!("{who} is now {}@{}", arg(0), arg(1)),
        "SETNAME" => format!("{who} changed the real name to {}", arg(0)),
        "INVITE" => format!("{who} invites {} to {}", arg(0), arg(1)),
        _ => format!("{who}: {command}"),
    }
}

/// Milliseconds since the epoch for an IRCv3 server-time value (`YYYY-MM-DDThh:mm:ss[.sss]Z`).
fn parse_server_time(value: &str) -> Option<u64> {
    let (date, time) = value.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = hms.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next()?.parse().ok()?;
    let millis: i64 = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    // Days since 1970-01-01 (proleptic Gregorian calendar).
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds * 1_000 + millis).ok()
}

/// A server notice for the status window.
fn status_notice(network: &str, text: String) -> IrcEvent {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    IrcEvent::Message {
        sender_address: None,
        network: network.to_string(),
        target: "status".into(),
        sender: "*".into(),
        spans: Vec::new(),
        text,
        is_notice: true,
        is_action: false,
        is_encrypted: false,
        timestamp: now,
        sender_host: None,
    }
}

/// Capabilities this client asks for when the server offers them.
const WANTED_CAPS: &[&str] = &[
    "multi-prefix",
    "extended-join",
    "echo-message",
    "server-time",
    "message-tags",
    "away-notify",
    "account-notify",
    "userhost-in-names",
    "chghost",
    "setname",
    "invite-notify",
    "cap-notify",
    "monitor",
];

/// The capabilities to request now: the wanted ones the server offers and that are not enabled yet,
/// plus SASL when credentials are set.
fn capabilities_to_request(
    offered: &std::collections::HashSet<String>,
    enabled: &std::collections::HashSet<String>,
    want_sasl: bool,
) -> Vec<&'static str> {
    WANTED_CAPS
        .iter()
        .copied()
        .chain(want_sasl.then_some("sasl"))
        .filter(|cap| offered.contains(*cap) && !enabled.contains(*cap))
        .collect()
}

/// Standard base64 (RFC 4648) with padding, for the SASL PLAIN payload.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The base64 payload split into SASL lines of at most 400 characters. An empty payload is "+",
/// and a payload of exactly 400 characters is followed by "+" to end it.
fn sasl_chunks(encoded: &str) -> Vec<String> {
    if encoded.is_empty() {
        return vec!["+".to_string()];
    }
    let mut lines: Vec<String> = encoded
        .as_bytes()
        .chunks(400)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    if encoded.len() % 400 == 0 {
        lines.push("+".to_string());
    }
    lines
}

/// The nick for the next refusal: the configured alternate first, then the base with numbers.
/// `attempt` counts the refusals so far; the numbers start at 1 after the alternate.
fn nick_candidate(base: &str, altnick: Option<&str>, attempt: u32) -> String {
    match altnick {
        Some(alt) if attempt == 0 => alt.to_string(),
        Some(_) => alternate_nick(base, attempt),
        None => alternate_nick(base, attempt + 1),
    }
}

/// How many alternate nicks are tried after a refusal before registration (433).
const MAX_NICK_ATTEMPTS: u32 = 10;

/// The nick for attempt `attempt` (1-based) after a refusal: the base with the attempt number
/// appended. The base is cut so the result keeps to 9 characters, the limit of RFC 1459 servers.
fn alternate_nick(base: &str, attempt: u32) -> String {
    const NICKLEN: usize = 9;
    let digits = attempt.to_string();
    let keep = NICKLEN.saturating_sub(digits.len()).max(1);
    let stem: String = base.chars().take(keep).collect();
    format!("{stem}{digits}")
}

#[cfg(test)]
mod wire_tests {
    use super::wire_line;

    #[test]
    fn wire_line_refuses_embedded_line_breaks() {
        assert_eq!(wire_line("PRIVMSG a :hi").unwrap(), "PRIVMSG a :hi\r\n");
        assert_eq!(wire_line("PRIVMSG a :hi\r\n").unwrap(), "PRIVMSG a :hi\r\n");
        assert!(wire_line("PRIVMSG a :hi\r\nQUIT").is_err());
        assert!(wire_line("PRIVMSG a :hi\nQUIT").is_err());
        assert!(wire_line("PRIVMSG a :hi\rQUIT").is_err());
        assert!(wire_line("PRIVMSG a :hi\0").is_err());
    }
}

#[cfg(test)]
mod nick_tests {
    use super::{alternate_nick, MAX_NICK_ATTEMPTS};

    #[test]
    fn the_configured_alternate_is_tried_first() {
        use super::nick_candidate;
        assert_eq!(nick_candidate("Alice", Some("Ali"), 0), "Ali");
        assert_eq!(nick_candidate("Alice", Some("Ali"), 1), "Alice1");
        assert_eq!(nick_candidate("Alice", None, 0), "Alice1");
        assert_eq!(nick_candidate("Alice", None, 1), "Alice2");
    }

    #[test]
    fn alternate_nicks_stay_within_nine_characters_and_differ() {
        assert_eq!(alternate_nick("Alice", 1), "Alice1");
        assert_eq!(alternate_nick("Alexandrina", 2), "Alexandr2");
        assert_eq!(alternate_nick("Alexandrina", 10), "Alexand10");
        assert_eq!(alternate_nick("Bob", MAX_NICK_ATTEMPTS), "Bob10");
        let all: Vec<String> = (1..=MAX_NICK_ATTEMPTS).map(|n| alternate_nick("Carol", n)).collect();
        let unique: std::collections::HashSet<&String> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
        assert!(all.iter().all(|nick| nick.chars().count() <= 9));
    }
}

#[cfg(test)]
mod reply_tests {
    use super::{capability_notice, error_text, parse_server_time, who_line, IrcMessage};

    #[test]
    fn a_who_reply_reads_as_nick_host_flags_and_name() {
        let msg = IrcMessage::parse(":srv 352 me #chan ~u host.example srv nick H :0 Real Name").unwrap();
        assert_eq!(who_line(&msg).as_deref(), Some("nick [~u@host.example] H Real Name"));
    }

    #[test]
    fn ircv3_notices_read_as_status_lines() {
        assert_eq!(capability_notice("AWAY", "Ann", &["gone".to_string()]), "Ann is away: gone");
        assert_eq!(capability_notice("AWAY", "Ann", &[]), "Ann is back");
        assert_eq!(capability_notice("ACCOUNT", "Ann", &["*".to_string()]), "Ann logged out of its account");
        assert_eq!(capability_notice("CHGHOST", "Ann", &["u2".to_string(), "h2".to_string()]), "Ann is now u2@h2");
    }

    #[test]
    fn server_time_is_read_in_milliseconds() {
        assert_eq!(parse_server_time("1970-01-01T00:00:01.500Z"), Some(1_500));
        assert_eq!(parse_server_time("2026-11-22T10:00:00.000Z"), Some(1_795_341_600_000));
        assert_eq!(parse_server_time("2026-11-22T10:00:00Z"), Some(1_795_341_600_000));
        assert_eq!(parse_server_time("not a time"), None);
    }


    #[test]
    fn an_error_reply_keeps_its_target_and_reason() {
        let join = IrcMessage::parse(":srv 471 me #chan :Cannot join channel (+l)").unwrap();
        assert_eq!(error_text(&join), "#chan Cannot join channel (+l)");
        let nick = IrcMessage::parse(":srv 401 me ghost :No such nick/channel").unwrap();
        assert_eq!(error_text(&nick), "ghost No such nick/channel");
    }
}

#[cfg(test)]
mod cap_tests {
    use super::{base64_encode, capabilities_to_request, is_own_echo, sasl_chunks};
    use std::collections::HashSet;

    #[test]
    fn only_our_own_line_is_dropped_under_echo_message() {
        assert!(is_own_echo(true, "Ferric", "ferric"));
        assert!(!is_own_echo(false, "Ferric", "ferric"));
        assert!(!is_own_echo(true, "Ann", "ferric"));
        assert!(!is_own_echo(true, "Ferric", ""));
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"\0alice\0secret"), "AGFsaWNlAHNlY3JldA==");
    }

    #[test]
    fn a_long_payload_is_split_and_closed_when_it_fills_the_last_line() {
        assert_eq!(sasl_chunks(""), vec!["+".to_string()]);
        assert_eq!(sasl_chunks("abc"), vec!["abc".to_string()]);
        let exact = "a".repeat(400);
        assert_eq!(sasl_chunks(&exact), vec![exact.clone(), "+".to_string()]);
        let long = "b".repeat(401);
        let parts = sasl_chunks(&long);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 400);
        assert_eq!(parts[1], "b");
    }

    #[test]
    fn only_wanted_and_offered_capabilities_are_requested() {
        let offered: HashSet<String> = ["multi-prefix", "sasl", "tls", "server-time"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let enabled: HashSet<String> = ["server-time".to_string()].into();
        assert_eq!(capabilities_to_request(&offered, &enabled, false), vec!["multi-prefix"]);
        assert_eq!(capabilities_to_request(&offered, &enabled, true), vec!["multi-prefix", "sasl"]);
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::{parse_fingerprint, PinnedCertificate};
    use std::sync::Arc;
    use rustls::client::danger::ServerCertVerifier;
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    #[test]
    fn pinned_verifier_accepts_only_the_pinned_certificate() {
        let good = b"certificate bytes".to_vec();
        let digest = <sha2::Sha256 as sha2::Digest>::digest(&good);
        let mut pin = [0u8; 32];
        pin.copy_from_slice(&digest);
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = PinnedCertificate::new(Some(pin), false, false, provider);
        let name = ServerName::try_from("irc.example.org").unwrap();
        let now = UnixTime::now();
        let cert = CertificateDer::from(good);
        assert!(verifier.verify_server_cert(&cert, &[], &name, &[], now).is_ok());
        let other = CertificateDer::from(b"another certificate".to_vec());
        assert!(verifier.verify_server_cert(&other, &[], &name, &[], now).is_err());
    }

    #[test]
    fn strict_verifier_refuses_garbage_certificates() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier = PinnedCertificate::new(None, true, true, provider);
        let name = ServerName::try_from("irc.example.org").unwrap();
        let now = UnixTime::now();
        let junk = CertificateDer::from(b"not a certificate".to_vec());
        assert!(verifier.verify_server_cert(&junk, &[], &name, &[], now).is_err());
    }

    #[test]
    fn fingerprints_are_parsed_with_or_without_separators() {
        let hex = "ab".repeat(32);
        let spaced: String = hex.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(":");
        assert_eq!(parse_fingerprint(&hex).unwrap(), [0xab; 32]);
        assert_eq!(parse_fingerprint(&spaced).unwrap(), [0xab; 32]);
        assert!(parse_fingerprint("abc").is_err());
        assert!(parse_fingerprint(&"zz".repeat(32)).is_err());
    }
}
