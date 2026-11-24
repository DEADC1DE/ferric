//! Byte decoding, line splitting and the statement tokenizer.
//!
//! Rules implemented here (see the module docs in `mod.rs` for sources):
//! * lines end with LF or CRLF;
//! * a line ending in `$&` is joined with the next line;
//! * a line starting with `;` is a comment, `/* ... */` comments span lines;
//! * a statement starting with `;` (for example after `{`) comments out the
//!   rest of the line;
//! * `|` separates commands, except inside the parentheses of an identifier
//!   call or of an `if`/`elseif`/`while` condition, and `||` is the OR
//!   operator, never a separator;
//! * `{` and `}` delimit blocks when they stand on their own (surrounded by
//!   whitespace or line ends) outside such parentheses.

use super::ast::Encoding;

/// A logical (code) line: physical lines joined by `$&`, trimmed.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    /// First physical line, 1-based.
    pub start: usize,
    /// Last physical line, 1-based.
    pub end: usize,
    pub text: String,
}

#[derive(Debug, Default)]
pub(crate) struct LineScan {
    pub lines: Vec<Line>,
    pub physical: usize,
    pub blank: usize,
    pub comments: usize,
    pub continuations: usize,
}

const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Decodes file bytes. UTF-16 (with BOM) and UTF-8 (BOM optional) are
/// recognised; anything else is read as Windows-1252, which cannot fail.
pub(crate) fn decode(bytes: &[u8]) -> (String, Encoding) {
    if bytes.len() >= 2 && (bytes[..2] == [0xFF, 0xFE] || bytes[..2] == [0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return (String::from_utf16_lossy(&units), Encoding::Utf16);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => (s.to_string(), Encoding::Utf8),
        Err(_) => {
            let s: String = body
                .iter()
                .map(|&b| match b {
                    0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
                    _ => b as char,
                })
                .collect();
            (s, Encoding::Windows1252)
        }
    }
}

fn ends_with_continuation(s: &str) -> bool {
    s.trim_end().ends_with("$&")
}

/// Splits decoded text into logical code lines.
pub(crate) fn scan_lines(src: &str) -> LineScan {
    let mut segs: Vec<&str> = src.split('\n').collect();
    if segs.last().map_or(false, |s| s.is_empty()) {
        segs.pop();
    }
    let mut scan = LineScan {
        physical: segs.len(),
        ..LineScan::default()
    };
    let mut in_block_comment = false;
    let mut i = 0;
    while i < segs.len() {
        let start = i + 1;
        let mut end = start;
        let mut text = segs[i].strip_suffix('\r').unwrap_or(segs[i]).to_string();
        let first = text.trim();
        let joinable = !in_block_comment && !first.starts_with(';') && !first.starts_with("/*");
        if joinable {
            while ends_with_continuation(&text) && i + 1 < segs.len() {
                let keep = text.trim_end().len() - 2;
                text.truncate(keep);
                i += 1;
                let next = segs[i].strip_suffix('\r').unwrap_or(segs[i]);
                text.push_str(next.trim_start());
                scan.continuations += 1;
                end = i + 1;
            }
        }
        i += 1;

        let t = text.trim();
        if in_block_comment {
            scan.comments += 1;
            if t.starts_with("*/") || t.ends_with("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if t.is_empty() {
            scan.blank += 1;
            continue;
        }
        if t.starts_with("/*") {
            scan.comments += 1;
            if !(t.len() >= 4 && t.ends_with("*/")) {
                in_block_comment = true;
            }
            continue;
        }
        if t.starts_with(';') {
            scan.comments += 1;
            continue;
        }
        scan.lines.push(Line {
            start,
            end,
            text: t.to_string(),
        });
    }
    scan
}

/// Token of a tokenized logical line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Stmt(String),
    Open,
    Close,
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == 0x0B || b == 0x0C || b == b'\r'
}

fn starts_with_ci(b: &[u8], kw: &str) -> bool {
    b.len() >= kw.len() && b[..kw.len()].eq_ignore_ascii_case(kw.as_bytes())
}

fn kw_boundary(b: &[u8], n: usize) -> bool {
    n >= b.len() || is_ws(b[n]) || b[n] == b'('
}

/// If `b` starts with `if`, `elseif`, `while` or `else if`, returns the
/// length of that keyword prefix.
fn cond_keyword_end(b: &[u8]) -> Option<usize> {
    for kw in ["elseif", "while", "if"] {
        if starts_with_ci(b, kw) && kw_boundary(b, kw.len()) {
            return Some(kw.len());
        }
    }
    if starts_with_ci(b, "else") && b.len() > 4 && is_ws(b[4]) {
        let mut j = 4;
        while j < b.len() && is_ws(b[j]) {
            j += 1;
        }
        if starts_with_ci(&b[j..], "if") && kw_boundary(b, j + 2) {
            return Some(j + 2);
        }
    }
    None
}

/// True when the `(` at `i` belongs to an identifier call (`$name(`, `$(`,
/// `$+(`) or directly follows an `if`/`elseif`/`while` keyword.
fn owned_open(b: &[u8], i: usize, seg_start: usize, cond_end: Option<usize>) -> bool {
    if let Some(e) = cond_end {
        if e <= i && b[e..i].iter().all(|&c| is_ws(c)) {
            return true;
        }
    }
    if i == 0 {
        return false;
    }
    if b[i - 1] == b'$' {
        return true;
    }
    if b[i - 1] == b'+' && i >= 2 && b[i - 2] == b'$' {
        return true;
    }
    let mut j = i;
    while j > seg_start && (b[j - 1].is_ascii_alphanumeric() || b[j - 1] == b'_') {
        j -= 1;
    }
    if j == i {
        return false;
    }
    if j > seg_start && b[j - 1] == b'!' {
        j -= 1;
    }
    j > seg_start && b[j - 1] == b'$'
}

fn open_brace_ok(b: &[u8], i: usize) -> bool {
    let before = i == 0 || is_ws(b[i - 1]) || matches!(b[i - 1], b')' | b'{' | b'}');
    let after = i + 1 >= b.len() || is_ws(b[i + 1]) || matches!(b[i + 1], b'{' | b'}' | b'|');
    before && after
}

fn close_brace_ok(b: &[u8], i: usize) -> bool {
    let before = i == 0 || is_ws(b[i - 1]) || matches!(b[i - 1], b'{' | b'}');
    let after = i + 1 >= b.len() || is_ws(b[i + 1]) || matches!(b[i + 1], b'{' | b'}' | b'|');
    before && after
}

/// Splits one logical line into statements and block delimiters.
/// Returns the tokens and whether a statement-initial `;` comment was seen.
pub(crate) fn tokenize(text: &str) -> (Vec<Tok>, bool) {
    let b = text.as_bytes();
    let n = b.len();
    let mut toks = Vec::new();
    let mut had_comment = false;
    let mut seg_start = 0usize;
    let mut i = 0usize;
    let mut depth: u32 = 0;
    let mut at_start = true;
    let mut cond_end: Option<usize> = None;

    fn flush(text: &str, from: usize, to: usize, toks: &mut Vec<Tok>) {
        if from < to {
            let s = text[from..to].trim();
            if !s.is_empty() {
                toks.push(Tok::Stmt(s.to_string()));
            }
        }
    }

    while i < n {
        if at_start {
            while i < n && is_ws(b[i]) {
                i += 1;
            }
            seg_start = i;
            if i >= n {
                break;
            }
            if b[i] == b';' {
                had_comment = true;
                seg_start = n;
                break;
            }
            cond_end = cond_keyword_end(&b[i..]).map(|e| i + e);
            at_start = false;
        }
        match b[i] {
            b'(' => {
                if depth > 0 {
                    depth = depth.saturating_add(1);
                } else if owned_open(b, i, seg_start, cond_end) {
                    depth = 1;
                    cond_end = None;
                } else {
                    cond_end = None;
                }
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'|' if depth == 0 => {
                if i + 1 < n && b[i + 1] == b'|' {
                    i += 2;
                    continue;
                }
                // A separator is a bar between spaces; a bar that touches
                // a word is text.
                if !(i > 0 && is_ws(b[i - 1]) && i + 1 < n && is_ws(b[i + 1])) {
                    i += 1;
                    continue;
                }
                flush(text, seg_start, i, &mut toks);
                i += 1;
                at_start = true;
            }
            b'{' if depth == 0 && open_brace_ok(b, i) => {
                flush(text, seg_start, i, &mut toks);
                toks.push(Tok::Open);
                i += 1;
                at_start = true;
            }
            b'}' if depth == 0 && close_brace_ok(b, i) => {
                flush(text, seg_start, i, &mut toks);
                toks.push(Tok::Close);
                i += 1;
                at_start = true;
            }
            _ => i += 1,
        }
    }
    if !at_start {
        flush(text, seg_start, n, &mut toks);
    }
    (toks, had_comment)
}
