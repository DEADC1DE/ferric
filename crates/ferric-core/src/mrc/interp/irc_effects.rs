//! IRC effects of a script: lines the caller sends through the existing connection.
//!
//! The evaluator checks every field before it makes an effect, so a value from a
//! script or from an IRC event cannot add a second command or a parameter: no CR,
//! LF or NUL, no space or control character inside a name, no leading colon, and
//! no raw line past 510 bytes.

use crate::text::is_channel_name;

/// Longest IRC line body without CRLF, as the send path builds it.
pub const MAX_LINE_BYTES: usize = 510;

/// Longest nick, channel or target accepted. Servers cap these lower; this only bounds the input.
const MAX_NAME_BYTES: usize = 64;

/// Verbs `/raw` refuses: a script must not log in, authenticate or become an operator.
const RAW_DENIED: &[&str] = &["PASS", "AUTHENTICATE", "OPER"];

/// Which open windows a broadcast reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastScope {
    /// Channels of the network the script ran on.
    Channels,
    /// Channels of every connected network (`/amsg`, `/ame`).
    ChannelsEveryNetwork,
    /// Query windows of the network the script ran on.
    Queries,
}

/// What a broadcast sends to each window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastKind {
    Msg,
    Action,
    /// PART; the text, if any, is the reason.
    Part,
}

/// One line for the caller to send. Each field has passed the checks of this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrcEffect {
    /// A command with fixed parameters: `verb params... :trailing`. The verb is one this
    /// module creates; each parameter has passed `name_ok`, `channel_ok` or `nick_ok`.
    Line { verb: &'static str, params: Vec<String>, trailing: Option<String> },
    /// The same message, action or part in every window of a scope. The caller expands it
    /// into one effect per window.
    Broadcast { scope: BroadcastScope, kind: BroadcastKind, text: Option<String> },
    /// PRIVMSG to a nick or a comma list. The caller splits long text.
    Msg { target: String, text: String },
    /// NOTICE to a nick or a comma list.
    Notice { target: String, text: String },
    /// CTCP ACTION to a nick or channel (`/describe`, `/me`).
    Action { target: String, text: String },
    /// JOIN, with an optional channel key.
    Join { channel: String, key: Option<String> },
    /// PART, with an optional reason.
    Part { channel: String, reason: Option<String> },
    /// NICK to a new nick.
    Nick(String),
    /// MODE on a channel or nick. `modes` holds the sign, the letters and their parameters.
    Mode { target: String, modes: String },
    /// KICK from a channel, with an optional reason.
    Kick { channel: String, nick: String, reason: Option<String> },
    /// Opens the query window of a nick and makes it active. `text`, if given, is sent to it.
    Query { nick: String, text: Option<String> },
    /// WHOIS of a nick.
    Whois(String),
    /// A line sent as it is, after the verb check.
    Raw(String),
}

impl IrcEffect {
    /// The line for effects that are one command. `None` for messages, which the
    /// send path builds (it splits long text and encrypts it when a key exists).
    pub fn line(&self) -> Option<String> {
        Some(match self {
            IrcEffect::Msg { .. }
            | IrcEffect::Notice { .. }
            | IrcEffect::Action { .. }
            | IrcEffect::Query { .. }
            | IrcEffect::Broadcast { .. } => return None,
            IrcEffect::Line { verb, params, trailing } => {
                let mut out = verb.to_string();
                for p in params {
                    out.push(' ');
                    out.push_str(p);
                }
                if let Some(t) = trailing {
                    out.push_str(" :");
                    out.push_str(t);
                }
                out
            }
            IrcEffect::Join { channel, key: None } => format!("JOIN {channel}"),
            IrcEffect::Join { channel, key: Some(key) } => format!("JOIN {channel} {key}"),
            IrcEffect::Part { channel, reason: None } => format!("PART {channel}"),
            IrcEffect::Part { channel, reason: Some(r) } => format!("PART {channel} :{r}"),
            IrcEffect::Nick(nick) => format!("NICK {nick}"),
            IrcEffect::Mode { target, modes } => format!("MODE {target} {modes}"),
            IrcEffect::Kick { channel, nick, reason: None } => format!("KICK {channel} {nick}"),
            IrcEffect::Kick { channel, nick, reason: Some(r) } => format!("KICK {channel} {nick} :{r}"),
            IrcEffect::Whois(nick) => format!("WHOIS {nick}"),
            IrcEffect::Raw(line) => line.clone(),
        })
    }
}

/// True for a nick or channel name that cannot carry a second parameter.
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.starts_with(':')
        && name.bytes().all(|b| b > b' ' && b != 0x7f)
}

/// A comma list of nicks or channels, such as `alice,#chan`.
pub fn targets_ok(list: &str) -> bool {
    !list.is_empty() && list.split(',').all(name_ok)
}

pub fn channel_ok(name: &str) -> bool {
    is_channel_name(name) && name_ok(name) && !name.contains(',')
}

pub fn nick_ok(name: &str) -> bool {
    name_ok(name) && !name.contains(',')
}

/// A nick for `/query`: a nick, never a channel.
pub fn query_ok(name: &str) -> bool {
    nick_ok(name) && !is_channel_name(name)
}

/// The reason for refusing a raw line, or `None` when it may be sent.
pub fn raw_refusal(line: &str) -> Option<String> {
    if line.is_empty() {
        return Some("/raw needs a line to send".into());
    }
    if line.contains(['\r', '\n', '\0']) {
        return Some("/raw line contains CR, LF or NUL".into());
    }
    if line.len() > MAX_LINE_BYTES {
        return Some(format!("/raw line is longer than {MAX_LINE_BYTES} bytes"));
    }
    // A leading prefix (`:server VERB ...`) is skipped; the verb is the next word.
    let body = match line.strip_prefix(':') {
        Some(rest) => rest.split_once(' ').map_or("", |(_, after)| after),
        None => line,
    };
    let verb = body.split(' ').next().unwrap_or("").to_ascii_uppercase();
    if RAW_DENIED.contains(&verb.as_str()) {
        return Some(format!("/raw may not send {verb}"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_refuse_spaces_colons_and_controls() {
        assert!(name_ok("alice"));
        assert!(name_ok("#chan"));
        assert!(!name_ok(""));
        assert!(!name_ok(":alice"));
        assert!(!name_ok("al ice"));
        assert!(!name_ok("al\u{1}ice"));
        assert!(!name_ok("al\nice"));
        assert!(!name_ok(&"a".repeat(65)));
    }

    #[test]
    fn target_lists_and_channels() {
        assert!(targets_ok("alice,#chan"));
        assert!(!targets_ok("alice,,bob"));
        assert!(!targets_ok("alice,bo b"));
        assert!(channel_ok("#chan"));
        assert!(!channel_ok("alice"));
        assert!(!channel_ok("#a,#b"));
        assert!(nick_ok("alice"));
        assert!(!nick_ok("a,b"));
    }

    #[test]
    fn raw_refuses_login_verbs_and_bad_lines() {
        assert!(raw_refusal("PRIVMSG #a :hi").is_none());
        assert!(raw_refusal("pass secret").is_some());
        assert!(raw_refusal("AUTHENTICATE PLAIN").is_some());
        assert!(raw_refusal("OPER name pw").is_some());
        assert!(raw_refusal(":server OPER name pw").is_some());
        assert!(raw_refusal("").is_some());
        assert!(raw_refusal("MODE a\r\nQUIT").is_some());
        assert!(raw_refusal(&format!("PRIVMSG a :{}", "x".repeat(600))).is_some());
    }
}
