//! Statement classification and block building.
//!
//! All nesting is bounded: block frames by `MAX_BLOCK_DEPTH`, inline chains
//! (`if (a) if (b) ...`) by `MAX_INLINE_DEPTH`. Input beyond those limits is
//! flattened, never recursed into, so adversarial files cannot overflow the
//! stack.

use super::ast::{Statement, StatementKind};
use super::lexer::{tokenize, Tok};

pub(crate) const MAX_BLOCK_DEPTH: usize = 32;
pub(crate) const MAX_INLINE_DEPTH: usize = 16;

fn is_ws_char(c: char) -> bool {
    c.is_whitespace()
}

/// Index of the `)` matching the `(` at the start of `s`.
fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Finds a standalone `else` word at parenthesis depth 0.
fn find_else(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'e' | b'E' if depth == 0 => {
                let before_ok = i == 0 || b[i - 1].is_ascii_whitespace();
                let end = i + 4;
                if before_ok
                    && end <= b.len()
                    && b[i..end].eq_ignore_ascii_case(b"else")
                    && (end == b.len() || b[end].is_ascii_whitespace() || b[end] == b'{')
                {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn kind_for(name: &str) -> StatementKind {
    match name {
        "if" => StatementKind::If,
        "elseif" => StatementKind::ElseIf,
        "else" => StatementKind::Else,
        "while" => StatementKind::While,
        "return" => StatementKind::Return,
        "break" => StatementKind::Break,
        "continue" => StatementKind::Continue,
        "goto" => StatementKind::Goto,
        "var" => StatementKind::Var,
        "set" => StatementKind::Set,
        "unset" => StatementKind::Unset,
        "inc" => StatementKind::Inc,
        "dec" => StatementKind::Dec,
        _ => StatementKind::Command,
    }
}

/// Classifies one statement text (no `|`, no braces) and appends the result
/// (and, for `if (...) a else b`, the trailing `else` statement) to `out`.
pub(crate) fn classify(
    text: &str,
    line: usize,
    depth: usize,
    inline: bool,
    unparen: &mut usize,
    out: &mut Vec<Statement>,
) {
    let t = text.trim();
    if t.is_empty() {
        return;
    }
    if let Some(label) = t.strip_prefix(':') {
        let name = label.split(is_ws_char).next().unwrap_or("").to_lowercase();
        let mut st = Statement::new(line, StatementKind::Label, name, String::new());
        st.inline = inline;
        out.push(st);
        return;
    }

    // Prefixes: `/`, `//` and the silent `.`.
    let mut s = t;
    let mut silent = false;
    let mut n = 0;
    while n < 4 {
        if let Some(r) = s.strip_prefix('/') {
            s = r;
        } else if let Some(r) = s.strip_prefix('.') {
            if r.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') {
                s = r;
                silent = true;
            } else {
                break;
            }
        } else {
            break;
        }
        n += 1;
    }
    if s.is_empty() {
        return;
    }

    let mut end = s
        .find(|c: char| c.is_whitespace() || matches!(c, '(' | '{' | '}'))
        .unwrap_or(s.len());
    if end == 0 {
        end = s.chars().next().map_or(0, |c| c.len_utf8());
    }
    let token = &s[..end];
    let rest = s[end..].trim_start();
    let mut name = token.to_lowercase();
    let first = token.chars().next().unwrap_or(' ');

    let mut kind = if matches!(first, '$' | '%' | '&') {
        StatementKind::Eval
    } else {
        kind_for(&name)
    };

    let mut rest_after = rest;
    if kind == StatementKind::Else {
        let r = rest;
        if r.len() >= 2
            && r.is_char_boundary(2)
            && r[..2].eq_ignore_ascii_case("if")
            && (r.len() == 2 || r[2..].starts_with(|c: char| c.is_whitespace() || c == '('))
        {
            kind = StatementKind::ElseIf;
            name = "elseif".to_string();
            rest_after = r[2..].trim_start();
        }
    }

    let mut st = Statement::new(line, kind, name, String::new());
    st.silent = silent;
    st.inline = inline;

    match kind {
        StatementKind::If | StatementKind::ElseIf | StatementKind::While => {
            if depth >= MAX_INLINE_DEPTH {
                st.args = rest_after.to_string();
                out.push(st);
                return;
            }
            let mut tail = "";
            if rest_after.starts_with('(') {
                match matching_paren(rest_after) {
                    Some(close) => {
                        st.args = rest_after[..=close].to_string();
                        tail = rest_after[close + 1..].trim();
                    }
                    None => st.args = rest_after.to_string(),
                }
            } else {
                st.args = rest_after.to_string();
                if !rest_after.is_empty() {
                    *unparen += 1;
                }
            }
            let mut else_part: Option<&str> = None;
            let mut inline_part = tail;
            if kind != StatementKind::While {
                if let Some(idx) = find_else(tail) {
                    inline_part = tail[..idx].trim();
                    else_part = Some(tail[idx..].trim());
                }
            }
            if !inline_part.is_empty() {
                classify(inline_part, line, depth + 1, true, unparen, &mut st.body);
            }
            out.push(st);
            if let Some(e) = else_part {
                classify(e, line, depth + 1, false, unparen, out);
            }
        }
        StatementKind::Else => {
            if depth < MAX_INLINE_DEPTH && !rest.is_empty() {
                classify(rest, line, depth + 1, true, unparen, &mut st.body);
            } else {
                st.args = rest.to_string();
            }
            out.push(st);
        }
        StatementKind::Eval => {
            // Keep the whole text: the leading token and what follows.
            st.args = rest.to_string();
            out.push(st);
        }
        _ => {
            st.args = rest.to_string();
            out.push(st);
        }
    }
}

fn deepest_mut(s: &mut Statement) -> &mut Statement {
    let mut cur = s;
    loop {
        let descend = cur.body.last().map_or(false, |l| l.inline);
        if !descend {
            return cur;
        }
        cur = cur.body.last_mut().unwrap();
    }
}

fn deepest_body_empty(s: &Statement) -> bool {
    let mut cur = s;
    while let Some(last) = cur.body.last() {
        if last.inline {
            cur = last;
        } else {
            return false;
        }
    }
    cur.body.is_empty()
}

/// Builds a statement tree from logical lines fed one by one.
pub(crate) struct BodyBuilder {
    stack: Vec<Vec<Statement>>,
    /// Open braces seen and not yet closed (including the item brace).
    pub braces: usize,
    overflow: usize,
    item_brace: bool,
    allow_item_open: bool,
    pub depth_exceeded: bool,
    pub unbalanced: Vec<usize>,
    pub comments: usize,
    pub unparen: usize,
}

impl BodyBuilder {
    pub fn new(allow_item_open: bool) -> Self {
        BodyBuilder {
            stack: vec![Vec::new()],
            braces: 0,
            overflow: 0,
            item_brace: false,
            allow_item_open,
            depth_exceeded: false,
            unbalanced: Vec::new(),
            comments: 0,
            unparen: 0,
        }
    }

    pub fn feed(&mut self, line: usize, text: &str) {
        let (toks, comment) = tokenize(text);
        if comment {
            self.comments += 1;
        }
        let mut prev_stmt = false;
        for tok in toks {
            match tok {
                Tok::Stmt(s) => {
                    let top = self.stack.last_mut().unwrap();
                    classify(&s, line, 0, false, &mut self.unparen, top);
                    prev_stmt = true;
                }
                Tok::Open => {
                    self.open(line, prev_stmt);
                    prev_stmt = false;
                }
                Tok::Close => {
                    self.close(line);
                    prev_stmt = false;
                }
            }
        }
    }

    fn open(&mut self, line: usize, trailing: bool) {
        self.braces += 1;
        if self.braces == 1
            && self.allow_item_open
            && self.stack.len() == 1
            && self.stack[0].is_empty()
        {
            self.item_brace = true;
            return;
        }
        if self.stack.len() >= MAX_BLOCK_DEPTH {
            self.overflow += 1;
            self.depth_exceeded = true;
            return;
        }
        let top = self.stack.last_mut().unwrap();
        let owned = match top.last() {
            Some(last) => (trailing) || (last.kind.is_control() && deepest_body_empty(last)),
            None => false,
        };
        if !owned {
            top.push(Statement::new(
                line,
                StatementKind::Block,
                String::new(),
                String::new(),
            ));
        }
        self.stack.push(Vec::new());
    }

    fn close(&mut self, line: usize) {
        if self.braces == 0 {
            self.unbalanced.push(line);
            return;
        }
        self.braces -= 1;
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }
        if self.item_brace && self.braces == 0 {
            return;
        }
        if self.stack.len() <= 1 {
            return;
        }
        let children = self.stack.pop().unwrap();
        let parent = self.stack.last_mut().unwrap();
        match parent.last_mut() {
            Some(last) => deepest_mut(last).body.extend(children),
            None => parent.extend(children),
        }
    }

    /// Flushes unterminated frames and returns the root statements.
    pub fn finish(mut self) -> Vec<Statement> {
        while self.stack.len() > 1 {
            let children = self.stack.pop().unwrap();
            let parent = self.stack.last_mut().unwrap();
            match parent.last_mut() {
                Some(last) => deepest_mut(last).body.extend(children),
                None => parent.extend(children),
            }
        }
        self.stack.pop().unwrap_or_default()
    }
}
