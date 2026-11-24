//! Expression text for the interpreter (milestone M1).
//!
//! Turns the raw argument text of a statement into [`Part`]s (literal text,
//! `%var`, `$N`, `$N-`, `$N-M`, `$name`, `[ ]`) and recognises integer
//! arithmetic (`+ - * /` and parentheses) over those parts.
//!
//! Everything here works on unevaluated text. Evaluated values are never
//! parsed again, so remote text cannot add parts, operators or commands.
//!
//! `$calc(args)` supports a bounded arithmetic subset. Other `$name(args)`
//! calls are not understood: the `$name` part is an unknown identifier and
//! `(args)` stays literal text.

use std::collections::HashMap;

/// One piece of an unevaluated text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Part {
    Lit(String),
    /// `%name`, stored lowercase without the percent sign.
    Var(String),
    /// `$0` (parameter count) or `$N`.
    Param(usize),
    /// `$N-` (`to` is `None`) or `$N-M`, both ends inclusive.
    Range { from: usize, to: Option<usize> },
    /// `$name`, stored lowercase.
    Ident(String),
    /// The unevaluated argument of a $calc call.
    Calc(String),
    /// `$name(args)` for a built-in text identifier (see `CALL_NAMES`). `args`
    /// is the raw argument text, split later at top-level commas.
    Call { name: String, args: String, prop: Option<String> },
    /// `[ ... ]`: evaluated as a nested expression (one level only).
    Bracket(Vec<Part>),
    /// `$?`, `$$?` or `$?="question"`: the next answer the caller gave (popup menus, plan M5).
    /// `None` asks with the default question.
    Prompt(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprError {
    /// A `[ ]` inside a `[ ]`.
    NestedBrackets,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Off,
    Top,
    Inner,
}

/// Length of the variable name at the start of `s` (the text after `%`).
/// Letters, digits, `_` and inner `.`; trailing dots stay text (Ferric choice:
/// "value is %x." must not swallow the full stop).
/// The prompts written in a text (`$?`, `$$?`, `$?="question"`), in order. `$$?` is one prompt.
pub(crate) fn prompts_in(text: &str) -> Vec<Option<String>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' {
            if let Some((end, Part::Prompt(question))) = dollar(text, i) {
                out.push(question);
                // Go on after the prompt: a `$` inside its question is not a prompt of its own.
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn var_name_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut n = 0;
    while n < b.len() && (b[n].is_ascii_alphanumeric() || b[n] == b'_' || b[n] == b'.') {
        n += 1;
    }
    while n > 0 && b[n - 1] == b'.' {
        n -= 1;
    }
    if n > 0 && b[0] == b'.' {
        return 0;
    }
    n
}

/// Parses `text` into parts. With `brackets` set, `[ ... ]` becomes
/// [`Part::Bracket`]; otherwise brackets are plain text.
pub(crate) fn parse(text: &str, brackets: bool) -> Result<Vec<Part>, ExprError> {
    parse_level(text, if brackets { Level::Top } else { Level::Off })
}

/// Matching `[`/`]` pairs, found in one pass (a repeated scan per `[` would
/// be quadratic on hostile text).
fn bracket_pairs(b: &[u8]) -> HashMap<usize, usize> {
    let mut pairs = HashMap::new();
    let mut open = Vec::new();
    for (i, &c) in b.iter().enumerate() {
        if c == b'[' {
            open.push(i);
        } else if c == b']' {
            if let Some(o) = open.pop() {
                pairs.insert(o, i);
            }
        }
    }
    pairs
}

fn digits_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    i
}

fn number(text: &str, from: usize, to: usize) -> usize {
    text[from..to].parse::<usize>().unwrap_or(usize::MAX)
}

