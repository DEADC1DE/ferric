//! Regular expressions: `$regex`, `$regml`, `$regmlex`, `$regsub`, `$regsubex`
//! and `$regerrstr`. `$regsub` and `$regsubex` are evaluated in `eval.rs`,
//! because they write to a variable and `$regsubex` evaluates its subtext for
//! each match.
//!
//! Semantics follow the scripting reference `regular_expressions`. Engine: the
//! `regex` crate. It has no back-references and no look-around: a pattern that
//! needs them does not compile, `$regex` then answers 0 and `$regerrstr` says why.
//!
//! Ferric choices (the help shows the modifier list, not the pattern syntax):
//! * A pattern written `/body/modifiers` has its body and modifiers split at the
//!   last slash. Any other text is the body, with no modifiers.
//! * Modifiers: `g` all matches (otherwise the first only), `i` case-insensitive,
//!   `m` multi-line, `s` dot matches a newline, `x` ignore whitespace, `A` anchored
//!   at the start, `U` ungreedy. `S` is accepted and keeps the text as it is: the
//!   client's event matchtext keeps color codes under `S`. `E`, `D`, `u`, `X`
//!   and `F` are accepted and change nothing: the engine already matches that way.
//! * `$regml` items: every matched group of every match, the whole match being
//!   group 0. Empty groups that took part are kept. `pos` counts characters from 1,
//!   `bytepos` bytes from 1.
//! * `$regsub` subtext: `\N` is group N (single digit, 0 is the whole match) and
//!   `\\` is a backslash. Its text is not evaluated.
//! * `$regsubex` subtext: `\n` match number, `\t` match text, `\a` the groups
//!   joined by a space, `\A` the groups joined without one, `\0` the number of
//!   groups that matched, `\N` group N. Identifiers in the subtext are evaluated
//!   once per match. Markers are replaced in the plain text of the subtext, not
//!   inside an identifier's arguments.
//! * `$regsub` and `$regsubex` need a `%variable`. `$regsubex` also needs a name.
//! * Only the last 50 names keep their results.

use std::collections::{BTreeMap, VecDeque};

use regex::RegexBuilder;

use super::text_funcs::CallError;

/// The name used when a call gives none.
pub(crate) const DEFAULT_NAME: &str = "default";
const KEEP_NAMES: usize = 50;
/// Most `$regml` items kept per name, and most bytes of item text: a match-heavy search cannot
/// fill memory.
const MAX_REGEX_ITEMS: usize = 2_000;
const MAX_REGEX_BYTES: usize = 1024 * 1024;

/// The groups of one match: index 0 is the whole match. Each entry is the byte
/// offset in the text and the matched text, or `None` when the group did not take part.
pub(crate) type Groups = Vec<Option<(usize, String)>>;

/// One `$regml` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    text: String,
    pos: usize,
    bytepos: usize,
    group: usize,
    match_no: usize,
}

/// Results of the regular expression calls, by lower-case name.
#[derive(Debug, Default)]
pub(crate) struct RegexState {
    results: BTreeMap<String, Vec<Item>>,
    order: VecDeque<String>,
    /// The error text of the last pattern the engine refused, for `$regerrstr`.
    pub(crate) last_error: String,
}

/// A successful search: the text it ran on and the matches.
pub(crate) struct Found {
    pub(crate) text: String,
    pub(crate) matches: Vec<Groups>,
}

/// Splits `/body/modifiers`. Text without a closing slash is all body.
fn split_pattern(pattern: &str) -> (&str, &str) {
    if let Some(stripped) = pattern.strip_prefix('/') {
        if let Some(close) = stripped.rfind('/') {
            return (&stripped[..close], &stripped[close + 1..]);
        }
    }
    (pattern, "")
}

/// Runs a pattern over a text.
pub(crate) fn find(text: &str, pattern: &str) -> Result<Found, String> {
    let (body, flags) = split_pattern(pattern);
    if let Some(bad) = flags.chars().find(|c| !"gimsxAEDUuXSF".contains(*c)) {
        return Err(format!("unknown regular expression modifier '{bad}'"));
    }
    let text = text.to_string();
    let body = if flags.contains('A') {
        format!(r"\A(?:{body})")
    } else {
        body.to_string()
    };
    let re = RegexBuilder::new(&body)
        .case_insensitive(flags.contains('i'))
        .multi_line(flags.contains('m'))
        .dot_matches_new_line(flags.contains('s'))
        .ignore_whitespace(flags.contains('x'))
        .swap_greed(flags.contains('U'))
        .build()
        .map_err(|e| e.to_string())?;
    let caps: Vec<regex::Captures<'_>> = if flags.contains('g') {
        re.captures_iter(&text).take(MAX_REGEX_ITEMS).collect()
    } else {
        re.captures(&text).into_iter().collect()
    };
    let matches = caps
        .iter()
        .map(|c| {
            (0..c.len())
                .map(|g| c.get(g).map(|m| (m.start(), m.as_str().to_string())))
                .collect()
        })
        .collect();
    Ok(Found { text, matches })
}

