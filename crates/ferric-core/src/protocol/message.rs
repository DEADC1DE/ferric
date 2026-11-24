//! RFC 1459, RFC 2812, and IRCv3 message parser & serializer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrcPrefix {
    pub nick: String,
    pub user: Option<String>,
    pub host: Option<String>,
}

impl IrcPrefix {
    pub fn parse(s: &str) -> Self {
        if let Some((nick, rest)) = s.split_once('!') {
            if let Some((user, host)) = rest.split_once('@') {
                Self {
                    nick: nick.to_string(),
                    user: Some(user.to_string()),
                    host: Some(host.to_string()),
                }
            } else {
                Self {
                    nick: nick.to_string(),
                    user: Some(rest.to_string()),
                    host: None,
                }
            }
        } else if let Some((nick, host)) = s.split_once('@') {
            Self {
                nick: nick.to_string(),
                user: None,
                host: Some(host.to_string()),
            }
        } else {
            Self {
                nick: s.to_string(),
                user: None,
                host: None,
            }
        }
    }
}

/// Unescapes an IRCv3 tag value: `\s` is a space, `\:` a semicolon, `\\` a backslash, `\r` and
/// `\n` line breaks. Any other escaped character stands for itself; a trailing backslash is dropped.
pub fn unescape_tag_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some(':') => out.push(';'),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrcMessage {
    pub tags: HashMap<String, String>,
    pub prefix: Option<IrcPrefix>,
    pub command: String,
    pub params: Vec<String>,
}

impl IrcMessage {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }

        let mut remainder = line;
        let mut tags = HashMap::new();

        // 1. Tags: @tag1=val;tag2 :...
        if remainder.starts_with('@') {
            let (tag_part, rest) = remainder[1..].split_once(' ')?;
            for tag in tag_part.split(';') {
                if let Some((k, v)) = tag.split_once('=') {
                    tags.insert(k.to_string(), unescape_tag_value(v));
                } else {
                    tags.insert(tag.to_string(), String::new());
                }
            }
            remainder = rest.trim_start();
        }

        // 2. Prefix: :nick!user@host ...
        let mut prefix = None;
        if remainder.starts_with(':') {
            let (prefix_str, rest) = remainder[1..].split_once(' ')?;
            prefix = Some(IrcPrefix::parse(prefix_str));
            remainder = rest.trim_start();
        }

        // 3. Command and params
        let mut params = Vec::new();
        while !remainder.is_empty() {
            if remainder.starts_with(':') {
                // Trailing parameter (rest of line)
                params.push(remainder[1..].to_string());
                break;
            }

            match remainder.split_once(' ') {
                Some((param, rest)) => {
                    params.push(param.to_string());
                    remainder = rest.trim_start();
                }
                None => {
                    params.push(remainder.to_string());
                    break;
                }
            }
        }

        if params.is_empty() {
            return None;
        }

        let command = params.remove(0).to_uppercase();

        Some(Self {
            tags,
            prefix,
            command,
            params,
        })
    }

    pub fn to_raw_line(&self) -> String {
        let mut out = String::new();

        // Tags
        if !self.tags.is_empty() {
            out.push('@');
            let tag_strs: Vec<String> = self
                .tags
                .iter()
                .map(|(k, v)| {
                    if v.is_empty() {
                        k.clone()
                    } else {
                        format!("{}={}", k, v)
                    }
                })
                .collect();
            out.push_str(&tag_strs.join(";"));
            out.push(' ');
        }

        // Prefix
        if let Some(ref p) = self.prefix {
            out.push(':');
            out.push_str(&p.nick);
            if let Some(ref u) = p.user {
                out.push('!');
                out.push_str(u);
            }
            if let Some(ref h) = p.host {
                out.push('@');
                out.push_str(h);
            }
            out.push(' ');
        }

        out.push_str(&self.command);

        for (i, param) in self.params.iter().enumerate() {
            out.push(' ');
            if i == self.params.len() - 1 && (param.contains(' ') || param.starts_with(':') || param.is_empty()) {
                out.push(':');
            }
            out.push_str(param);
        }

        out.push_str("\r\n");
        out
    }
}

#[cfg(test)]
mod tag_tests {
    use super::{unescape_tag_value, IrcMessage};

    #[test]
    fn tag_values_are_unescaped() {
        assert_eq!(unescape_tag_value(r"a\sb\:c\\d\r\n"), "a b;c\\d\r\n");
        assert_eq!(unescape_tag_value(r"x\"), "x");
        assert_eq!(unescape_tag_value(r"\q"), "q");
    }

    #[test]
    fn a_server_time_tag_is_read_unescaped() {
        let msg = IrcMessage::parse("@time=2026-11-22T10:00:00.000Z;msgid=abc :nick!u@h PRIVMSG #c :hi").unwrap();
        assert_eq!(msg.tags.get("time").map(String::as_str), Some("2026-11-22T10:00:00.000Z"));
        assert_eq!(msg.tags.get("msgid").map(String::as_str), Some("abc"));
    }
}