/// Parses the `$` at `i`. Returns the end index and the part, or `None` when
/// the `$` is plain text.
fn dollar(text: &str, i: usize) -> Option<(usize, Part)> {
    let b = text.as_bytes();
    let s = i + 1;
    // `$$1` is `$1` (it asks when the parameter is missing; that is not modelled here). Without this
    // the first `$` stayed as text and the value came out as `$MereX`.
    if b.get(s) == Some(&b'$') && b.get(s + 1).is_some_and(|c| c.is_ascii_digit()) {
        return dollar(text, s);
    }
    // `$?` and `$$?` ask the user; `$?="question"` gives the question.
    let question_at = if b.get(s) == Some(&b'$') { s + 1 } else { s };
    if b.get(question_at) == Some(&b'?') {
        let mut end = question_at + 1;
        let mut question = None;
        if b.get(end) == Some(&b'=') && b.get(end + 1) == Some(&b'"') {
            if let Some(close) = text[end + 2..].find('"') {
                question = Some(text[end + 2..end + 2 + close].to_string());
                end += 2 + close + 1;
            }
        }
        return Some((end, Part::Prompt(question)));
    }
    // `$!name` is read when a timer runs (plan M4); the name keeps its `!`.
    if b.get(s) == Some(&b'!') && b.get(s + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
        let mut e = s + 1;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
            e += 1;
        }
        return Some((e, Part::Ident(format!("!{}", text[s + 1..e].to_ascii_lowercase()))));
    }
    let first = *b.get(s)?;
    if first.is_ascii_digit() {
        let e = digits_end(b, s);
        let n = number(text, s, e);
        if n >= 1 && b.get(e) == Some(&b'-') {
            let m_end = digits_end(b, e + 1);
            if m_end > e + 1 {
                let m = number(text, e + 1, m_end);
                return Some((
                    m_end,
                    Part::Range {
                        from: n,
                        to: Some(m),
                    },
                ));
            }
            return Some((e + 1, Part::Range { from: n, to: None }));
        }
        return Some((e, Part::Param(n)));
    }
    if first.is_ascii_alphabetic() || first == b'_' {
        let mut e = s;
        while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
            e += 1;
        }
        if text[s..e].eq_ignore_ascii_case("calc") && b.get(e) == Some(&b'(') {
            let mut depth = 1usize;
            for (offset, &c) in b[e + 1..].iter().enumerate() {
                if c == b'(' { depth += 1; }
                if c == b')' {
                    depth -= 1;
                    if depth == 0 {
                        let end = e + 1 + offset;
                        return Some((end + 1, Part::Calc(text[e + 1..end].to_string())));
                    }
                }
            }
        }
        let lower = text[s..e].to_ascii_lowercase();
        if CALL_NAMES.contains(&lower.as_str()) && b.get(e) == Some(&b'(') {
            let mut depth = 0usize;
            for (offset, &c) in b[e..].iter().enumerate() {
                if c == b'(' { depth += 1; }
                if c == b')' {
                    depth -= 1;
                    if depth == 0 {
                        let end = e + offset;
                        let (after, prop) = property_after(b, end + 1);
                        return Some((
                            after,
                            Part::Call { name: lower, args: text[e + 1..end].to_string(), prop },
                        ));
                    }
                }
            }
        }
        return Some((e, Part::Ident(lower)));
    }
    None
}

/// A `.name` property right after a call, as in `$hget(t,1).item`. Returns the index after it.
fn property_after(b: &[u8], at: usize) -> (usize, Option<String>) {
    if b.get(at) == Some(&b'.') && b.get(at + 1).is_some_and(|c| c.is_ascii_alphabetic()) {
        let mut e = at + 1;
        while e < b.len() && b[e].is_ascii_alphabetic() {
            e += 1;
        }
        return (e, Some(String::from_utf8_lossy(&b[at + 1..e]).to_ascii_lowercase()));
    }
    (at, None)
}