/// Stores the items of a search under a name, dropping the oldest name when full. Per name at
/// most `MAX_REGEX_ITEMS` items and `MAX_REGEX_BYTES` of item text are kept.
pub(crate) fn record(state: &mut RegexState, name: &str, found: &Found) {
    if found.matches.is_empty() {
        store(state, name, Vec::new());
        return;
    }
    // The character position of every character start, computed once: an item's position is
    // then a lookup instead of a count from the start of the text.
    let mut char_at = vec![0usize; found.text.len() + 1];
    let mut count = 0usize;
    for (byte, _) in found.text.char_indices() {
        char_at[byte] = count;
        count += 1;
    }
    char_at[found.text.len()] = count;
    let mut items = Vec::new();
    let mut bytes = 0usize;
    'matches: for (k, groups) in found.matches.iter().enumerate() {
        for (g, group) in groups.iter().enumerate() {
            if let Some((start, text)) = group {
                if items.len() >= MAX_REGEX_ITEMS || bytes + text.len() > MAX_REGEX_BYTES {
                    break 'matches;
                }
                bytes += text.len();
                items.push(Item {
                    text: text.clone(),
                    pos: char_at[*start] + 1,
                    bytepos: start + 1,
                    group: g,
                    match_no: k + 1,
                });
            }
        }
    }
    store(state, name, items);
}

fn store(state: &mut RegexState, name: &str, items: Vec<Item>) {
    let key = name.to_ascii_lowercase();
    if !state.results.contains_key(&key) {
        if state.order.len() >= KEEP_NAMES {
            if let Some(old) = state.order.pop_front() {
                state.results.remove(&old);
            }
        }
        state.order.push_back(key.clone());
    }
    state.results.insert(key, items);
}

/// Splits the arguments of a call with an optional leading name: a name is
/// present when there is one argument more than `base`.
fn with_name(args: &[String], base: usize) -> Result<(String, &[String]), CallError> {
    if args.len() == base + 1 {
        Ok((args[0].clone(), &args[1..]))
    } else if args.len() == base {
        Ok((DEFAULT_NAME.to_string(), args))
    } else {
        Err(CallError::Arguments(format!(
            "the call takes {base} or {} parameters, got {}",
            base + 1,
            args.len()
        )))
    }
}

/// `$regex([name], text, pattern)`: the number of matches. A pattern the engine
/// refuses answers 0 and sets the error text.
pub(crate) fn regex_call(state: &mut RegexState, args: &[String]) -> Result<String, CallError> {
    let (name, rest) = with_name(args, 2)?;
    match find(&rest[0], &rest[1]) {
        Ok(found) => {
            record(state, &name, &found);
            Ok(found.matches.len().to_string())
        }
        Err(message) => {
            state.last_error = message;
            store(state, &name, Vec::new());
            Ok("0".to_string())
        }
    }
}

/// `$regml([name], N)` and `$regmlex([name], M, N)`.
pub(crate) fn ident(
    state: &RegexState,
    name: &str,
    args: &[String],
    prop: Option<&str>,
) -> Result<String, CallError> {
    let empty = Vec::new();
    match name {
        "regml" => {
            let (regex_name, rest) = with_name(args, 1)?;
            let n: usize = rest[0]
                .trim()
                .parse()
                .map_err(|_| CallError::Arguments("$regml needs an N of 0 or more".into()))?;
            let items = state
                .results
                .get(&regex_name.to_ascii_lowercase())
                .unwrap_or(&empty);
            if n == 0 {
                return Ok(items.len().to_string());
            }
            let Some(item) = items.get(n - 1) else {
                return Ok(String::new());
            };
            Ok(match prop {
                None => item.text.clone(),
                Some("pos") => item.pos.to_string(),
                Some("bytepos") => item.bytepos.to_string(),
                Some("group") => item.group.to_string(),
                Some("match") => item.match_no.to_string(),
                Some(p) => {
                    return Err(CallError::Arguments(format!(
                        "$regml has no property '{p}'"
                    )))
                }
            })
        }
        "regmlex" => {
            // With three arguments the first is the name; with two, they are M and N.
            let (regex_name, m, nth) = match args.len() {
                3 => (args[0].clone(), &args[1], &args[2]),
                2 => (DEFAULT_NAME.to_string(), &args[0], &args[1]),
                _ => {
                    return Err(CallError::Arguments(
                        "$regmlex takes [name], M and N".into(),
                    ))
                }
            };
            let m: usize = m
                .trim()
                .parse()
                .map_err(|_| CallError::Arguments("$regmlex needs an M of 1 or more".into()))?;
            let nth: usize = nth
                .trim()
                .parse()
                .map_err(|_| CallError::Arguments("$regmlex needs an N of 0 or more".into()))?;
            let items = state
                .results
                .get(&regex_name.to_ascii_lowercase())
                .unwrap_or(&empty);
            Ok(items
                .iter()
                .find(|i| i.match_no == m && i.group == nth)
                .map_or(String::new(), |i| i.text.clone()))
        }
        other => Err(CallError::Arguments(format!(
            "${other} is not a regular expression identifier"
        ))),
    }
}

