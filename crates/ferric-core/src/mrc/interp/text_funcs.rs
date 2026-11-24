//! Built-in text identifiers, batch one: `$len`, `$left`, `$right`, `$mid`,
//! `$upper`, `$lower`, `$str`, `$replace`, `$gettok` and `$numtok`.
//!
//! Semantics follow the scripting reference `text_number_identifiers` and
//! `token_identifiers`. Negative counts and positions are refused with an
//! error instead of guessed.
//!
//! Ferric choices:
//! * `$replace` matches ASCII letters case-insensitively. `$replacecs` is not
//!   implemented.
//! * `$numtok` and `$gettok` treat an empty token between two separators as a
//!   token.

/// Why a call failed. `Arguments` ends the run with a notice; `TooLong` maps to
/// the string length limit.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CallError {
    Arguments(String),
    TooLong,
}

/// Splits the raw argument text of an identifier call at top-level commas.
/// Commas inside nested `(...)` belong to the nested call.
pub(crate) fn split_call_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in raw.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Evaluates one batch-one identifier on already evaluated arguments.
/// `max_bytes` is the string limit the result must stay within.
pub(crate) fn call(name: &str, args: &[String], max_bytes: usize) -> Result<String, CallError> {
    let want = |n: usize| -> Result<(), CallError> {
        if args.len() == n {
            Ok(())
        } else {
            Err(CallError::Arguments(format!(
                "${name} takes {n} parameter(s), got {}",
                args.len()
            )))
        }
    };
    let out = match name {
        "len" => {
            want(1)?;
            args[0].chars().count().to_string()
        }
        "upper" => {
            want(1)?;
            args[0].to_uppercase()
        }
        "lower" => {
            want(1)?;
            args[0].to_lowercase()
        }
        "left" | "right" => {
            want(2)?;
            let n = count(&args[1])?;
            let chars: Vec<char> = args[0].chars().collect();
            let len = chars.len() as i64;
            // Negative N means all but |N| characters.
            let keep = (if n < 0 { (len + n).max(0) } else { n.min(len) }) as usize;
            if name == "left" {
                chars[..keep].iter().collect()
            } else {
                chars[chars.len() - keep..].iter().collect()
            }
        }
        "mid" => {
            want(3)?;
            let start = count(&args[1])?;
            let n = count(&args[2])?;
            if start < 1 || n < 0 {
                return Err(CallError::Arguments(
                    "$mid needs a start of at least 1 and a count of 0 or more".into(),
                ));
            }
            let chars: Vec<char> = args[0].chars().collect();
            let from = (start - 1) as usize;
            if from >= chars.len() {
                String::new()
            } else {
                // N zero means: to the end of the text.
                let to = if n == 0 {
                    chars.len()
                } else {
                    (from + n as usize).min(chars.len())
                };
                chars[from..to].iter().collect()
            }
        }
        "str" => {
            want(2)?;
            let n = count(&args[1])?;
            if n < 0 {
                return Err(CallError::Arguments(
                    "$str needs a count of 0 or more".into(),
                ));
            }
            if args[0].len().saturating_mul(n as usize) > max_bytes {
                return Err(CallError::TooLong);
            }
            args[0].repeat(n as usize)
        }
        "replace" => {
            // text, then one or more (substring, newstring) pairs.
            if args.len() < 3 || args.len() % 2 == 0 {
                return Err(CallError::Arguments(
                    "$replace takes a text and one or more pairs".into(),
                ));
            }
            let mut text = args[0].clone();
            for pair in args[1..].chunks(2) {
                text = replace_ci(&text, &pair[0], &pair[1], max_bytes)?;
            }
            text
        }
        "numtok" => {
            want(2)?;
            let sep = separator(&args[1])?;
            if args[0].is_empty() {
                "0".to_string()
            } else {
                args[0].split(sep).count().to_string()
            }
        }
        "gettok" => {
            want(3)?;
            let sep = separator(&args[2])?;
            let (from, to) = token_range(&args[1])?;
            let tokens: Vec<&str> = args[0].split(sep).collect();
            let end = to.unwrap_or(tokens.len()).min(tokens.len());
            if from == 0 || from > end {
                String::new()
            } else {
                let joiner = sep.to_string();
                tokens[from - 1..end].join(joiner.as_str())
            }
        }
        other if super::math_funcs::is_math(other) => super::math_funcs::call(other, args)?,
        other if super::token_funcs::is_token(other) => super::token_funcs::call(other, args, max_bytes)?,
        other if super::string_funcs::is_string(other) => super::string_funcs::call(other, args)?,
        "address" => {
            return Err(CallError::Arguments(
                "$address(nick,type) is not implemented; run stopped".into(),
            ));
        }
        other => {
            return Err(CallError::Arguments(format!(
                "${other} is not a text identifier"
            )));
        }
    };
    if out.len() > max_bytes {
        return Err(CallError::TooLong);
    }
    Ok(out)
}