/// Built-in text identifiers that take a `(...)` argument list. Other `$name(`
/// stays a plain identifier followed by text, as before.
const CALL_NAMES: &[&str] = &[
    "len", "left", "right", "mid", "upper", "lower", "str", "replace", "gettok", "numtok",
    "iif", "rand", "time", "date", "gmt", "ctime", "fulldate", "day", "asctime", "timezone",
    "daylight",
    "address",
    "abs", "int", "floor", "ceil", "round", "sqrt", "gcd", "lcm", "min", "max", "and", "or", "xor",
    "not", "biton", "bitoff", "isbit", "addtok", "deltok", "findtok", "instok", "istok", "puttok",
    "remtok", "reptok",
    "asc", "chr", "count", "countcs", "pos", "poscs", "remove", "removecs", "replacex",
    "replacexcs", "isnum", "islower", "isupper", "ord", "qt", "noqt", "strip", "hget", "hfind", "timer",
    "regex", "regml", "regmlex", "regsub", "regsubex",
];

fn parse_level(text: &str, level: Level) -> Result<Vec<Part>, ExprError> {
    let b = text.as_bytes();
    let pairs = if level != Level::Off && b.contains(&b'[') {
        bracket_pairs(b)
    } else {
        HashMap::new()
    };
    let mut parts: Vec<Part> = Vec::new();
    let mut lit = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        // A bare `$+` joins the text on its two sides: the spaces around it go (example: `Hel $+ lo`).
        // `$+(` is a call, handled below.
        if b[i] == b'$' && text[i..].starts_with("$+") && b.get(i + 2) != Some(&b'(') {
            let kept = lit + text[lit..i].trim_end_matches(' ').len();
            if lit < kept {
                parts.push(Part::Lit(text[lit..kept].to_string()));
            }
            i += 2;
            while i < b.len() && b[i] == b' ' {
                i += 1;
            }
            lit = i;
            continue;
        }
        // Every index used to slice `text` below sits on an ASCII byte.
        let (end, part) = match b[i] {
            b'%' => {
                let n = var_name_len(&text[i + 1..]);
                if n == 0 {
                    i += 1;
                    continue;
                }
                (
                    i + 1 + n,
                    Part::Var(text[i + 1..i + 1 + n].to_ascii_lowercase()),
                )
            }
            b'$' => match dollar(text, i) {
                Some(x) => x,
                None => {
                    i += 1;
                    continue;
                }
            },
            b'[' if level != Level::Off => match pairs.get(&i) {
                Some(&close) => {
                    if level == Level::Inner {
                        return Err(ExprError::NestedBrackets);
                    }
                    let inner = parse_level(&text[i + 1..close], Level::Inner)?;
                    (close + 1, Part::Bracket(inner))
                }
                None => {
                    i += 1;
                    continue;
                }
            },
            _ => {
                i += 1;
                continue;
            }
        };
        if lit < i {
            parts.push(Part::Lit(text[lit..i].to_string()));
        }
        parts.push(part);
        i = end;
        lit = end;
    }
    if lit < b.len() {
        parts.push(Part::Lit(text[lit..].to_string()));
    }
    Ok(parts)
}

// ---------------------------------------------------------------- arithmetic

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArithError {
    DivByZero,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tok {
    /// The k-th non-literal part of the expression.
    Atom(usize),
    Num(i64),
    Plus,
    Minus,
    Star,
    Slash,
    Open,
    Close,
}

/// Parenthesis nesting allowed in an expression.
const MAX_PAREN_DEPTH: usize = 16;

/// An expression that is arithmetic by its source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shape {
    pub toks: Vec<Tok>,
    /// Indices (into the part list) of the operands, in order of `Atom(k)`.
    pub atoms: Vec<usize>,
}

fn lex_lit(s: &str, toks: &mut Vec<Tok>) -> Option<()> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' => i += 1,
            b'+' => {
                toks.push(Tok::Plus);
                i += 1;
            }
            b'-' => {
                toks.push(Tok::Minus);
                i += 1;
            }
            b'*' => {
                toks.push(Tok::Star);
                i += 1;
            }
            b'/' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            b'(' => {
                toks.push(Tok::Open);
                i += 1;
            }
            b')' => {
                toks.push(Tok::Close);
                i += 1;
            }
            b'0'..=b'9' => {
                let e = digits_end(b, i);
                toks.push(Tok::Num(s[i..e].parse::<i64>().ok()?));
                i = e;
            }
            _ => return None,
        }
    }
    Some(())
}

