//! Proxy support for IRC connections (SOCKS5 and HTTP CONNECT). Off by default.
//!
//! The password is never stored: the caller passes it to `connect_via` per call.
//! Error messages are fixed strings and never include the password or request bytes.

use std::future::Future;
use std::io::{self, Error, ErrorKind};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use zeroize::Zeroize;

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HTTP_HEAD: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    Socks4,
    Socks5,
    HttpConnect,
}

/// Proxy settings. Holds no secret: the password is passed to `connect_via`.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
}

fn invalid(msg: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, msg)
}

fn proto(msg: &'static str) -> Error {
    Error::new(ErrorKind::Other, msg)
}

/// Run one network step with the per-step timeout.
async fn step<T>(fut: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    match timeout(STEP_TIMEOUT, fut).await {
        Ok(r) => r,
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "proxy step timed out")),
    }
}

fn validate_target(host: &str, port: u16) -> io::Result<()> {
    if host.is_empty() || host.len() > 253 {
        return Err(invalid("invalid target host length"));
    }
    // An IPv6 literal has colons, so it is checked as an address, not as a name.
    let is_ipv6 = host.parse::<std::net::Ipv6Addr>().is_ok();
    if !is_ipv6
        && !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(invalid("invalid character in target host"));
    }
    if port == 0 {
        return Err(invalid("invalid target port"));
    }
    Ok(())
}

fn no_ctl(s: &str) -> bool {
    // Every control character, not only the ones that end a header line.
    !s.bytes().any(|b| b < 0x20 || b == 0x7f)
}

/// Connect to `target_host:target_port` through the proxy. Returns the stream
/// positioned right after the proxy handshake.
pub async fn connect_via(
    config: &ProxyConfig,
    password: Option<&str>,
    target_host: &str,
    target_port: u16,
) -> io::Result<TcpStream> {
    validate_target(target_host, target_port)?;
    if config.kind == ProxyKind::Socks4 && target_host.parse::<std::net::Ipv6Addr>().is_ok() {
        return Err(invalid("SOCKS4 cannot reach an IPv6 address"));
    }
    if config.host.is_empty() || config.port == 0 || !no_ctl(&config.host) {
        return Err(invalid("invalid proxy address"));
    }
    // Credentials are only used when both parts are present.
    let creds = match (config.username.as_deref(), password) {
        (Some(u), Some(p)) => {
            if !no_ctl(u) || !no_ctl(p) {
                return Err(invalid("invalid character in proxy credentials"));
            }
            Some((u, p))
        }
        _ => None,
    };

    let work = async {
        let mut stream = step(TcpStream::connect((config.host.as_str(), config.port))).await?;
        match config.kind {
            ProxyKind::Socks4 => {
                let user_id = config.username.as_deref().unwrap_or("");
                socks4(&mut stream, user_id, target_host, target_port).await?
            }
            ProxyKind::Socks5 => socks5(&mut stream, creds, target_host, target_port).await?,
            ProxyKind::HttpConnect => http_connect(&mut stream, creds, target_host, target_port).await?,
        }
        Ok(stream)
    };
    match timeout(TOTAL_TIMEOUT, work).await {
        Ok(r) => r,
        Err(_) => Err(Error::new(ErrorKind::TimedOut, "proxy connect timed out")),
    }
}

/// SOCKS4a: an IPv4 target goes by address; a name goes by the 4a extension (address 0.0.0.1,
/// the name after the user ID). The user ID is the proxy's user name. SOCKS4 has no password.
async fn socks4(s: &mut TcpStream, user_id: &str, host: &str, port: u16) -> io::Result<()> {
    if !no_ctl(user_id) {
        return Err(invalid("invalid character in proxy user name"));
    }
    let ip4 = host.parse::<std::net::Ipv4Addr>().ok();
    let mut req = vec![0x04, 0x01];
    req.extend_from_slice(&port.to_be_bytes());
    req.extend_from_slice(&ip4.map_or([0, 0, 0, 1], |ip| ip.octets()));
    req.extend_from_slice(user_id.as_bytes());
    req.push(0);
    if ip4.is_none() {
        req.extend_from_slice(host.as_bytes());
        req.push(0);
    }
    step(s.write_all(&req)).await?;
    let mut reply = [0u8; 8];
    step(s.read_exact(&mut reply)).await?;
    if reply[0] != 0x00 {
        return Err(proto("socks4 bad reply version"));
    }
    if reply[1] != 0x5a {
        return Err(proto("socks4 connect refused"));
    }
    Ok(())
}