/// Records an empty result for a name, after a failed call that has to leave one.
pub(crate) fn record_empty(state: &mut RegexState, name: &str) {
    store(state, name, Vec::new());
}

/// Appends `s` unless the result would pass `max_bytes`. Returns None when it would.
fn push_within(out: &mut String, s: &str, max_bytes: usize) -> Option<()> {
    if out.len() + s.len() > max_bytes {
        return None;
    }
    out.push_str(s);
    Some(())
}

/// The substitution of `$regsub`: `\N` for a group and `\\` for a backslash.
pub(crate) fn expand_plain(subtext: &str, groups: &Groups, max_bytes: usize) -> Option<String> {
    let mut out = String::new();
    let mut chars = subtext.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            push_within(&mut out, c.encode_utf8(&mut [0u8; 4]), max_bytes)?;
            continue;
        }
        match chars.peek().copied() {
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                let g = d.to_digit(10).unwrap_or(0) as usize;
                if let Some(Some((_, text))) = groups.get(g) {
                    push_within(&mut out, text, max_bytes)?;
                }
            }
            Some('\\') => {
                chars.next();
                push_within(&mut out, "\\", max_bytes)?;
            }
            _ => push_within(&mut out, "\\", max_bytes)?,
        }
    }
    Some(out)
}

/// Replaces the `$regsubex` markers in the plain text of a subtext.
pub(crate) fn replace_markers(s: &str, k: usize, groups: &Groups, max_bytes: usize) -> Option<String> {
    let group_text = |g: usize| {
        groups
            .get(g)
            .and_then(|x| x.as_ref())
            .map(|(_, t)| t.as_str())
    };
    let joined = |sep: &str| {
        groups[1..]
            .iter()
            .filter_map(|x| x.as_ref().map(|(_, t)| t.as_str()))
            .collect::<Vec<_>>()
            .join(sep)
    };
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            push_within(&mut out, c.encode_utf8(&mut [0u8; 4]), max_bytes)?;
            continue;
        }
        match chars.peek().copied() {
            Some('n') => {
                chars.next();
                push_within(&mut out, &k.to_string(), max_bytes)?;
            }
            Some('t') => {
                chars.next();
                push_within(&mut out, group_text(0).unwrap_or(""), max_bytes)?;
            }
            Some('a') => {
                chars.next();
                push_within(&mut out, &joined(" "), max_bytes)?;
            }
            Some('A') => {
                chars.next();
                push_within(&mut out, &joined(""), max_bytes)?;
            }
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                let g = d.to_digit(10).unwrap_or(0) as usize;
                if g == 0 {
                    let captured = groups[1..].iter().filter(|x| x.is_some()).count();
                    push_within(&mut out, &captured.to_string(), max_bytes)?;
                } else {
                    push_within(&mut out, group_text(g).unwrap_or(""), max_bytes)?;
                }
            }
            // `\\` is one backslash, as in `$regsub` (the help page's escape).
            Some('\\') => {
                chars.next();
                push_within(&mut out, "\\", max_bytes)?;
            }
            _ => push_within(&mut out, "\\", max_bytes)?,
        }
    }
    Some(out)
}

