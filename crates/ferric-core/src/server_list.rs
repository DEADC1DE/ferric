//! The `server` lines of connectd.mrc as a list that the Server manager edits.
//!
//! Only lines that start with `server -m ` are servers; every other line (comments, other script
//! text) is kept as it is. Writing replaces the server lines only.

use serde::{Deserialize, Serialize};

use crate::config_loader::parse_server_port;

const PREFIX: &str = "server -m ";

/// One server line: `server -m HOST:PORT [PASSWORD] -i NICK ALTNICK USERNAME@ [REALNAME]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRow {
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub password: Option<String>,
    pub nick: String,
    pub altnick: String,
    pub username: String,
    pub realname: String,
}

/// Reads one line of the file. `None` unless it is a server line with a host.
/// The defaults for missing parts match the connection code (`Ferric`, `ferric`, `Ferric User`).
pub fn parse_server_line(line: &str) -> Option<ServerRow> {
    let line = line.trim();
    if !line.starts_with(PREFIX) {
        return None;
    }
    let after_prefix = line.trim_start_matches(PREFIX).trim();
    let parts: Vec<&str> = after_prefix.split_whitespace().collect();
    let (host, port, use_tls) = parse_server_port(parts.first()?);

    let mut password = None;
    let mut nick = "Ferric".to_string();
    let mut altnick = nick.clone();
    let mut username = "ferric".to_string();
    let mut realname = "Ferric User".to_string();

    let mut i = 1;
    while i < parts.len() {
        if parts[i] == "-i" {
            if i + 1 < parts.len() {
                nick = parts[i + 1].to_string();
            }
            altnick = parts.get(i + 2).map_or_else(|| nick.clone(), |a| a.to_string());
            // -i <nick> <anick> <email> [name]. The ident is the user part of the e-mail.
            if i + 3 < parts.len() {
                let email_part = parts[i + 3].trim_end_matches('@').trim();
                username = if email_part.is_empty() { nick.clone() } else { email_part.to_string() };
                realname = if i + 4 < parts.len() { parts[i + 4..].join(" ") } else { nick.clone() };
            } else {
                username = nick.clone();
                realname = nick.clone();
            }
            break;
        } else if password.is_none() {
            password = Some(parts[i].to_string());
        }
        i += 1;
    }

    Some(ServerRow {
        host,
        port,
        use_tls,
        password,
        nick,
        altnick,
        username,
        realname,
    })
}

/// All server rows of a file, in file order.
pub fn rows_from_text(text: &str) -> Vec<ServerRow> {
    text.lines().filter_map(parse_server_line).collect()
}

/// Returns `text` with its server lines replaced by `rows`. The new lines go where the first
/// server line was (at the end when there was none); the other server lines are removed. Every
/// other line is kept byte for byte. Fails, changing nothing, when a row cannot be written
/// back so that it reads the same.
pub fn text_with_rows(text: &str, rows: &[ServerRow]) -> Result<String, String> {
    for row in rows {
        check_row(row)?;
    }
    let new_lines: Vec<String> = rows.iter().map(format_row).collect();
    let ends_with_newline = text.is_empty() || text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(text);

    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    if !text.is_empty() {
        for line in body.split('\n') {
            if line.trim().starts_with(PREFIX) {
                if !placed {
                    out.extend(new_lines.iter().cloned());
                    placed = true;
                }
            } else {
                out.push(line.to_string());
            }
        }
    }
    if !placed {
        out.extend(new_lines);
    }

    let mut result = out.join("\n");
    if ends_with_newline && !out.is_empty() {
        result.push('\n');
    }
    Ok(result)
}

/// The server line for one row. TLS is written as `+port`; a host with colons is bracketed.
fn format_row(row: &ServerRow) -> String {
    let host = if row.host.contains(':') {
        format!("[{}]", row.host)
    } else {
        row.host.clone()
    };
    let port = if row.use_tls {
        format!("+{}", row.port)
    } else {
        row.port.to_string()
    };
    let mut line = format!("{PREFIX}{host}:{port}");
    if let Some(password) = row.password.as_deref().filter(|p| !p.is_empty()) {
        line.push(' ');
        line.push_str(password);
    }
    line.push_str(&format!(" -i {} {} {}@", row.nick, row.altnick, row.username));
    if !row.realname.is_empty() {
        line.push(' ');
        line.push_str(&row.realname);
    }
    line
}

