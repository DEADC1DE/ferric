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

/// Decodes bytes as UTF-8. Only the byte runs that are not valid UTF-8 are decoded as
/// Windows-1252, so one stray byte does not garble the valid UTF-8 characters around it.
pub fn decode_lossy(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            out.push_str(&crate::script_files::decode_windows_1252(chunk.invalid()));
        }
    }
    out
}

/// Converts ANSI SGR colour sequences (`ESC[...m`) into IRC colour and format codes, as the
/// bots' output uses them: `0` resets, `1` is bold, `91`/`92`/`93` are red, green and yellow.
/// Any other escape sequence is removed, so no raw control characters reach the chat.
pub fn ansi_to_irc(text: &str) -> String {
    // Colour numbers for ANSI 30-37 / 40-47 (normal) and 90-97 / 100-107 (bright).
    const NORMAL: [u8; 8] = [1, 5, 3, 7, 2, 6, 10, 15];
    const BRIGHT: [u8; 8] = [14, 4, 9, 8, 12, 13, 11, 0];
    // Intensity remap of an already active normal foreground colour.
    fn intense(fg: u8) -> u8 {
        match fg {
            1 => 14,
            5 => 4,
            3 => 9,
            2 => 12,
            10 => 11,
            15 => 0,
            7 => 8,
            6 => 13,
            other => other,
        }
    }

    let mut out = String::with_capacity(text.len());
    let mut fg: Option<u8> = None;
    let mut bg: Option<u8> = None;
    let mut intensity = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            continue; // a lone ESC carries no text; the character after it stays
        }
        chars.next();
        let mut params = String::new();
        let mut final_byte = None;
        for p in chars.by_ref() {
            if ('@'..='~').contains(&p) {
                final_byte = Some(p);
                break;
            }
            params.push(p);
        }
        if final_byte != Some('m') {
            continue;
        }
        let mut colour_changed = false;
        for param in params.split(';') {
            let code: u8 = match param.parse() {
                Ok(n) => n,
                Err(_) if param.is_empty() => 0,
                Err(_) => continue,
            };
            match code {
                0 => {
                    out.push('\u{0f}');
                    fg = None;
                    bg = None;
                    intensity = false;
                }
                1 => {
                    intensity = true;
                    if let Some(f) = fg {
                        fg = Some(intense(f));
                        colour_changed = true;
                    }
                }
                22 => intensity = false,
                3 => out.push('\u{1d}'),
                4 | 21 => out.push('\u{1f}'),
                7 => out.push('\u{16}'),
                9 => out.push('\u{1e}'),
                30..=37 => {
                    let idx = (code - 30) as usize;
                    fg = Some(if intensity { BRIGHT[idx] } else { NORMAL[idx] });
                    colour_changed = true;
                }
                90..=97 => { fg = Some(BRIGHT[(code - 90) as usize]); colour_changed = true; }
                40..=47 => { bg = Some(NORMAL[(code - 40) as usize]); colour_changed = true; }
                100..=107 => { bg = Some(NORMAL[(code - 100) as usize]); colour_changed = true; }
                _ => {}
            }
        }
        if colour_changed {
            match (fg, bg) {
                (None, None) => out.push('\u{03}'),
                (f, None) => out.push_str(&format!("\u{03}{:02}", f.unwrap_or(99))),
                (f, Some(b)) => out.push_str(&format!("\u{03}{:02},{:02}", f.unwrap_or(99), b)),
            }
        }
    }
    out
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
    fn ansi_colours_become_irc_codes() {
        // The stats bot's crown: yellow ♔, then reset.
        assert_eq!(ansi_to_irc("ago\u{1b}[93m\u{2654}\u{1b}[0m)"), "ago\u{3}08\u{2654}\u{f})");
        assert_eq!(ansi_to_irc("\u{1b}[92m▲\u{1b}[0m 1"), "\u{3}09▲\u{f} 1");
        assert_eq!(ansi_to_irc("\u{1b}[1;91mX"), "\u{3}04X");
    }

    #[test]
    fn ansi_all_colours_and_backgrounds() {
        assert_eq!(ansi_to_irc("\u{1b}[31mR"), "\u{3}05R");
        assert_eq!(ansi_to_irc("\u{1b}[30mK"), "\u{3}01K");
        assert_eq!(ansi_to_irc("\u{1b}[97mW"), "\u{3}00W");
        assert_eq!(ansi_to_irc("\u{1b}[37mG"), "\u{3}15G");
        assert_eq!(ansi_to_irc("\u{1b}[31;44mZ"), "\u{3}05,02Z");
        assert_eq!(ansi_to_irc("\u{1b}[41mQ"), "\u{3}99,05Q");
        assert_eq!(ansi_to_irc("\u{1b}[39mD"), "D"); // 39 and 49 change nothing
    }

    #[test]
    fn ansi_unknown_sequences_are_removed() {
        assert_eq!(ansi_to_irc("a\u{1b}[2Kb"), "ab");
        assert_eq!(ansi_to_irc("a\u{1b}c"), "ac");
        assert_eq!(ansi_to_irc("plain"), "plain");
    }

    #[test]
    fn decode_keeps_utf8_next_to_a_stray_byte() {
        // U+2654 WHITE CHESS KING (E2 99 94) next to a lone Windows-1252 byte (0xE9).
        let bytes = [b'a', 0xE2, 0x99, 0x94, b' ', 0xE9];
        assert_eq!(decode_lossy(&bytes), "a\u{2654} \u{e9}");
        assert_eq!(decode_lossy("\u{2654}".as_bytes()), "\u{2654}");
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
