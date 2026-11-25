//! IRC formatting & color code parser (`\x03`, `\x02`, `\x1F`, `\x1D`, `\x1E`, `\x16`, `\x0F`).

use serde::{Deserialize, Serialize};

/// Most styled pieces kept per message. Past this, text joins the last piece and its style is
/// kept; the rest of the message's style changes are not, so one line cannot multiply memory.
pub const MAX_SPANS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyledSpan {
    pub text: String,
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub reverse: bool,
}

#[derive(Default, Debug, Clone)]
struct FormatState {
    fg: Option<u8>,
    bg: Option<u8>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    reverse: bool,
}

pub fn parse_irc_formatting(raw: &str) -> Vec<StyledSpan> {
    let mut spans: Vec<StyledSpan> = Vec::new();
    let mut current_text = String::new();
    let mut state = FormatState::default();

    let mut chars = raw.char_indices().peekable();

    macro_rules! flush {
        () => {
            if !current_text.is_empty() && spans.len() >= MAX_SPANS {
                if let Some(last) = spans.last_mut() {
                    last.text.push_str(&std::mem::take(&mut current_text));
                }
            } else if !current_text.is_empty() {
                spans.push(StyledSpan {
                    text: std::mem::take(&mut current_text),
                    fg: state.fg,
                    bg: state.bg,
                    bold: state.bold,
                    italic: state.italic,
                    underline: state.underline,
                    strikethrough: state.strikethrough,
                    reverse: state.reverse,
                });
            }
        };
    }

    while let Some((_, ch)) = chars.next() {
        match ch {
            '\x02' => {
                flush!();
                state.bold = !state.bold;
            }
            '\x1D' => {
                flush!();
                state.italic = !state.italic;
            }
            '\x1F' => {
                flush!();
                state.underline = !state.underline;
            }
            '\x1E' => {
                flush!();
                state.strikethrough = !state.strikethrough;
            }
            '\x16' => {
                flush!();
                state.reverse = !state.reverse;
            }
            '\x0F' => {
                flush!();
                state = FormatState::default();
            }
            '\x03' => {
                flush!();
                // Parse optional fg and bg: \x03[0-9]{1,2}(,[0-9]{1,2})?
                let fg_digits = read_up_to_2_digits(&mut chars);
                if let Some(fg) = fg_digits {
                    state.fg = Some(fg);

                    // Check for comma followed by digits: \x03fg,bg
                    if let Some(&(_, ',')) = chars.peek() {
                        let mut lookahead = chars.clone();
                        lookahead.next(); // skip comma
                        if let Some(&(_, d)) = lookahead.peek() {
                            if d.is_ascii_digit() {
                                chars.next(); // consume comma
                                state.bg = read_up_to_2_digits(&mut chars);
                            }
                        }
                    }
                } else {
                    // Bare \x03 resets colors
                    state.fg = None;
                    state.bg = None;
                }
            }
            other => {
                current_text.push(other);
            }
        }
    }

    flush!();
    spans
}

fn read_up_to_2_digits(
    chars: &mut std::iter::Peekable<std::str::CharIndices>,
) -> Option<u8> {
    let mut num_str = String::new();
    for _ in 0..2 {
        if let Some(&(_, d)) = chars.peek() {
            if d.is_ascii_digit() {
                num_str.push(d);
                chars.next();
                continue;
            }
        }
        break;
    }

    if num_str.is_empty() {
        None
    } else {
        num_str.parse::<u8>().ok()
    }
}

/// Standard IRC 16-color hex palette
pub fn irc_color_to_hex(color_index: u8) -> &'static str {
    match color_index % 16 {
        0 => "#ffffff",  // 0: White
        1 => "#000000",  // 1: Black
        2 => "#00007f",  // 2: Blue (Navy)
        3 => "#009300",  // 3: Green
        4 => "#ff0000",  // 4: Red
        5 => "#7f0000",  // 5: Brown / Maroon
        6 => "#9c009c",  // 6: Purple
        7 => "#fc7f00",  // 7: Orange
        8 => "#ffff00",  // 8: Yellow
        9 => "#00fc00",  // 9: Light Green
        10 => "#009393", // 10: Teal
        11 => "#00ffff", // 11: Light Cyan
        12 => "#0000fc", // 12: Light Blue
        13 => "#ff00ff", // 13: Pink / Fuchsia
        14 => "#7f7f7f", // 14: Grey
        15 => "#d2d2d2", // 15: Light Grey
        _ => "#ffffff",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_irc_colors_parsing() {
        let raw = "\x0303Green text \x02Bold green\x02\x0304,01Red on black\x0F Normal text";
        let spans = parse_irc_formatting(raw);

        assert_eq!(spans[0].text, "Green text ");
        assert_eq!(spans[0].fg, Some(3));
        assert_eq!(spans[0].bg, None);
        assert!(!spans[0].bold);

        assert_eq!(spans[1].text, "Bold green");
        assert_eq!(spans[1].fg, Some(3));
        assert!(spans[1].bold);

        assert_eq!(spans[2].text, "Red on black");
        assert_eq!(spans[2].fg, Some(4));
        assert_eq!(spans[2].bg, Some(1));

        assert_eq!(spans[3].text, " Normal text");
        assert_eq!(spans[3].fg, None);
        assert_eq!(spans[3].bg, None);
        assert!(!spans[3].bold);
    }

    #[test]
    fn test_irc_comma_in_text() {
        let raw = "\x0303Hello, world!";
        let spans = parse_irc_formatting(raw);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "Hello, world!");
        assert_eq!(spans[0].fg, Some(3));
        assert_eq!(spans[0].bg, None);
    }

    #[test]
    fn style_changes_are_capped_and_no_text_is_lost() {
        let raw: String = (0..200).map(|i| format!("\x03{}x", i % 16)).collect();
        let spans = parse_irc_formatting(&raw);
        assert_eq!(spans.len(), MAX_SPANS);
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "x".repeat(200));
    }
}

#[cfg(test)]
mod ansi_chain_tests {
    use super::*;

    #[test]
    fn converted_crown_line_has_yellow_crown_span() {
        let raw = crate::text::ansi_to_irc("ago\u{1b}[93m\u{2654}\u{1b}[0m)");
        let spans = parse_irc_formatting(&raw);
        let crown = spans.iter().find(|s| s.text.contains('\u{2654}')).expect("crown span");
        assert_eq!(crown.fg, Some(8));
        assert!(spans.iter().all(|s| !s.text.contains('\u{1b}')));
    }
}