/// Rejects a row whose fields would not read back as written. Fields are single words, except
/// the real name.
pub fn check_row(row: &ServerRow) -> Result<(), String> {
    let word = |s: &str| !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    if !word(&row.host) || row.host.contains(['[', ']', '@']) {
        return Err(format!("Host \"{}\" must be one word without [ ] or @.", row.host));
    }
    if row.port == 0 {
        return Err("Port must be between 1 and 65535.".to_string());
    }
    if let Some(password) = row.password.as_deref().filter(|p| !p.is_empty()) {
        if !word(password) || password == "-i" {
            return Err("Password must be one word and not -i.".to_string());
        }
    }
    for (label, value) in [("Nick", &row.nick), ("Alt nick", &row.altnick)] {
        if !word(value) || value.starts_with('-') {
            return Err(format!("{label} must be one word and must not start with -."));
        }
    }
    if !word(&row.username) || row.username.contains('@') {
        return Err("Ident must be one word without @.".to_string());
    }
    if row.realname.is_empty() || row.realname.chars().any(char::is_control) {
        return Err("Real name must not be empty or contain control characters (tabs, line breaks).".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(host: &str, port: u16, use_tls: bool) -> ServerRow {
        ServerRow {
            host: host.into(),
            port,
            use_tls,
            password: None,
            nick: "ferric".into(),
            altnick: "ferric_".into(),
            username: "ferric".into(),
            realname: "Ferric User".into(),
        }
    }

    #[test]
    fn parses_server_lines_in_file_order_and_skips_other_lines() {
        let text = "; a comment\nalias x { echo hi }\nserver -m irc.one.org:6667 -i nick alt me@ Real Name\n  server -m two.org:+6697 pw -i n2 a2 u2@\n";
        let rows = rows_from_text(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].host, "irc.one.org");
        assert_eq!(rows[0].altnick, "alt");
        assert_eq!(rows[0].username, "me");
        assert_eq!(rows[0].realname, "Real Name");
        assert!(rows[1].use_tls);
        assert_eq!(rows[1].port, 6697);
        assert_eq!(rows[1].password.as_deref(), Some("pw"));
    }

    #[test]
    fn text_keeps_other_lines_and_puts_servers_where_the_first_one_was() {
        let text = "; top\nserver -m old.org:6667 -i a b c@\n; middle\nserver -m older.org:6667 -i a b c@\n; end\n";
        let out = text_with_rows(text, &[row("new.org", 6667, false)]).unwrap();
        assert_eq!(
            out,
            "; top\nserver -m new.org:6667 -i ferric ferric_ ferric@ Ferric User\n; middle\n; end\n"
        );
    }

    #[test]
    fn rows_survive_a_round_trip() {
        let rows = vec![
            ServerRow {
                host: "2001:db8::1".into(),
                port: 6697,
                use_tls: true,
                password: Some("secret".into()),
                nick: "nick".into(),
                altnick: "nick_".into(),
                username: "ident".into(),
                realname: "Käthe Müller".into(),
            },
            row("irc.example.org", 6667, false),
        ];
        let text = text_with_rows("", &rows).unwrap();
        assert!(text.contains("server -m [2001:db8::1]:+6697 secret -i nick nick_ ident@ Käthe Müller\n"));
        assert_eq!(rows_from_text(&text), rows);
    }

    #[test]
    fn appends_when_there_is_no_server_line_and_removes_all_when_rows_are_empty() {
        let text = "; only a comment\n";
        let out = text_with_rows(text, &[row("a.org", 6667, false)]).unwrap();
        assert_eq!(out, "; only a comment\nserver -m a.org:6667 -i ferric ferric_ ferric@ Ferric User\n");
        let out = text_with_rows(&out, &[]).unwrap();
        assert_eq!(out, "; only a comment\n");
    }

    #[test]
    fn refuses_fields_that_would_not_read_back() {
        let mut bad = row("irc.example.org", 6667, false);
        bad.nick = "two words".into();
        assert!(text_with_rows("", &[bad]).is_err());
        assert!(check_row(&ServerRow { port: 0, ..row("a.org", 6667, false) }).is_err());
        assert!(check_row(&ServerRow { password: Some("pass word".into()), ..row("a.org", 6667, false) }).is_err());
        assert!(check_row(&ServerRow { username: "me@x".into(), ..row("a.org", 6667, false) }).is_err());
        assert!(check_row(&ServerRow { realname: "x\ny".into(), ..row("a.org", 6667, false) }).is_err());
        assert!(check_row(&row("a.org", 6667, false)).is_ok());
    }
}