/// `$regsub` without the variable write: the substituted text and the count. `None` when the
/// text would grow past `max_bytes`; the text is not built past that point.
pub(crate) fn plain_replace(found: &Found, subtext: &str, max_bytes: usize) -> Option<(usize, String)> {
    let mut out = String::new();
    let mut at = 0;
    for groups in &found.matches {
        let Some((start, whole)) = groups[0].clone() else {
            continue;
        };
        let used = out.len() + (start - at);
        let Some(room) = max_bytes.checked_sub(used) else {
            return None;
        };
        let Some(piece) = expand_plain(subtext, groups, room) else {
            return None;
        };
        out.push_str(&found.text[at..start]);
        out.push_str(&piece);
        at = start + whole.len();
    }
    if out.len() + (found.text.len() - at) > max_bytes {
        return None;
    }
    out.push_str(&found.text[at..]);
    Some((found.matches.len(), out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn count_and_items() {
        let mut st = RegexState::default();
        assert_eq!(
            regex_call(&mut st, &v(&["xabyab", "/a(b)/g"])).unwrap(),
            "2"
        );
        assert_eq!(ident(&st, "regml", &v(&["0"]), None).unwrap(), "4");
        assert_eq!(ident(&st, "regml", &v(&["1"]), None).unwrap(), "ab");
        // The group "b" of "xabyab" starts after "xa", so its position is 3.
        assert_eq!(ident(&st, "regml", &v(&["2"]), Some("pos")).unwrap(), "3");
        assert_eq!(ident(&st, "regml", &v(&["2"]), Some("group")).unwrap(), "1");
        assert_eq!(ident(&st, "regml", &v(&["3"]), Some("match")).unwrap(), "2");
        assert_eq!(ident(&st, "regmlex", &v(&["1", "1"]), None).unwrap(), "b");
    }

    #[test]
    fn modifiers_change_matching() {
        let mut st = RegexState::default();
        assert_eq!(
            regex_call(&mut st, &v(&["hello", "/HELLO/i"])).unwrap(),
            "1"
        );
        assert_eq!(regex_call(&mut st, &v(&["hello", "/HELLO/"])).unwrap(), "0");
        assert_eq!(regex_call(&mut st, &v(&["aaa", "/a/g"])).unwrap(), "3");
        assert_eq!(regex_call(&mut st, &v(&["aaa", "/a/"])).unwrap(), "1");
        assert_eq!(regex_call(&mut st, &v(&["ab", "/b/A"])).unwrap(), "0");
    }

    #[test]
    fn bad_patterns_answer_zero_and_keep_the_error() {
        let mut st = RegexState::default();
        assert_eq!(regex_call(&mut st, &v(&["aa", "/(a)\\1/"])).unwrap(), "0");
        assert!(!st.last_error.is_empty());
        assert_eq!(regex_call(&mut st, &v(&["a", "/a/q"])).unwrap(), "0");
        assert!(st.last_error.contains("'q'"));
    }

    #[test]
    fn plain_substitution_uses_group_numbers() {
        let found = find("John Smith", "/(\\w+) (\\w+)/").unwrap();
        let (n, out) = plain_replace(&found, "\\2, \\1", usize::MAX).unwrap();
        assert_eq!((n, out.as_str()), (1, "Smith, John"));
    }

    #[test]
    fn plain_substitution_stops_at_the_byte_budget() {
        let found = find("aaaa", "/a/g").unwrap();
        assert_eq!(plain_replace(&found, "bb", 8), Some((4, "bbbbbbbb".to_string())));
        assert_eq!(plain_replace(&found, "bb", 7), None);
    }

    #[test]
    fn item_positions_count_characters() {
        // The 'a' is the second character, though its byte offset is 2.
        let found = find("éa", "/a/g").unwrap();
        let mut st = RegexState::default();
        record(&mut st, "n", &found);
        assert_eq!(st.results["n"][0].pos, 2);
    }

    #[test]
    fn expansion_stops_at_the_byte_budget() {
        let groups: Groups = vec![Some((0, "abc".to_string()))];
        assert_eq!(expand_plain("\\0\\0", &groups, 6).as_deref(), Some("abcabc"));
        assert_eq!(expand_plain("\\0\\0", &groups, 5), None);
        assert_eq!(replace_markers("\\t\\t", 1, &groups, 6).as_deref(), Some("abcabc"));
        assert_eq!(replace_markers("\\t\\t", 1, &groups, 5), None);
    }

    #[test]
    fn markers_for_regsubex() {
        let found = find("ab", "/(a)(b)/").unwrap();
        let groups = &found.matches[0];
        assert_eq!(
            replace_markers("\\n:\\t:\\a:\\A:\\0:\\1", 1, groups, usize::MAX).unwrap(),
            "1:ab:a b:ab:2:a"
        );
    }
}
