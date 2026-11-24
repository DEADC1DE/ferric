//! String and character identifiers: `$asc`, `$chr`, `$count`, `$countcs`,
//! `$pos`, `$poscs`, `$remove`, `$removecs`, `$replacex`, `$replacexcs`,
//! `$isnum`, `$islower`, `$isupper`, `$ord`, `$qt`, `$noqt` and `$strip`.
//! The bare `$cr`, `$lf` and `$crlf` are answered by the evaluator.
//!
//! Semantics follow the scripting reference `text_number_identifiers`. Matching is
//! ASCII case-insensitive, except in the `cs` names.
//!
//! Ferric choices (the help gives no error case or edge case):
//! * Matches are counted and replaced left to right, without overlap.
//! * `$pos` counts characters from 1. With N = 0 it gives the number of matches.
//! * `$isnum` without switches accepts digits only. `s` also accepts a leading
//!   sign, `d` a decimal point. Its answer is 1 or 0.
//! * `$strip` removes bold, underline, reverse, italic, strikethrough and color
//!   codes; the hex color code is part of color. The switches `m` and `o` read
//!   the Messages dialog, which is not part of the interpreter, so they stop the run.
//! * `$stripped`, the count of codes removed from a message, is not implemented.
//! * `$asc` takes exactly one character.

use super::text_funcs::CallError;

const NAMES: &[&str] = &[
    "asc",
    "chr",
    "count",
    "countcs",
    "pos",
    "poscs",
    "remove",
    "removecs",
    "replacex",
    "replacexcs",
    "isnum",
    "islower",
    "isupper",
    "ord",
    "qt",
    "noqt",
    "strip",
];

/// True for the names this module answers.
pub(crate) fn is_string(name: &str) -> bool {
    NAMES.contains(&name)
}

fn same(a: char, b: char, cs: bool) -> bool {
    if cs {
        a == b
    } else {
        a.eq_ignore_ascii_case(&b)
    }
}

/// Start indexes of the non-overlapping matches of `needle` in `hay`, left to right.
fn matches(hay: &[char], needle: &[char], cs: bool) -> Vec<usize> {
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| same(*a, *b, cs))
        {
            found.push(i);
            i += needle.len();
        } else {
            i += 1;
        }
    }
    found
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn remove_all(text: &str, needle: &str, cs: bool) -> String {
    let hay = chars(text);
    let pat = chars(needle);
    let mut out = String::new();
    let mut at = 0;
    for start in matches(&hay, &pat, cs) {
        out.extend(&hay[at..start]);
        at = start + pat.len();
    }
    out.extend(&hay[at..]);
    out
}

/// Replaces each pair in turn. Text put in by an earlier pair is not replaced again.
fn replace_x(text: &str, pairs: &[String], cs: bool) -> String {
    let mut segments: Vec<(String, bool)> = vec![(text.to_string(), false)];
    for pair in pairs.chunks(2) {
        let pat = chars(&pair[0]);
        if pat.is_empty() {
            continue;
        }
        let mut next = Vec::new();
        for (segment, protected) in segments {
            if protected {
                next.push((segment, true));
                continue;
            }
            let hay = chars(&segment);
            let mut at = 0;
            for start in matches(&hay, &pat, cs) {
                if start > at {
                    next.push((hay[at..start].iter().collect(), false));
                }
                next.push((pair[1].clone(), true));
                at = start + pat.len();
            }
            if at < hay.len() {
                next.push((hay[at..].iter().collect(), false));
            }
        }
        segments = next;
    }
    segments.into_iter().map(|(s, _)| s).collect()
}

fn count_all(text: &str, needles: &[String], cs: bool) -> usize {
    let hay = chars(text);
    needles
        .iter()
        .map(|n| matches(&hay, &chars(n), cs).len())
        .sum()
}

fn integer(s: &str) -> Result<i64, CallError> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| CallError::Arguments(format!("'{s}' is not an integer")))
}

fn number(s: &str) -> Result<f64, CallError> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| CallError::Arguments(format!("'{s}' is not a number")))
}

fn bool_text(b: bool) -> String {
    (if b { "1" } else { "0" }).to_string()
}

fn ordinal_suffix(n: u64) -> &'static str {
    match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

/// `$isnum(N,sd,I,J)`: a number, with the optional sign and decimal point, and
/// optionally within the range I to J.
fn isnum(args: &[String]) -> Result<String, CallError> {
    if args.len() == 3 {
        return Err(CallError::Arguments(
            "$isnum takes the range as two values, I and J".into(),
        ));
    }
    let switches = args.get(1).map_or("", String::as_str);
    if let Some(bad) = switches
        .chars()
        .find(|c| !matches!(c, 's' | 'S' | 'd' | 'D'))
    {
        return Err(CallError::Arguments(format!(
            "$isnum has no switch '{bad}'"
        )));
    }
    let signed = switches.chars().any(|c| matches!(c, 's' | 'S'));
    let decimal = switches.chars().any(|c| matches!(c, 'd' | 'D'));
    let text = args[0].as_str();
    let body = match text.chars().next() {
        Some('+' | '-') if signed => &text[1..],
        _ => text,
    };
    let mut digits = 0;
    let mut dot = false;
    let mut valid = true;
    for c in body.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == '.' && decimal && !dot {
            dot = true;
        } else {
            valid = false;
            break;
        }
    }
    valid = valid && digits > 0;
    let value = if valid {
        text.parse::<f64>().ok()
    } else {
        None
    };
    let in_range = match (args.get(2), args.get(3)) {
        (Some(lo), Some(hi)) => {
            let (lo, hi) = (number(lo)?, number(hi)?);
            value.is_some_and(|v| v >= lo && v <= hi)
        }
        _ => value.is_some(),
    };
    Ok(bool_text(in_range))
}

