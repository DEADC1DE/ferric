//! Small helpers shared by the connection, state and loader code.

/// Channel prefixes used until the server announces CHANTYPES.
pub const DEFAULT_CHANTYPES: &str = "#&+!";

/// True when `name` starts with one of the default channel prefixes (`#`, `&`, `+`, `!`).
pub fn is_channel_name(name: &str) -> bool {
    is_channel_name_with(name, None)
}

/// Like `is_channel_name`, but uses the server's 005 CHANTYPES when it is known.
/// An absent or empty value falls back to the default prefixes.
pub fn is_channel_name_with(name: &str, chantypes: Option<&str>) -> bool {
    let set = chantypes.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_CHANTYPES);
    name.chars().next().is_some_and(|c| set.contains(c))
}

/// A message to a channel's operators or voiced users (STATUSMSG, `@#chan`, `+#chan`) is a message
/// to the channel. Returns the channel name without the status prefix; any other target is returned
/// as it is.
pub fn strip_status_prefix<'a>(target: &'a str, chantypes: Option<&str>) -> &'a str {
    let set = chantypes.filter(|s| !s.is_empty()).unwrap_or(DEFAULT_CHANTYPES);
    let mut chars = target.char_indices();
    match chars.next() {
        Some((_, c)) if "~&@%+".contains(c) => match chars.next() {
            Some((i, next)) if set.contains(next) => &target[i..],
            _ => target,
        },
        _ => target,
    }
}

/// Decodes bytes as UTF-8, falling back to Windows-1252 when they are not valid UTF-8.
pub fn decode_lossy(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => crate::script_files::decode_windows_1252(bytes),
    }
}

/// Splits `text` into pieces of at most `max_bytes` bytes, cutting only at char boundaries and
/// preferring the last space in the window (that space is dropped). Never returns empty pieces.
pub fn split_text(text: &str, max_bytes: usize) -> Vec<String> {
    let max_bytes = max_bytes.max(4);
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.len() <= max_bytes {
            out.push(rest.to_string());
            break;
        }
        let mut cut = max_bytes;
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        // A space exactly at `cut` also ends the piece cleanly.
        let space = if rest.as_bytes()[cut] == b' ' { Some(cut) } else { rest[..cut].rfind(' ') }
            .filter(|&i| i > 0);
        let (piece, next) = match space {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => (&rest[..cut], &rest[cut..]),
        };
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        rest = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_prefixes_are_stripped_only_in_front_of_a_channel() {
        assert_eq!(strip_status_prefix("@#chan", None), "#chan");
        assert_eq!(strip_status_prefix("%#x", Some("#")), "#x");
        assert_eq!(strip_status_prefix("#chan", None), "#chan");
        assert_eq!(strip_status_prefix("@Nick", None), "@Nick");
        assert_eq!(strip_status_prefix("+q", None), "+q");
    }

    #[test]
    fn channel_names() {
        for name in ["#a", "&a", "+a", "!a"] {
            assert!(is_channel_name(name), "{name}");
        }
        assert!(!is_channel_name("nick"));
        assert!(!is_channel_name(""));
        assert!(!is_channel_name("status"));
    }

    #[test]
    fn chantypes_override_defaults() {
        assert!(is_channel_name_with("#a", Some("#")));
        assert!(!is_channel_name_with("&a", Some("#")));
        assert!(is_channel_name_with("%a", Some("%#")));
        assert!(is_channel_name_with("&a", Some("")));
    }

    #[test]
    fn decode_falls_back_to_windows_1252() {
        assert_eq!(decode_lossy("h\u{e9}llo".as_bytes()), "h\u{e9}llo");
        assert_eq!(decode_lossy(&[b'h', 0xE9, 0x80]), "h\u{e9}\u{20AC}");
    }

    #[test]
    fn split_prefers_spaces_and_char_boundaries() {
        let pieces = split_text("aaaa bbbb cccc", 9);
        assert_eq!(pieces, vec!["aaaa bbbb", "cccc"]);
        let pieces = split_text("abcdefghij", 4);
        assert_eq!(pieces, vec!["abcd", "efgh", "ij"]);
        // Multi-byte chars are never cut.
        let text = "\u{e9}".repeat(10); // 20 bytes
        for piece in split_text(&text, 5) {
            assert!(piece.len() <= 5);
            assert!(piece.chars().all(|c| c == '\u{e9}'));
        }
        assert!(split_text("", 10).is_empty());
    }
}
