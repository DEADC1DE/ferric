//! Item level parser: aliases, events, raw/ctcp events, menus, dialogs,
//! groups and file-level commands. Bodies are handed to `body.rs`.

use super::ast::*;
use super::body::BodyBuilder;
use super::lexer::{tokenize, Line, LineScan, Tok};

pub(crate) struct Parsed {
    pub items: Vec<Item>,
    pub unclassified: Vec<Unclassified>,
    pub inline_comments: usize,
    pub unparenthesised: usize,
}

struct Parser {
    lines: Vec<Line>,
    pos: usize,
    unclassified: Vec<Unclassified>,
    inline_comments: usize,
    unparen: usize,
}

/// Number of colon separated header fields between the event name and the
/// commands, per event. Unknown events default to one field.
fn event_field_count(event: &str) -> usize {
    match event {
        "load" | "start" | "unload" | "exit" | "connect" | "disconnect" | "connectfail"
        | "logon" | "nick" | "quit" | "ping" | "pong" | "usermode" | "appactive" => 0,
        "text" | "action" | "notice" | "open" | "hotlink" | "agent" => 2,
        "dialog" => 3,
        _ => 1,
    }
}

fn split_word(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn split_levels(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn first_token_has_colon(s: &str) -> bool {
    s.split_whitespace()
        .next()
        .map_or(false, |t| t.contains(':'))
}

fn is_ini_entry(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || !b[0].is_ascii_alphabetic() {
        return false;
    }
    let mut i = 1;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    i > 1 && i < b.len() && b[i] == b'='
}

fn command_start(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '/' | '.' | '$' | '%' | ':' | '&' | '@' | '!' | '-' | '+')
}

/// First `:` at parenthesis depth 0.
fn top_level_colon(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

impl Parser {
    fn unc(&mut self, line: usize, kind: UnclassifiedKind) {
        self.unclassified.push(Unclassified { line, kind });
    }

    /// Consumes the body of an item. `inline` is the text after the header on
    /// the header line. Returns the statements and the last line consumed.
    fn consume_body(
        &mut self,
        header_line: usize,
        header_end: usize,
        inline: &str,
        allow_item_open: bool,
    ) -> (Vec<Statement>, usize) {
        let mut b = BodyBuilder::new(allow_item_open);
        let mut end = header_end;
        b.feed(header_line, inline);
        if allow_item_open && b.braces == 0 && inline.trim().is_empty() {
            let next = self
                .lines
                .get(self.pos)
                .filter(|l| l.text.starts_with('{'))
                .cloned();
            if let Some(l) = next {
                b.feed(l.start, &l.text);
                end = l.end;
                self.pos += 1;
            }
        }
        while b.braces > 0 {
            match self.lines.get(self.pos).cloned() {
                Some(l) => {
                    b.feed(l.start, &l.text);
                    end = l.end;
                    self.pos += 1;
                }
                None => {
                    self.unc(header_line, UnclassifiedKind::UnterminatedBlock);
                    break;
                }
            }
        }
        for l in std::mem::take(&mut b.unbalanced) {
            self.unc(l, UnclassifiedKind::UnbalancedClose);
        }
        if b.depth_exceeded {
            self.unc(header_line, UnclassifiedKind::NestingTooDeep);
        }
        self.inline_comments += b.comments;
        self.unparen += b.unparen;
        (b.finish(), end)
    }

    fn malformed(&mut self, line: &Line, kind: UnclassifiedKind) -> Item {
        self.unc(line.start, kind);
        Item::Other(Other {
            line: line.start,
            end_line: line.end,
            kind: OtherKind::MalformedHeader,
            statements: Vec::new(),
        })
    }

    fn parse_alias(&mut self, line: &Line, rest: &str) -> Item {
        let mut rest = rest.trim_start();
        let mut local = false;
        loop {
            let (w, r) = split_word(rest);
            if w.len() > 1 && w.starts_with('-') && w[1..].chars().all(|c| c.is_ascii_alphabetic())
            {
                if w.contains('l') {
                    local = true;
                }
                rest = r;
            } else {
                break;
            }
        }
        let (name_tok, remainder) = split_word(rest);
        let (name, remainder) = match name_tok.find('{') {
            Some(i) => {
                // `name{` glued form: keep the brace as part of the body.
                let brace_and_rest = &rest[i..];
                (&name_tok[..i], brace_and_rest)
            }
            None => (name_tok, remainder),
        };
        if name.is_empty() {
            return self.malformed(line, UnclassifiedKind::MalformedAliasHeader);
        }
        let name = name.to_lowercase();
        let (body, end) = self.consume_body(line.start, line.end, remainder, true);
        Item::Alias(Alias {
            line: line.start,
            end_line: end,
            name,
            local,
            body,
        })
    }

    fn parse_event(&mut self, line: &Line, rest: &str) -> Option<Item> {
        if !first_token_has_colon(rest) {
            return None;
        }
        let mut parts = rest.splitn(3, ':');
        let level = parts.next().unwrap_or("").trim();
        let event = parts.next().unwrap_or("").trim().to_lowercase();
        let remainder = parts.next();
        let valid_event = !event.is_empty() && event.chars().all(|c| c.is_ascii_alphanumeric());
        if level.is_empty() || level.contains(char::is_whitespace) || !valid_event {
            return Some(self.malformed(line, UnclassifiedKind::MalformedEventHeader));
        }
        let mut cmd = remainder.unwrap_or("");
        let mut fields = Vec::new();
        let mut incomplete = remainder.is_none();
        for _ in 0..event_field_count(&event) {
            match cmd.split_once(':') {
                Some((f, r)) => {
                    fields.push(f.trim().to_string());
                    cmd = r;
                }
                None => {
                    incomplete = true;
                    break;
                }
            }
        }
        let mut inline = cmd.trim().to_string();
        if incomplete && !cmd.trim().is_empty() {
            // Keep a trailing block opener so the body is still consumed.
            let t = cmd.trim();
            if t.ends_with('{') {
                let head = t[..t.len() - 1].trim();
                if !head.is_empty() {
                    fields.push(head.to_string());
                }
                inline = "{".to_string();
            } else if !t.starts_with('{') {
                fields.push(t.to_string());
                inline = String::new();
            }
        }
        if incomplete && event_field_count(&event) > 0 {
            self.unc(line.start, UnclassifiedKind::MalformedEventHeader);
        }
        let (body, end) = self.consume_body(line.start, line.end, &inline, true);
        Some(Item::Event(Event {
            line: line.start,
            end_line: end,
            levels: split_levels(level),
            event,
            fields,
            body: std::sync::Arc::new(body),
        }))
    }

    fn parse_raw(&mut self, line: &Line, rest: &str) -> Option<Item> {
        if !first_token_has_colon(rest) {
            return None;
        }
        let mut parts = rest.splitn(3, ':');
        let numeric = parts.next().unwrap_or("").trim().to_string();
        let match_text = parts.next();
        let cmd = parts.next();
        if numeric.is_empty() || numeric.contains(char::is_whitespace) || cmd.is_none() {
            return Some(self.malformed(line, UnclassifiedKind::MalformedRawHeader));
        }
        let inline = cmd.unwrap_or("").trim().to_string();
        let (body, end) = self.consume_body(line.start, line.end, &inline, true);
        Some(Item::Raw(Raw {
            line: line.start,
            end_line: end,
            numeric,
            match_text: match_text.unwrap_or("").to_string(),
            body,
        }))
    }

    fn parse_ctcp(&mut self, line: &Line, rest: &str) -> Option<Item> {
        if !first_token_has_colon(rest) {
            return None;
        }
        let mut parts = rest.splitn(4, ':');
        let level = parts.next().unwrap_or("").trim();
        let match_text = parts.next();
        let target = parts.next();
        let cmd = parts.next();
        if level.is_empty() || level.contains(char::is_whitespace) || cmd.is_none() {
            return Some(self.malformed(line, UnclassifiedKind::MalformedCtcpHeader));
        }
        let inline = cmd.unwrap_or("").trim().to_string();
        let (body, end) = self.consume_body(line.start, line.end, &inline, true);
        Some(Item::Ctcp(Ctcp {
            line: line.start,
            end_line: end,
            levels: split_levels(level),
            match_text: match_text.unwrap_or("").to_string(),
            target: target.unwrap_or("").trim().to_string(),
            body,
        }))
    }

    fn next_starts_with_brace(&self) -> bool {
        self.lines
            .get(self.pos)
            .map_or(false, |l| l.text.starts_with('{'))
    }

    fn parse_menu(&mut self, line: &Line, rest: &str) -> Option<Item> {
        let rest = rest.trim_start();
        let (names, after) = match rest.find(|c: char| c.is_whitespace() || c == '{') {
            Some(i) => (&rest[..i], rest[i..].trim_start()),
            None => (rest, ""),
        };
        if names.is_empty() {
            return None;
        }
        let first_content: Option<String> = if let Some(a) = after.strip_prefix('{') {
            Some(a.to_string())
        } else if after.is_empty() && self.next_starts_with_brace() {
            let l = self.lines[self.pos].clone();
            self.pos += 1;
            Some(l.text[1..].to_string())
        } else {
            None
        };
        let first_content = first_content?;
        let windows: Vec<String> = names
            .split(',')
            .map(|w| {
                let w = w.trim();
                if w.starts_with('@') {
                    w.to_string()
                } else {
                    w.to_lowercase()
                }
            })
            .filter(|w| !w.is_empty())
            .collect();

        let mut flat: Vec<MenuItem> = Vec::new();
        let mut end = line.end;
        let mut closed = false;
        let mut pending: Option<(usize, usize, String)> =
            Some((line.start, line.end, first_content));
        loop {
            let (lno, lend, text) = match pending.take() {
                Some(p) => p,
                None => match self.lines.get(self.pos).cloned() {
                    Some(l) => {
                        self.pos += 1;
                        (l.start, l.end, l.text)
                    }
                    None => break,
                },
            };
            end = lend;
            let t = text.trim();
            if t.is_empty() {
                continue;
            }
            if t.starts_with('}') {
                closed = true;
                break;
            }
            // A line like `Item:cmd }` carries the closing brace of the menu.
            let mut t = t;
            let mut ends_menu = false;
            if t.ends_with('}') {
                let (toks, _) = tokenize(t);
                let opens = toks.iter().filter(|x| **x == Tok::Open).count();
                let closes = toks.iter().filter(|x| **x == Tok::Close).count();
                if closes > opens {
                    t = t[..t.len() - 1].trim_end();
                    ends_menu = true;
                }
            }
            if let Some(item) = self.parse_menu_item(lno, lend, t) {
                end = end.max(item.end_line);
                flat.push(item);
            }
            if ends_menu {
                closed = true;
                break;
            }
        }
        if !closed {
            self.unc(line.start, UnclassifiedKind::UnterminatedMenu);
        }
        Some(Item::Menu(Menu {
            line: line.start,
            end_line: end,
            windows,
            items: nest_menu(flat),
        }))
    }

    fn parse_menu_item(&mut self, lno: usize, lend: usize, t: &str) -> Option<MenuItem> {
        let dots = t.bytes().take_while(|&c| c == b'.').count();
        let level = dots.min(31);
        let body_text = t[dots..].trim_start();
        if body_text.is_empty() {
            return None;
        }
        if body_text == "-" {
            return Some(MenuItem {
                line: lno,
                end_line: lend,
                level,
                kind: MenuItemKind::Separator,
                label: "-".to_string(),
                body: Vec::new(),
                children: Vec::new(),
            });
        }
        if let Some(ci) = top_level_colon(body_text) {
            let label = body_text[..ci].trim().to_string();
            let cmd = body_text[ci + 1..].to_string();
            let (body, end) = self.consume_body(lno, lend, &cmd, true);
            return Some(MenuItem {
                line: lno,
                end_line: end,
                level,
                kind: MenuItemKind::Action,
                label,
                body,
                children: Vec::new(),
            });
        }
        // `Label { commands }`, on one line or with the block opened on this line.
        if let Some(open) = body_text.find('{').filter(|&i| i > 0 && !body_text[..i].trim().is_empty()) {
            let label = body_text[..open].trim().to_string();
            let (body, end) = self.consume_body(lno, lend, &body_text[open..], true);
            return Some(MenuItem {
                line: lno,
                end_line: end,
                level,
                kind: MenuItemKind::Action,
                label,
                body,
                children: Vec::new(),
            });
        }
        if self.next_starts_with_brace() {
            let (body, end) = self.consume_body(lno, lend, "", true);
            return Some(MenuItem {
                line: lno,
                end_line: end,
                level,
                kind: MenuItemKind::Action,
                label: body_text.to_string(),
                body,
                children: Vec::new(),
            });
        }
        Some(MenuItem {
            line: lno,
            end_line: lend,
            level,
            kind: MenuItemKind::Submenu,
            label: body_text.to_string(),
            body: Vec::new(),
            children: Vec::new(),
        })
    }

    fn parse_dialog(&mut self, line: &Line, rest: &str) -> Option<Item> {
        let mut rest = rest.trim_start();
        let mut local = false;
        loop {
            let (w, r) = split_word(rest);
            if w.len() > 1 && w.starts_with('-') && w[1..].chars().all(|c| c.is_ascii_alphabetic())
            {
                if w.contains('l') {
                    local = true;
                }
                rest = r;
            } else {
                break;
            }
        }
        let (head, after): (&str, Option<String>) = match rest.find('{') {
            Some(i) => (&rest[..i], Some(rest[i + 1..].to_string())),
            None => (rest, None),
        };
        let name = head.split_whitespace().next().unwrap_or("").to_lowercase();
        if name.is_empty() {
            return None;
        }
        let after = match after {
            Some(a) => a,
            None => {
                if head.split_whitespace().count() == 1 && self.next_starts_with_brace() {
                    let l = self.lines[self.pos].clone();
                    self.pos += 1;
                    l.text[1..].to_string()
                } else {
                    return None;
                }
            }
        };
        let mut content_lines = 0usize;
        let mut end = line.end;
        let mut closed = false;
        let first = after.trim();
        if let Some(inner) = first.strip_suffix('}') {
            if !inner.trim().is_empty() {
                content_lines += 1;
            }
            closed = true;
        } else {
            if !first.is_empty() {
                content_lines += 1;
            }
            while let Some(l) = self.lines.get(self.pos).cloned() {
                self.pos += 1;
                end = l.end;
                if l.text.starts_with('}') {
                    closed = true;
                    break;
                }
                content_lines += 1;
            }
        }
        if !closed {
            self.unc(line.start, UnclassifiedKind::UnterminatedDialog);
        }
        Some(Item::Dialog(Dialog {
            line: line.start,
            end_line: end,
            name,
            local,
            content_lines,
        }))
    }

    fn parse_item(&mut self, line: &Line) -> Option<Item> {
        let (word, rest) = split_word(&line.text);
        match word.to_ascii_lowercase().as_str() {
            "alias" => Some(self.parse_alias(line, rest)),
            "on" => self.parse_event(line, rest),
            "raw" => self.parse_raw(line, rest),
            "ctcp" => self.parse_ctcp(line, rest),
            "menu" => self.parse_menu(line, rest),
            "dialog" => self.parse_dialog(line, rest),
            _ => None,
        }
    }

    fn parse_other(&mut self, line: &Line) -> Option<Item> {
        let text = line.text.as_str();
        let first = text.chars().next()?;
        match first {
            '}' => {
                self.unc(line.start, UnclassifiedKind::UnbalancedClose);
                None
            }
            '{' => {
                self.unc(line.start, UnclassifiedKind::StrayOpenBrace);
                let (statements, end) = self.consume_body(line.start, line.end, text, true);
                Some(Item::Other(Other {
                    line: line.start,
                    end_line: end,
                    kind: OtherKind::StrayBlock,
                    statements,
                }))
            }
            '[' if text.ends_with(']') => Some(Item::Other(Other {
                line: line.start,
                end_line: line.end,
                kind: OtherKind::IniSection,
                statements: Vec::new(),
            })),
            _ if is_ini_entry(text) => Some(Item::Other(Other {
                line: line.start,
                end_line: line.end,
                kind: OtherKind::IniEntry,
                statements: Vec::new(),
            })),
            c if command_start(c) => {
                let (statements, end) = self.consume_body(line.start, line.end, text, false);
                Some(Item::Other(Other {
                    line: line.start,
                    end_line: end,
                    kind: OtherKind::TopLevelCommand,
                    statements,
                }))
            }
            _ => {
                self.unc(line.start, UnclassifiedKind::UnknownTopLevel);
                None
            }
        }
    }

    fn run(mut self) -> Parsed {
        let mut root: Vec<Item> = Vec::new();
        let mut group: Option<Group> = None;

        while self.pos < self.lines.len() {
            let line = self.lines[self.pos].clone();
            self.pos += 1;

            if line.text.starts_with('#') {
                let toks: Vec<&str> = line.text.split_whitespace().collect();
                let state = toks.get(1).map(|s| s.to_ascii_lowercase());
                let marker = toks.len() == 2
                    && toks[0].len() > 1
                    && matches!(state.as_deref(), Some("on") | Some("off") | Some("end"));
                if !marker {
                    self.unc(line.start, UnclassifiedKind::UnknownTopLevel);
                    continue;
                }
                match state.as_deref() {
                    Some("end") => match group.take() {
                        Some(mut g) if g.name.eq_ignore_ascii_case(toks[0]) => {
                            g.end_line = Some(line.end);
                            root.push(Item::Group(g));
                        }
                        other => {
                            group = other;
                            self.unc(line.start, UnclassifiedKind::UnmatchedGroupEnd);
                        }
                    },
                    st => {
                        if group.is_some() {
                            self.unc(line.start, UnclassifiedKind::NestedGroup);
                        } else {
                            group = Some(Group {
                                line: line.start,
                                end_line: None,
                                name: toks[0].to_lowercase(),
                                enabled: st == Some("on"),
                                items: Vec::new(),
                            });
                        }
                    }
                }
                continue;
            }

            let item = match self.parse_item(&line) {
                Some(i) => Some(i),
                None => self.parse_other(&line),
            };
            if let Some(item) = item {
                match group.as_mut() {
                    Some(g) => g.items.push(item),
                    None => root.push(item),
                }
            }
        }

        if let Some(g) = group.take() {
            self.unc(g.line, UnclassifiedKind::UnterminatedGroup);
            root.push(Item::Group(g));
        }
        Parsed {
            items: root,
            unclassified: self.unclassified,
            inline_comments: self.inline_comments,
            unparenthesised: self.unparen,
        }
    }
}

fn attach_menu(stack: &mut Vec<MenuItem>, roots: &mut Vec<MenuItem>, done: MenuItem) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => roots.push(done),
    }
}

/// Builds the menu tree from the flat list using the dot levels. Iterative;
/// the depth is bounded by the clamped level (at most 32).
fn nest_menu(flat: Vec<MenuItem>) -> Vec<MenuItem> {
    let mut roots = Vec::new();
    let mut stack: Vec<MenuItem> = Vec::new();
    for item in flat {
        while stack.last().map_or(false, |top| top.level >= item.level) {
            let done = stack.pop().unwrap();
            attach_menu(&mut stack, &mut roots, done);
        }
        stack.push(item);
    }
    while let Some(done) = stack.pop() {
        attach_menu(&mut stack, &mut roots, done);
    }
    roots
}

pub(crate) fn parse_lines(scan: LineScan) -> Parsed {
    Parser {
        lines: scan.lines,
        pos: 0,
        unclassified: Vec::new(),
        inline_comments: 0,
        unparen: 0,
    }
    .run()
}