/// `$strip(text,switches)`: removes the control codes named by the switches.
pub(crate) fn strip(text: &str, switches: Option<&str>) -> Result<String, CallError> {
    let sw = switches.unwrap_or("buriec");
    if let Some(bad) = sw.chars().find(|c| !"buriec".contains(*c)) {
        return Err(CallError::Arguments(format!(
            "$strip has no switch '{bad}'; m and o read the Messages dialog and are not supported"
        )));
    }
    let want = |c: char| sw.contains(c);
    let text = chars(text);
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        let plain = match c {
            '\u{2}' => want('b'),
            '\u{1F}' => want('u'),
            '\u{16}' => want('r'),
            '\u{1D}' => want('i'),
            '\u{1E}' => want('e'),
            _ => false,
        };
        if plain {
            i += 1;
            continue;
        }
        // Color: \x03 with up to two digits, an optional comma and two more.
        if c == '\u{3}' && want('c') {
            i += 1;
            i = skip_digits(&text, i, 2, |d| d.is_ascii_digit());
            if text.get(i) == Some(&',') && text.get(i + 1).is_some_and(char::is_ascii_digit) {
                i = skip_digits(&text, i + 1, 2, |d| d.is_ascii_digit());
            }
            continue;
        }
        // Hex color: \x04 with up to six hex digits, an optional comma and six more.
        if c == '\u{4}' && want('c') {
            i += 1;
            i = skip_digits(&text, i, 6, |d| d.is_ascii_hexdigit());
            if text.get(i) == Some(&',') && text.get(i + 1).is_some_and(char::is_ascii_hexdigit) {
                i = skip_digits(&text, i + 1, 6, |d| d.is_ascii_hexdigit());
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    Ok(out)
}

/// Index after up to `max` characters from `at` that satisfy `ok`.
fn skip_digits(text: &[char], at: usize, max: usize, ok: impl Fn(&char) -> bool) -> usize {
    let mut i = at;
    while i < text.len() && i - at < max && ok(&text[i]) {
        i += 1;
    }
    i
}

/// Answers one string identifier. `args` are the evaluated parameters.
pub(crate) fn call(name: &str, args: &[String]) -> Result<String, CallError> {
    let arity = |lo: usize, hi: usize| -> Result<(), CallError> {
        if (lo..=hi).contains(&args.len()) {
            Ok(())
        } else {
            Err(CallError::Arguments(format!(
                "${name} takes {lo} to {hi} parameters, got {}",
                args.len()
            )))
        }
    };
    let cs = name.ends_with("cs");
    let out = match name {
        "asc" => {
            arity(1, 1)?;
            let mut it = args[0].chars();
            match (it.next(), it.next()) {
                (Some(c), None) => (c as u32).to_string(),
                _ => {
                    return Err(CallError::Arguments(
                        "$asc takes exactly one character".into(),
                    ))
                }
            }
        }
        "chr" => {
            arity(1, 1)?;
            let n = integer(&args[0])?;
            let c = u32::try_from(n)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| CallError::Arguments(format!("'{n}' is not a character code")))?;
            c.to_string()
        }
        "count" | "countcs" => {
            arity(2, usize::MAX)?;
            count_all(&args[0], &args[1..], cs).to_string()
        }
        "pos" | "poscs" => {
            arity(3, 3)?;
            let nth = integer(&args[2])?;
            if nth < 0 {
                return Err(CallError::Arguments("$pos needs an N of 0 or more".into()));
            }
            let hay = chars(&args[0]);
            let pat = chars(&args[1]);
            let hits = matches(&hay, &pat, cs);
            if nth == 0 {
                hits.len().to_string()
            } else {
                hits.get(nth as usize - 1)
                    .map_or(String::new(), |&i| (i + 1).to_string())
            }
        }
        "remove" | "removecs" => {
            arity(2, usize::MAX)?;
            let mut text = args[0].clone();
            for needle in &args[1..] {
                text = remove_all(&text, needle, cs);
            }
            text
        }
        "replacex" | "replacexcs" => {
            arity(3, usize::MAX)?;
            if args.len() % 2 == 0 {
                return Err(CallError::Arguments(
                    "$replacex takes a text and pairs of substring and newstring".into(),
                ));
            }
            replace_x(&args[0], &args[1..], cs)
        }
        "isnum" => {
            arity(1, 4)?;
            isnum(args)?
        }
        "islower" => {
            arity(1, 1)?;
            bool_text(!args[0].is_empty() && args[0].chars().all(char::is_lowercase))
        }
        "isupper" => {
            arity(1, 1)?;
            bool_text(!args[0].is_empty() && args[0].chars().all(char::is_uppercase))
        }
        "ord" => {
            arity(1, 1)?;
            let n = integer(&args[0])?;
            format!("{n}{}", ordinal_suffix(n.unsigned_abs()))
        }
        "qt" => {
            arity(1, 1)?;
            format!("\"{}\"", args[0])
        }
        "noqt" => {
            arity(1, 1)?;
            let t = args[0].as_str();
            if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
                t[1..t.len() - 1].to_string()
            } else {
                t.to_string()
            }
        }
        "strip" => {
            arity(1, 2)?;
            strip(&args[0], args.get(1).map(String::as_str))?
        }
        other => {
            return Err(CallError::Arguments(format!(
                "${other} is not a string identifier"
            )))
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_examples() {
        assert_eq!(call("asc", &v(&["A"])).unwrap(), "65");
        assert_eq!(call("asc", &v(&["*"])).unwrap(), "42");
        assert_eq!(call("chr", &v(&["65"])).unwrap(), "A");
        assert_eq!(call("count", &v(&["hello", "el"])).unwrap(), "1");
        assert_eq!(call("count", &v(&["hello", "l"])).unwrap(), "2");
        assert_eq!(call("countcs", &v(&["Hello", "l"])).unwrap(), "2");
        assert_eq!(call("count", &v(&["Hello", "L"])).unwrap(), "2");
        assert_eq!(call("countcs", &v(&["Hello", "L"])).unwrap(), "0");
        assert_eq!(call("pos", &v(&["hello there", "e", "1"])).unwrap(), "2");
        assert_eq!(call("pos", &v(&["hello there", "e", "2"])).unwrap(), "9");
        assert_eq!(call("pos", &v(&["hello there", "a", "1"])).unwrap(), "");
        assert_eq!(call("pos", &v(&["hello there", "e", "0"])).unwrap(), "3");
        assert_eq!(call("remove", &v(&["abcdefg", "cd"])).unwrap(), "abefg");
        assert_eq!(
            call("remove", &v(&["abcdefg", "a", "c", "e", "g"])).unwrap(),
            "bdf"
        );
        assert_eq!(
            call("replacex", &v(&["abcdefg", "cd", "xyz"])).unwrap(),
            "abxyzefg"
        );
        assert_eq!(
            call(
                "replacex",
                &v(&["abcdefg", "a", "A", "b", "B", "c", "C", "d", "D"])
            )
            .unwrap(),
            "ABCDefg"
        );
        assert_eq!(
            call("replacex", &v(&["aa", "a", "b", "b", "c"])).unwrap(),
            "bb"
        );
        assert_eq!(call("islower", &v(&["abc"])).unwrap(), "1");
        assert_eq!(call("islower", &v(&["abC"])).unwrap(), "0");
        assert_eq!(call("isupper", &v(&["ABC"])).unwrap(), "1");
        assert_eq!(call("ord", &v(&["26"])).unwrap(), "26th");
        assert_eq!(call("ord", &v(&["3"])).unwrap(), "3rd");
        assert_eq!(call("qt", &v(&["hi"])).unwrap(), "\"hi\"");
        assert_eq!(call("noqt", &v(&["\"hi\""])).unwrap(), "hi");
        assert_eq!(call("noqt", &v(&["hi"])).unwrap(), "hi");
    }

    #[test]
    fn isnum_rules() {
        assert_eq!(call("isnum", &v(&["42"])).unwrap(), "1");
        assert_eq!(call("isnum", &v(&["-4"])).unwrap(), "0");
        assert_eq!(call("isnum", &v(&["-4", "s"])).unwrap(), "1");
        assert_eq!(call("isnum", &v(&["4.5"])).unwrap(), "0");
        assert_eq!(call("isnum", &v(&["4.5", "d"])).unwrap(), "1");
        assert_eq!(call("isnum", &v(&["5", "", "1", "10"])).unwrap(), "1");
        assert_eq!(call("isnum", &v(&["11", "", "1", "10"])).unwrap(), "0");
        assert!(matches!(
            call("isnum", &v(&["5", "1", "2"])),
            Err(CallError::Arguments(_))
        ));
    }

    #[test]
    fn strip_removes_codes() {
        let bold = "\u{2}";
        let color = "\u{3}04,12";
        let text = format!("{bold}a{color}b{bold}");
        assert_eq!(call("strip", &v(&[&text])).unwrap(), "ab");
        assert_eq!(
            call("strip", &v(&[&text, "c"])).unwrap(),
            format!("{bold}ab{bold}")
        );
        assert!(matches!(
            call("strip", &v(&["x", "m"])),
            Err(CallError::Arguments(_))
        ));
    }
}