/// Returns the arithmetic shape of `parts`, or `None` when the text is not an
/// arithmetic expression (so it stays plain text). The decision depends on the
/// source text only: operands are `%var`, `$N` and the like, never their
/// values. At least one binary operator is required, so `-5` and `$1` stay text.
pub(crate) fn shape(parts: &[Part]) -> Option<Shape> {
    let mut toks = Vec::new();
    let mut atoms = Vec::new();
    for (idx, p) in parts.iter().enumerate() {
        match p {
            Part::Lit(s) => lex_lit(s, &mut toks)?,
            _ => {
                toks.push(Tok::Atom(atoms.len()));
                atoms.push(idx);
            }
        }
    }
    let probe = run(&toks, &vec![0; atoms.len()]);
    if probe.bad || probe.ops == 0 {
        return None;
    }
    Some(Shape { toks, atoms })
}

/// Computes a shape with the operand values in `vals` (one per atom).
pub(crate) fn compute(shape: &Shape, vals: &[i64]) -> Result<i64, ArithError> {
    let r = run(&shape.toks, vals);
    match r.err {
        Some(e) => Err(e),
        None => Ok(r.value),
    }
}

struct Outcome {
    value: i64,
    bad: bool,
    ops: usize,
    err: Option<ArithError>,
}

struct Calc<'a> {
    toks: &'a [Tok],
    vals: &'a [i64],
    i: usize,
    depth: usize,
    ops: usize,
    bad: bool,
    err: Option<ArithError>,
}

fn run(toks: &[Tok], vals: &[i64]) -> Outcome {
    let mut c = Calc {
        toks,
        vals,
        i: 0,
        depth: 0,
        ops: 0,
        bad: false,
        err: None,
    };
    let value = c.expr();
    Outcome {
        value,
        bad: c.bad || c.i != toks.len(),
        ops: c.ops,
        err: c.err,
    }
}