pub(crate) fn count(s: &str) -> Result<i64, CallError> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| CallError::Arguments(format!("'{s}' is not a number")))
}

/// Separator from a character code.b,2,46)`).
pub(crate) fn separator(s: &str) -> Result<char, CallError> {
    let code = count(s)?;
    u32::try_from(code)
        .ok()
        .and_then(char::from_u32)
        .filter(|c| *c != '\0')
        .ok_or_else(|| CallError::Arguments(format!("'{s}' is not a separator code")))
}

/// Token spec `N`, `N-` or `N-M`. Returns the 1-based start and the optional end.
pub(crate) fn token_range(spec: &str) -> Result<(usize, Option<usize>), CallError> {
    let bad = || CallError::Arguments(format!("'{spec}' is not a token range"));
    let part = |t: &str| -> Result<usize, CallError> {
        let n = count(t).map_err(|_| bad())?;
        usize::try_from(n).map_err(|_| bad())
    };
    match spec.split_once('-') {
        None => {
            let n = part(spec)?;
            Ok((n, Some(n)))
        }
        Some((from, "")) => Ok((part(from)?, None)),
        Some((from, to)) => Ok((part(from)?, Some(part(to)?))),
    }
}

/// Replaces every occurrence of `needle` (ASCII case-insensitive, left to right,
/// no overlap) with `with`.
fn replace_ci(text: &str, needle: &str, with: &str, max_bytes: usize) -> Result<String, CallError> {
    if needle.is_empty() {
        return Ok(text.to_string());
    }
    let hay: Vec<char> = text.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < hay.len() {
        let hit = i + pat.len() <= hay.len()
            && hay[i..i + pat.len()]
                .iter()
                .zip(&pat)
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if hit {
            out.push_str(with);
            i += pat.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
        if out.len() > max_bytes {
            return Err(CallError::TooLong);
        }
    }
    Ok(out)
}

/// `$rand(v1,v2)`: a random integer in v1..=v2, or a random letter in v1..=v2
/// when both are single ASCII letters. `state` is the generator state.
/// Ferric choices: the result is not cryptographic (`$rands` is not
/// implemented), and a reversed range is an error.
pub(crate) fn rand(args: &[String], state: &mut u64) -> Result<String, CallError> {
    if args.len() != 2 {
        return Err(CallError::Arguments(format!(
            "$rand takes 2 parameters, got {}",
            args.len()
        )));
    }
    let (lo, hi) = (args[0].trim(), args[1].trim());
    if let (Ok(a), Ok(b)) = (lo.parse::<i64>(), hi.parse::<i64>()) {
        if a > b {
            return Err(CallError::Arguments(
                "$rand: the first number is greater than the second".into(),
            ));
        }
        return Ok(pick(a as i128, b as i128, state).to_string());
    }
    let letter = |s: &str| s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic();
    if letter(lo) && letter(hi) {
        let (a, b) = (lo.as_bytes()[0] as i128, hi.as_bytes()[0] as i128);
        if a > b {
            return Err(CallError::Arguments(
                "$rand: the first letter is after the second".into(),
            ));
        }
        return Ok((pick(a, b, state) as u8 as char).to_string());
    }
    Err(CallError::Arguments(
        "$rand takes two numbers or two letters".into(),
    ))
}

/// A value in lo..=hi. The range is never empty: callers check lo <= hi.
fn pick(lo: i128, hi: i128, state: &mut u64) -> i128 {
    let span = (hi - lo + 1) as u128;
    lo + (next_u64(state) as u128 % span) as i128
}

/// xorshift64*. The state must not be zero; `Runtime` seeds it odd.
pub(crate) fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn split_keeps_nested_commas_together() {
        assert_eq!(split_call_args("a,b"), vec!["a", "b"]);
        assert_eq!(split_call_args("$x(1,2),c"), vec!["$x(1,2)", "c"]);
        assert_eq!(split_call_args(""), vec![""]);
    }

    #[test]
    fn help_examples() {
        let m = 10_340;
        assert_eq!(call("len", &v(&["#Ferric"]), m).unwrap(), "7");
        assert_eq!(call("left", &v(&["goodbye", "4"]), m).unwrap(), "good");
        assert_eq!(call("left", &v(&["goodbye", "-2"]), m).unwrap(), "goodb");
        assert_eq!(call("right", &v(&["othello", "5"]), m).unwrap(), "hello");
        assert_eq!(call("mid", &v(&["othello", "3", "4"]), m).unwrap(), "hell");
        assert_eq!(call("mid", &v(&["othello", "3", "0"]), m).unwrap(), "hello");
        assert_eq!(call("upper", &v(&["hello"]), m).unwrap(), "HELLO");
        assert_eq!(call("lower", &v(&["HELLO"]), m).unwrap(), "hello");
        assert_eq!(call("str", &v(&["ho", "3"]), m).unwrap(), "hohoho");
        assert_eq!(
            call("replace", &v(&["abcdefg", "cd", "xyz"]), m).unwrap(),
            "abxyzefg"
        );
        assert_eq!(
            call(
                "replace",
                &v(&["abcdefg", "a", "A", "b", "B", "c", "C", "d", "D"]),
                m
            )
            .unwrap(),
            "ABCDefg"
        );
        assert_eq!(
            call("gettok", &v(&["a.b.c.d.e", "3", "46"]), m).unwrap(),
            "c"
        );
        assert_eq!(
            call("gettok", &v(&["a.b.c.d.e", "9", "46"]), m).unwrap(),
            ""
        );
        assert_eq!(
            call("gettok", &v(&["a.b.c.d.e", "2-", "46"]), m).unwrap(),
            "b.c.d.e"
        );
        assert_eq!(
            call("gettok", &v(&["a.b.c.d.e", "2-4", "46"]), m).unwrap(),
            "b.c.d"
        );
        assert_eq!(call("numtok", &v(&["a.b.c", "46"]), m).unwrap(), "3");
        assert_eq!(call("numtok", &v(&["", "46"]), m).unwrap(), "0");
    }

    #[test]
    fn bad_arguments_are_errors_not_guesses() {
        let m = 10_340;
        assert!(matches!(
            call("len", &v(&["a", "b"]), m),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("mid", &v(&["a", "-1", "2"]), m),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("str", &v(&["a", "-1"]), m),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("gettok", &v(&["a", "x", "46"]), m),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("replace", &v(&["a", "b"]), m),
            Err(CallError::Arguments(_))
        ));
    }

    #[test]
    fn results_respect_the_string_limit() {
        assert_eq!(call("str", &v(&["abc", "10"]), 10), Err(CallError::TooLong));
        assert_eq!(
            call("replace", &v(&["a", "a", "bbbbbbbb"]), 4),
            Err(CallError::TooLong)
        );
    }
}
