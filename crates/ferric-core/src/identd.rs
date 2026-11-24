//! Identd responder on port 113. Off by default.
//!
//! RFC 1413 reply for one request line. The module never binds a port: the
//! caller binds the listener and passes it to [`serve`].

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::timeout;

const MAX_REQUEST: usize = 64;
const MAX_USERNAME: usize = 32;
const MAX_CONNECTIONS: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const FALLBACK_USER: &str = "ferric";
const INVALID_ZERO: &str = "0, 0 : ERROR : INVALID-PORT";

fn sanitize_username(username: &str) -> String {
    let clean: String = username
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(MAX_USERNAME)
        .collect();
    if clean.is_empty() {
        FALLBACK_USER.to_string()
    } else {
        clean
    }
}

/// Valid port: decimal digits only, 1..=65535.
fn parse_port(s: &str) -> Option<u16> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>()
        .ok()
        .filter(|p| (1..=65535).contains(p))
        .map(|p| p as u16)
}

/// Answer one identd request (no line ending). Success replies end in CRLF;
/// error replies do not, `serve` adds the terminator.
pub fn answer(request: &str, username: &str) -> String {
    if request.len() > MAX_REQUEST
        || !request
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b' ')
    {
        return INVALID_ZERO.to_string();
    }
    let parts: Vec<&str> = request.split(',').map(str::trim).collect();
    if parts.len() != 2 {
        return INVALID_ZERO.to_string();
    }
    match (parse_port(parts[0]), parse_port(parts[1])) {
        (Some(p1), Some(p2)) => format!(
            "{p1}, {p2} : USERID : UNIX : {}\r\n",
            sanitize_username(username)
        ),
        _ => {
            // Echo only digit strings (request is already digits/comma/space).
            let echo = |s: &str| {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    "0".to_string()
                } else {
                    s.to_string()
                }
            };
            format!("{}, {} : ERROR : INVALID-PORT", echo(parts[0]), echo(parts[1]))
        }
    }
}

async fn handle(mut stream: tokio::net::TcpStream, username: &str) {
    let mut buf = Vec::with_capacity(MAX_REQUEST);
    let read = async {
        let mut reader = BufReader::new((&mut stream).take(MAX_REQUEST as u64));
        reader.read_until(b'\n', &mut buf).await
    };
    match timeout(IO_TIMEOUT, read).await {
        Ok(Ok(n)) if n > 0 => {}
        _ => return,
    }
    let mut reply = if buf.last() != Some(&b'\n') && buf.len() >= MAX_REQUEST {
        // Whole budget used without a line ending: too long.
        INVALID_ZERO.to_string()
    } else {
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        answer(&String::from_utf8_lossy(&buf), username)
    };
    if !reply.ends_with("\r\n") {
        reply.push_str("\r\n");
    }
    let _ = timeout(IO_TIMEOUT, async {
        stream.write_all(reply.as_bytes()).await?;
        stream.shutdown().await
    })
    .await;
}

/// Accept loop. Runs until the task is dropped; a failing connection or
/// accept never ends it. At most 32 connections are served at once, extra
/// ones are closed immediately.
pub async fn serve(listener: TcpListener, username: String) -> std::io::Result<()> {
    let username: Arc<str> = username.into();
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(c) => c,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue; // drop stream: closed
        };
        let username = username.clone();
        tokio::spawn(async move {
            handle(stream, &username).await;
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn valid_request_gives_userid() {
        assert_eq!(
            answer("6193, 23", "alice"),
            "6193, 23 : USERID : UNIX : alice\r\n"
        );
        assert_eq!(
            answer(" 6193 ,23 ", "alice"),
            "6193, 23 : USERID : UNIX : alice\r\n"
        );
    }

    #[tokio::test]
    async fn bad_ports_are_invalid() {
        assert_eq!(answer("0, 23", "a"), "0, 23 : ERROR : INVALID-PORT");
        assert_eq!(answer("70000, 23", "a"), "70000, 23 : ERROR : INVALID-PORT");
        assert_eq!(answer("23", "a"), INVALID_ZERO);
        assert_eq!(answer("1, 2, 3", "a"), INVALID_ZERO);
    }

    #[tokio::test]
    async fn letters_and_long_requests_are_invalid() {
        assert_eq!(answer("12, ab", "a"), INVALID_ZERO);
        assert_eq!(answer("12, 34\r\n", "a"), INVALID_ZERO);
        let long = format!("1, {}", "2".repeat(70));
        assert_eq!(answer(&long, "a"), INVALID_ZERO);
    }

    #[tokio::test]
    async fn long_username_is_cut() {
        let name = "a".repeat(50);
        let r = answer("1, 2", &name);
        assert_eq!(r, format!("1, 2 : USERID : UNIX : {}\r\n", "a".repeat(32)));
        assert_eq!(
            answer("1, 2", "bad name:x\r\n"),
            "1, 2 : USERID : UNIX : badnamex\r\n"
        );
    }

    #[tokio::test]
    async fn empty_username_becomes_ferric() {
        assert_eq!(answer("1, 2", ""), "1, 2 : USERID : UNIX : ferric\r\n");
        assert_eq!(answer("1, 2", " :\r\n"), "1, 2 : USERID : UNIX : ferric\r\n");
    }

    async fn roundtrip(request: &[u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(serve(listener, "bob".to_string()));
        let mut c = TcpStream::connect(addr).await.unwrap();
        c.write_all(request).await.unwrap();
        let mut out = Vec::new();
        timeout(Duration::from_secs(10), c.read_to_end(&mut out))
            .await
            .unwrap()
            .unwrap();
        task.abort();
        String::from_utf8(out).unwrap()
    }

    #[tokio::test]
    async fn serve_answers_one_request() {
        assert_eq!(
            roundtrip(b"6667, 51234\r\n").await,
            "6667, 51234 : USERID : UNIX : bob\r\n"
        );
    }

    #[tokio::test]
    async fn serve_rejects_overlong_line() {
        assert_eq!(roundtrip(&[b'1'; 200]).await, "0, 0 : ERROR : INVALID-PORT\r\n");
    }
}