impl Calc<'_> {
    fn peek(&self) -> Option<Tok> {
        self.toks.get(self.i).copied()
    }

    fn ok(&mut self, r: Option<i64>) -> i64 {
        r.unwrap_or_else(|| {
            self.err.get_or_insert(ArithError::Overflow);
            0
        })
    }

    fn expr(&mut self) -> i64 {
        let mut v = self.term();
        loop {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.term();
                    v = self.ok(v.checked_add(r));
                }
                Some(Tok::Minus) => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.term();
                    v = self.ok(v.checked_sub(r));
                }
                _ => return v,
            }
        }
    }

    fn term(&mut self) -> i64 {
        let mut v = self.unary();
        loop {
            match self.peek() {
                Some(Tok::Star) => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.unary();
                    v = self.ok(v.checked_mul(r));
                }
                Some(Tok::Slash) => {
                    self.i += 1;
                    self.ops += 1;
                    let r = self.unary();
                    if r == 0 {
                        self.err.get_or_insert(ArithError::DivByZero);
                        v = 0;
                    } else {
                        v = self.ok(v.checked_div(r));
                    }
                }
                _ => return v,
            }
        }
    }

    fn unary(&mut self) -> i64 {
        // Signs are folded in a loop so a long run of them cannot recurse.
        let mut neg = false;
        while let Some(t @ (Tok::Minus | Tok::Plus)) = self.peek() {
            neg ^= t == Tok::Minus;
            self.i += 1;
        }
        let v = match self.peek() {
            Some(Tok::Num(n)) => {
                self.i += 1;
                n
            }
            Some(Tok::Atom(k)) => {
                self.i += 1;
                self.vals.get(k).copied().unwrap_or(0)
            }
            Some(Tok::Open) if self.depth < MAX_PAREN_DEPTH => {
                self.i += 1;
                self.depth += 1;
                let v = self.expr();
                self.depth -= 1;
                if self.peek() == Some(Tok::Close) {
                    self.i += 1;
                } else {
                    self.bad = true;
                }
                v
            }
            _ => {
                self.bad = true;
                0
            }
        };
        if neg {
            self.ok(v.checked_neg())
        } else {
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Part {
        Part::Lit(s.to_string())
    }

    fn calc(text: &str) -> Option<Result<i64, ArithError>> {
        let parts = parse(text, true).unwrap();
        let sh = shape(&parts)?;
        Some(compute(&sh, &vec![0; sh.atoms.len()]))
    }

    #[test]
    fn parts_of_plain_text() {
        assert_eq!(parse("", false).unwrap(), vec![]);
        assert_eq!(parse("Hello", false).unwrap(), vec![lit("Hello")]);
        assert_eq!(
            parse("Hello $1 and %Name.", false).unwrap(),
            vec![
                lit("Hello "),
                Part::Param(1),
                lit(" and "),
                Part::Var("name".into()),
                lit("."),
            ]
        );
    }

    #[test]
    fn double_dollar_parameter_is_the_parameter() {
        assert_eq!(parse("add $$1 x", false).unwrap(), vec![lit("add "), Part::Param(1), lit(" x")]);
        assert_eq!(parse("$$1-", false).unwrap(), parse("$1-", false).unwrap());
    }

    #[test]
    fn prompts_are_parts_and_are_listed_in_order() {
        assert_eq!(
            parse("a $? b $$?", false).unwrap(),
            vec![lit("a "), Part::Prompt(None), lit(" b "), Part::Prompt(None)]
        );
        assert_eq!(
            parse("$?=\"Channel?\" $1", false).unwrap(),
            vec![Part::Prompt(Some("Channel?".into())), lit(" "), Part::Param(1)]
        );
        assert_eq!(
            super::prompts_in("x $? y $$?=\"two\" $nick $?"),
            vec![None, Some("two".to_string()), None]
        );
    }

    #[test]
    fn params_and_ranges() {
        assert_eq!(parse("$0", false).unwrap(), vec![Part::Param(0)]);
        assert_eq!(parse("$12", false).unwrap(), vec![Part::Param(12)]);
        assert_eq!(
            parse("$2-", false).unwrap(),
            vec![Part::Range { from: 2, to: None }]
        );
        assert_eq!(
            parse("$2-4x", false).unwrap(),
            vec![
                Part::Range {
                    from: 2,
                    to: Some(4)
                },
                lit("x")
            ]
        );
        // `$0-` is not a range: the count, then a dash.
        assert_eq!(
            parse("$0-", false).unwrap(),
            vec![Part::Param(0), lit("-")]
        );
        assert_eq!(parse("$1abc", false).unwrap(), vec![Part::Param(1), lit("abc")]);
    }

    #[test]
    fn text_identifier_calls_parse_only_for_known_names() {
        assert_eq!(
            parse("$len(a,b)", false).unwrap(),
            vec![Part::Call { name: "len".into(), args: "a,b".into(), prop: None }]
        );
        assert_eq!(
            parse("x $gettok(a.b,2,46) y", false).unwrap(),
            vec![
                lit("x "),
                Part::Call { name: "gettok".into(), args: "a.b,2,46".into(), prop: None },
                lit(" y"),
            ]
        );
        // Unknown names and unclosed calls keep the old split.
        assert_eq!(
            parse("$foo(x)", false).unwrap(),
            vec![Part::Ident("foo".into()), lit("(x)")]
        );
        assert_eq!(
            parse("$len(a", false).unwrap(),
            vec![Part::Ident("len".into()), lit("(a")]
        );
    }

    #[test]
    fn dollars_and_percents_that_are_text() {
        assert_eq!(parse("cost $ 5 100% %", false).unwrap(), vec![lit("cost $ 5 100% %")]);
        assert_eq!(
            parse("$Nick(x)", false).unwrap(),
            vec![Part::Ident("nick".into()), lit("(x)")]
        );
        assert_eq!(parse("%.x", false).unwrap(), vec![lit("%.x")]);
        assert_eq!(
            parse("%a.b.", false).unwrap(),
            vec![Part::Var("a.b".into()), lit(".")]
        );
    }

    #[test]
    fn brackets_one_level() {
        assert_eq!(parse("[ 1 ]", false).unwrap(), vec![lit("[ 1 ]")]);
        assert_eq!(
            parse("a [ $1 ] b", true).unwrap(),
            vec![lit("a "), Part::Bracket(vec![lit(" "), Part::Param(1), lit(" ")]), lit(" b")]
        );
        assert_eq!(parse("[ [ 1 ] ]", true), Err(ExprError::NestedBrackets));
        // Unmatched brackets are text.
        assert_eq!(parse("[ 1", true).unwrap(), vec![lit("[ 1")]);
        assert_eq!(parse("1 ]", true).unwrap(), vec![lit("1 ]")]);
        // An unmatched opener does not hide a later pair.
        assert_eq!(
            parse("[ [ 2 ]", true).unwrap(),
            vec![lit("[ "), Part::Bracket(vec![lit(" 2 ")])]
        );
    }

    #[test]
    fn arithmetic_table() {
        let cases: &[(&str, i64)] = &[
            ("1 + 2", 3),
            ("2 + 3 * 4", 14),
            ("(2 + 3) * 4", 20),
            ("10 - 2 - 3", 5),
            ("7 / 2", 3),
            ("-7 / 2", -3),
            ("2 * -3", -6),
            ("1 - -1", 2),
            ("---1 + 0", -1),
            ("1+1", 2),
        ];
        for (src, want) in cases {
            assert_eq!(calc(src), Some(Ok(*want)), "{src}");
        }
    }

    #[test]
    fn not_arithmetic_stays_text() {
        for src in ["", "5", "-5", "(5)", "hello", "5 apples", "1 +", "1 + * 2", "(1 + 2", "1 2", "3.5 + 1", "1)", "99999999999999999999 + 1"] {
            assert_eq!(calc(src), None, "{src:?}");
        }
    }

    #[test]
    fn arithmetic_errors() {
        assert_eq!(calc("1 / 0"), Some(Err(ArithError::DivByZero)));
        assert_eq!(calc("1 / (2 - 2)"), Some(Err(ArithError::DivByZero)));
        assert_eq!(calc("9223372036854775807 + 1"), Some(Err(ArithError::Overflow)));
        assert_eq!(calc("9223372036854775807 * 2"), Some(Err(ArithError::Overflow)));
        assert_eq!(
            calc("-9223372036854775807 - 2"),
            Some(Err(ArithError::Overflow))
        );
        // A syntax problem wins over a runtime error: the text stays text.
        assert_eq!(calc("1 / 0 +"), None);
    }

    #[test]
    fn operands_come_from_parts_not_values() {
        let parts = parse("%a + $1 * 2", true).unwrap();
        let sh = shape(&parts).unwrap();
        assert_eq!(sh.atoms, vec![0, 2]);
        assert_eq!(compute(&sh, &[1, 5]), Ok(11));
        // A single operand is text, whatever its value looks like.
        assert_eq!(shape(&parse("$1", true).unwrap()), None);
    }

    #[test]
    fn deep_parentheses_and_long_sign_runs_do_not_overflow_the_stack() {
        let deep = format!("{}1{} + 1", "(".repeat(5000), ")".repeat(5000));
        assert_eq!(calc(&deep), None);
        let signs = format!("{}1 + 1", "-".repeat(100_000));
        assert_eq!(calc(&signs), Some(Ok(2)));
        let brackets = "[".repeat(50_000);
        assert!(parse(&brackets, true).is_ok());
    }
}