async fn socks5(
    s: &mut TcpStream,
    creds: Option<(&str, &str)>,
    host: &str,
    port: u16,
) -> io::Result<()> {
    let method: u8 = if creds.is_some() { 0x02 } else { 0x00 };
    step(s.write_all(&[0x05, 0x01, method])).await?;
    let mut rep = [0u8; 2];
    step(s.read_exact(&mut rep)).await?;
    if rep[0] != 0x05 || rep[1] != method {
        return Err(proto("socks5 method rejected"));
    }

    if let Some((user, pass)) = creds {
        // RFC 1929: each field is limited to 255 bytes.
        if user.len() > 255 || pass.len() > 255 {
            return Err(invalid("proxy credentials too long"));
        }
        let mut msg = Vec::with_capacity(3 + user.len() + pass.len());
        msg.push(0x01);
        msg.push(user.len() as u8);
        msg.extend_from_slice(user.as_bytes());
        msg.push(pass.len() as u8);
        msg.extend_from_slice(pass.as_bytes());
        let sent = step(s.write_all(&msg)).await;
        msg.zeroize();
        sent?;
        let mut ar = [0u8; 2];
        step(s.read_exact(&mut ar)).await?;
        if ar[1] != 0x00 {
            return Err(Error::new(ErrorKind::PermissionDenied, "socks5 authentication failed"));
        }
    }

    // An IPv6 literal goes as its 16 raw bytes (ATYP 4); a name goes as a domain (ATYP 3).
    // Host length is already validated to 1..=253.
    let mut req = Vec::with_capacity(22 + host.len());
    match host.parse::<std::net::Ipv6Addr>() {
        Ok(ip) => {
            req.extend_from_slice(&[0x05, 0x01, 0x00, 0x04]);
            req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
            req.extend_from_slice(host.as_bytes());
        }
    }
    req.extend_from_slice(&port.to_be_bytes());
    step(s.write_all(&req)).await?;

    let mut head = [0u8; 4];
    step(s.read_exact(&mut head)).await?;
    if head[0] != 0x05 {
        return Err(proto("socks5 bad reply version"));
    }
    if head[1] != 0x00 {
        return Err(proto("socks5 connect refused"));
    }
    // Consume the bound address so the stream sits after the reply.
    let addr_len = match head[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => {
            let mut l = [0u8; 1];
            step(s.read_exact(&mut l)).await?;
            l[0] as usize
        }
        _ => return Err(proto("socks5 bad address type")),
    };
    let mut rest = vec![0u8; addr_len + 2];
    step(s.read_exact(&mut rest)).await?;
    Ok(())
}

async fn http_connect(
    s: &mut TcpStream,
    creds: Option<(&str, &str)>,
    host: &str,
    port: u16,
) -> io::Result<()> {
    // An IPv6 literal is bracketed in the authority, as in [2001:db8::1]:6667.
    let authority = match host.parse::<std::net::Ipv6Addr>() {
        Ok(_) => format!("[{host}]:{port}"),
        Err(_) => format!("{host}:{port}"),
    };
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((user, pass)) = creds {
        if user.contains(':') {
            return Err(invalid("proxy username must not contain ':'"));
        }
        let mut raw = format!("{user}:{pass}");
        let mut enc = base64(raw.as_bytes());
        req.push_str("Proxy-Authorization: Basic ");
        req.push_str(&enc);
        req.push_str("\r\n");
        raw.zeroize();
        enc.zeroize();
    }
    req.push_str("\r\n");
    let sent = step(s.write_all(req.as_bytes())).await;
    req.zeroize();
    sent?;

    // Read byte by byte so no tunnelled data is consumed past the blank line.
    let mut head: Vec<u8> = Vec::with_capacity(256);
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HTTP_HEAD {
            return Err(proto("proxy response too large"));
        }
        let n = step(s.read(&mut b)).await?;
        if n == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "proxy closed connection"));
        }
        head.push(b[0]);
    }

    let line_end = head.iter().position(|&c| c == b'\r').unwrap_or(head.len());
    let line = &head[..line_end];
    let ok = [&b"HTTP/1.1 200"[..], &b"HTTP/1.0 200"[..]]
        .iter()
        .any(|p| line.starts_with(p) && matches!(line.get(p.len()), None | Some(b' ')));
    if ok {
        return Ok(());
    }
    // Report only a numeric status code, never proxy-supplied text.
    let code: String = line
        .split(|&c| c == b' ')
        .nth(1)
        .map(|c| c.iter().take(3).filter(|d| d.is_ascii_digit()).map(|&d| d as char).collect())
        .unwrap_or_default();
    Err(Error::new(
        ErrorKind::ConnectionRefused,
        if code.len() == 3 {
            format!("proxy refused CONNECT (status {code})")
        } else {
            "proxy refused CONNECT".to_string()
        },
    ))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn listener() -> (TcpListener, u16) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        (l, p)
    }

    fn cfg(kind: ProxyKind, port: u16, user: Option<&str>) -> ProxyConfig {
        ProxyConfig { kind, host: "127.0.0.1".into(), port, username: user.map(String::from) }
    }

    /// Read until the blank line ending an HTTP request head.
    async fn read_head(s: &mut TcpStream) -> String {
        let mut buf = Vec::new();
        let mut b = [0u8; 1];
        while !buf.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).await.unwrap();
            buf.push(b[0]);
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[tokio::test]
    async fn socks5_no_auth_handshake() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut g = [0u8; 3];
            s.read_exact(&mut g).await.unwrap();
            assert_eq!(g, [5, 1, 0]);
            s.write_all(&[5, 0]).await.unwrap();
            let mut h = [0u8; 5];
            s.read_exact(&mut h).await.unwrap();
            let mut rest = vec![0u8; h[4] as usize + 2];
            s.read_exact(&mut rest).await.unwrap();
            // success, bound address as a domain name to test full read
            s.write_all(&[5, 0, 0, 3, 3, b'a', b'b', b'c', 0x1f, 0x90]).await.unwrap();
            s.write_all(b"Z").await.unwrap();
            let mut all = h.to_vec();
            all.extend_from_slice(&rest);
            all
        });
        let mut st = connect_via(&cfg(ProxyKind::Socks5, port, None), None, "irc.example.net", 6697)
            .await
            .unwrap();
        // stream must be positioned after the reply: first byte is the tunnel data
        let mut z = [0u8; 1];
        st.read_exact(&mut z).await.unwrap();
        assert_eq!(z[0], b'Z');
        let got = server.await.unwrap();
        let mut want = vec![5, 1, 0, 3, 15];
        want.extend_from_slice(b"irc.example.net");
        want.extend_from_slice(&6697u16.to_be_bytes());
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn socks5_auth_failure_hides_password() {
        let (l, port) = listener().await;
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut g = [0u8; 3];
            s.read_exact(&mut g).await.unwrap();
            assert_eq!(g, [5, 1, 2]);
            s.write_all(&[5, 2]).await.unwrap();
            let mut v = [0u8; 2];
            s.read_exact(&mut v).await.unwrap();
            let mut rest = vec![0u8; v[1] as usize + 1];
            s.read_exact(&mut rest).await.unwrap();
            let mut pw = vec![0u8; rest[v[1] as usize] as usize];
            s.read_exact(&mut pw).await.unwrap();
            s.write_all(&[1, 1]).await.unwrap();
        });
        let err = connect_via(&cfg(ProxyKind::Socks5, port, Some("bob")), Some("hunter2secret"), "irc.example.net", 6667)
            .await
            .unwrap_err();
        assert!(!err.to_string().contains("hunter2secret"));
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn http_connect_200() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let head = read_head(&mut s).await;
            s.write_all(b"HTTP/1.1 200 Connection established\r\nX-Foo: bar\r\n\r\nQ").await.unwrap();
            head
        });
        let mut st = connect_via(&cfg(ProxyKind::HttpConnect, port, None), None, "irc.example.net", 6667)
            .await
            .unwrap();
        let mut q = [0u8; 1];
        st.read_exact(&mut q).await.unwrap();
        assert_eq!(q[0], b'Q');
        let head = server.await.unwrap();
        assert!(head.starts_with("CONNECT irc.example.net:6667 HTTP/1.1\r\n"));
        assert!(head.contains("Host: irc.example.net:6667\r\n"));
        assert!(!head.contains("Proxy-Authorization"));
    }

    #[tokio::test]
    async fn http_connect_sends_basic_auth_when_given() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let head = read_head(&mut s).await;
            s.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await.unwrap();
            head
        });
        connect_via(&cfg(ProxyKind::HttpConnect, port, Some("user")), Some("pass"), "irc.example.net", 6667)
            .await
            .unwrap();
        assert!(server.await.unwrap().contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"));
    }

    #[tokio::test]
    async fn http_connect_407_fails_without_leaking_password() {
        let (l, port) = listener().await;
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = read_head(&mut s).await;
            s.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n").await.unwrap();
        });
        let err = connect_via(&cfg(ProxyKind::HttpConnect, port, Some("bob")), Some("hunter2secret"), "irc.example.net", 6667)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("407"));
        assert!(!msg.contains("hunter2secret"));
        assert!(!msg.contains("aHVudGVyMnNlY3JldA")); // base64 of the password
    }

    #[tokio::test]
    async fn socks5_ipv6_target_is_sent_as_address_type_4() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut g = [0u8; 3];
            s.read_exact(&mut g).await.unwrap();
            s.write_all(&[5, 0]).await.unwrap();
            let mut h = [0u8; 4];
            s.read_exact(&mut h).await.unwrap();
            let mut rest = vec![0u8; 16 + 2];
            s.read_exact(&mut rest).await.unwrap();
            // success, bound address 0.0.0.0:0
            s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await.unwrap();
            let mut all = h.to_vec();
            all.extend_from_slice(&rest);
            all
        });
        connect_via(&cfg(ProxyKind::Socks5, port, None), None, "2001:db8::1", 6697)
            .await
            .unwrap();
        let mut want = vec![5, 1, 0, 4];
        want.extend_from_slice(&"2001:db8::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
        want.extend_from_slice(&6697u16.to_be_bytes());
        assert_eq!(server.await.unwrap(), want);
    }

    #[tokio::test]
    async fn http_connect_brackets_an_ipv6_target() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let head = read_head(&mut s).await;
            s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
            head
        });
        connect_via(&cfg(ProxyKind::HttpConnect, port, None), None, "2001:db8::1", 6667)
            .await
            .unwrap();
        let head = server.await.unwrap();
        assert!(head.starts_with("CONNECT [2001:db8::1]:6667 HTTP/1.1\r\n"));
        assert!(head.contains("Host: [2001:db8::1]:6667\r\n"));
    }

    #[tokio::test]
    async fn socks4a_sends_the_name_after_the_user_id() {
        let (l, port) = listener().await;
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut all = vec![0u8; 8];
            s.read_exact(&mut all).await.unwrap();
            // The user ID and the host name, each ended by a zero byte.
            let mut zeros = 0;
            while zeros < 2 {
                let mut b = [0u8; 1];
                s.read_exact(&mut b).await.unwrap();
                all.push(b[0]);
                if b[0] == 0 {
                    zeros += 1;
                }
            }
            s.write_all(&[0, 0x5a, 0, 0, 0, 0, 0, 0]).await.unwrap();
            all
        });
        connect_via(&cfg(ProxyKind::Socks4, port, Some("bob")), None, "irc.example.net", 6667)
            .await
            .unwrap();
        let mut want = vec![4, 1];
        want.extend_from_slice(&6667u16.to_be_bytes());
        want.extend_from_slice(&[0, 0, 0, 1]);
        want.extend_from_slice(b"bob\0irc.example.net\0");
        assert_eq!(server.await.unwrap(), want);
    }

    #[tokio::test]
    async fn socks4_refuses_an_ipv6_target_before_connecting() {
        let (l, port) = listener().await;
        let err = connect_via(&cfg(ProxyKind::Socks4, port, None), None, "2001:db8::1", 6667)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(timeout(Duration::from_millis(100), l.accept()).await.is_err());
    }

    #[tokio::test]
    async fn invalid_target_refused_before_connecting() {
        let (l, port) = listener().await;
        let long = "a".repeat(254);
        let cases: Vec<(&str, u16)> = vec![
            ("", 6667),
            (long.as_str(), 6667),
            ("bad host", 6667),
            ("evil.com\r\nX: y", 6667),
            ("nul\0.com", 6667),
            ("ex_ample.com", 6667),
            ("irc.example.net", 0),
        ];
        for kind in [ProxyKind::Socks5, ProxyKind::HttpConnect] {
            for (h, p) in &cases {
                let err = connect_via(&cfg(kind, port, None), None, h, *p).await.unwrap_err();
                assert_eq!(err.kind(), ErrorKind::InvalidInput, "{h:?}:{p}");
            }
        }
        // the proxy listener never saw a connection
        assert!(timeout(Duration::from_millis(100), l.accept()).await.is_err());
    }

    #[tokio::test]
    async fn control_chars_in_credentials_refused() {
        let (l, port) = listener().await;
        let err = connect_via(&cfg(ProxyKind::HttpConnect, port, Some("bob")), Some("pw\r\nX: y"), "irc.example.net", 6667)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(!err.to_string().contains("pw"));
        assert!(timeout(Duration::from_millis(100), l.accept()).await.is_err());
    }
}

#[cfg(test)]
mod target_tests {
    use super::validate_target;

    #[test]
    fn ipv6_literals_are_targets_and_bad_names_are_not() {
        assert!(validate_target("2001:db8::1", 6667).is_ok());
        assert!(validate_target("irc.example.org", 6667).is_ok());
        assert!(validate_target("bad host", 6667).is_err());
        assert!(validate_target("a:b", 6667).is_err());
        assert!(validate_target("irc.example.org", 0).is_err());
    }
}
