//! Token identifiers, batch four: `$addtok`, `$deltok`, `$findtok`, `$instok`,
//! `$istok`, `$puttok`, `$remtok` and `$reptok`.
//!
//! Semantics follow the scripting reference `token_identifiers`. Tokens are split
//! by the separator character code, as `$gettok` splits them. Token matching
//! is ASCII case-insensitive, as `$istok` is; the case-sensitive `$istokcs`
//! family is not implemented.
//!
//! Ferric choices (the help gives no error case):
//! * An empty text has no tokens.
//! * A position outside the text stops the run, except that `$instok` at a
//!   position past the end appends, as the help example shows.
//! * `$istok` answers 1 or 0, because the interpreter has no boolean type.

use super::text_funcs::{count, separator, token_range, CallError};

const NAMES: &[&str] = &[
    "addtok", "deltok", "findtok", "instok", "istok", "puttok", "remtok", "reptok",
];

/// True for the names this module answers.
pub(crate) fn is_token(name: &str) -> bool {
    NAMES.contains(&name)
}

fn split(text: &str, sep: char) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(sep).map(str::to_string).collect()
    }
}

fn join(tokens: &[String], sep: char) -> String {
    let joiner = sep.to_string();
    tokens.join(joiner.as_str())
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn position(s: &str) -> Result<i64, CallError> {
    count(s)
}

/// Answers one token identifier. `args` are the evaluated parameters.
pub(crate) fn call(name: &str, args: &[String], max_bytes: usize) -> Result<String, CallError> {
    let arity = |want: usize| -> Result<(), CallError> {
        if args.len() == want {
            Ok(())
        } else {
            Err(CallError::Arguments(format!(
                "${name} takes {want} parameters, got {}",
                args.len()
            )))
        }
    };
    let out = match name {
        "addtok" => {
            arity(3)?;
            let sep = separator(&args[2])?;
            let mut tokens = split(&args[0], sep);
            if !tokens.iter().any(|t| same(t, &args[1])) {
                tokens.push(args[1].clone());
            }
            join(&tokens, sep)
        }
        "istok" => {
            arity(3)?;
            let sep = separator(&args[2])?;
            let found = split(&args[0], sep).iter().any(|t| same(t, &args[1]));
            (if found { "1" } else { "0" }).to_string()
        }
        "findtok" => {
            arity(4)?;
            let sep = separator(&args[3])?;
            let nth = position(&args[2])?;
            if nth < 1 {
                return Err(CallError::Arguments(
                    "$findtok needs an N of 1 or more".into(),
                ));
            }
            let tokens = split(&args[0], sep);
            let mut seen = 0i64;
            let mut found = String::new();
            for (i, t) in tokens.iter().enumerate() {
                if same(t, &args[1]) {
                    seen += 1;
                    if seen == nth {
                        found = (i + 1).to_string();
                        break;
                    }
                }
            }
            found
        }
        "deltok" => {
            arity(3)?;
            let sep = separator(&args[2])?;
            let (from, to) = token_range(&args[1])?;
            let mut tokens = split(&args[0], sep);
            let end = to.unwrap_or(tokens.len()).min(tokens.len());
            if from >= 1 && from <= end {
                tokens.drain(from - 1..end);
            }
            join(&tokens, sep)
        }
        "instok" => {
            arity(4)?;
            let sep = separator(&args[3])?;
            let nth = position(&args[2])?;
            if nth < 1 {
                return Err(CallError::Arguments(
                    "$instok needs an N of 1 or more".into(),
                ));
            }
            let mut tokens = split(&args[0], sep);
            let at = (nth as usize - 1).min(tokens.len());
            tokens.insert(at, args[1].clone());
            join(&tokens, sep)
        }
        "puttok" => {
            arity(4)?;
            let sep = separator(&args[3])?;
            let nth = position(&args[2])?;
            let mut tokens = split(&args[0], sep);
            let len = tokens.len() as i64;
            // A negative N counts from the end: -1 is the last token.
            let index = if nth > 0 { nth - 1 } else { len + nth };
            if nth == 0 || index < 0 || index >= len {
                return Err(CallError::Arguments(format!("$puttok has no token {nth}")));
            }
            tokens[index as usize] = args[1].clone();
            join(&tokens, sep)
        }
        "remtok" => {
            arity(4)?;
            let sep = separator(&args[3])?;
            let nth = position(&args[2])?;
            if nth < 0 {
                return Err(CallError::Arguments(
                    "$remtok needs an N of 0 or more".into(),
                ));
            }
            let tokens = split(&args[0], sep);
            let mut seen = 0i64;
            let kept: Vec<String> = tokens
                .into_iter()
                .filter(|t| {
                    if same(t, &args[1]) {
                        seen += 1;
                        // N = 0 removes every match; otherwise only the Nth.
                        !(nth == 0 || seen == nth)
                    } else {
                        true
                    }
                })
                .collect();
            join(&kept, sep)
        }
        "reptok" => {
            arity(5)?;
            let sep = separator(&args[4])?;
            let nth = position(&args[3])?;
            if nth < 0 {
                return Err(CallError::Arguments(
                    "$reptok needs an N of 0 or more".into(),
                ));
            }
            let mut tokens = split(&args[0], sep);
            let mut seen = 0i64;
            let mut replaced = vec![false; tokens.len()];
            for (i, t) in tokens.iter().enumerate() {
                if same(t, &args[1]) {
                    seen += 1;
                    replaced[i] = nth == 0 || seen == nth;
                }
            }
            // The size of the result is worked out before anything is built: a short text with a long
            // replacement must not allocate far past the string limit.
            let size: usize = tokens
                .iter()
                .zip(&replaced)
                .map(|(t, &r)| if r { args[2].len() } else { t.len() })
                .sum::<usize>()
                + tokens.len().saturating_sub(1) * sep.len_utf8();
            if size > max_bytes {
                return Err(CallError::TooLong);
            }
            for (t, &r) in tokens.iter_mut().zip(&replaced) {
                if r {
                    *t = args[2].clone();
                }
            }
            join(&tokens, sep)
        }
        _ => {
            return Err(CallError::Arguments(format!(
                "${name} is not a token identifier"
            )))
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests check results, not the size budget: no limit here.
    fn call(name: &str, args: &[String]) -> Result<String, CallError> {
        super::call(name, args, usize::MAX)
    }

    #[test]
    fn reptok_refuses_a_result_past_the_limit() {
        let args = v(&["a.b.c.d", "b", "xxxxxxxxxx", "0", "46"]);
        // "a.xxxxxxxxxx.c.d" is 16 bytes.
        assert_eq!(super::call("reptok", &args, 12), Err(CallError::TooLong));
        assert!(super::call("reptok", &args, 40).is_ok());
    }

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_examples() {
        assert_eq!(
            call("addtok", &v(&["a.b.c", "d", "46"])).unwrap(),
            "a.b.c.d"
        );
        assert_eq!(
            call("addtok", &v(&["a.b.c.d", "c", "46"])).unwrap(),
            "a.b.c.d"
        );
        assert_eq!(
            call("deltok", &v(&["a.b.c.d", "3", "46"])).unwrap(),
            "a.b.d"
        );
        assert_eq!(
            call("deltok", &v(&["a.b.c.d", "2-3", "46"])).unwrap(),
            "a.d"
        );
        assert_eq!(
            call("findtok", &v(&["a.b.c.d", "c", "1", "46"])).unwrap(),
            "3"
        );
        assert_eq!(
            call("findtok", &v(&["a.b.c.d", "e", "1", "46"])).unwrap(),
            ""
        );
        assert_eq!(
            call("instok", &v(&["a.b.d", "c", "3", "46"])).unwrap(),
            "a.b.c.d"
        );
        assert_eq!(
            call("instok", &v(&["a.b.d", "c", "9", "46"])).unwrap(),
            "a.b.d.c"
        );
        assert_eq!(call("istok", &v(&["a.b.c", "B", "46"])).unwrap(), "1");
        assert_eq!(call("istok", &v(&["a.b.c", "e", "46"])).unwrap(), "0");
        assert_eq!(
            call("puttok", &v(&["a.b.c.d", "e", "2", "46"])).unwrap(),
            "a.e.c.d"
        );
        assert_eq!(
            call("puttok", &v(&["a.b.c.d", "e", "-1", "46"])).unwrap(),
            "a.b.c.e"
        );
        assert_eq!(
            call("remtok", &v(&["a.b.c.d", "b", "1", "46"])).unwrap(),
            "a.c.d"
        );
        assert_eq!(
            call("remtok", &v(&["a.b.c.d", "e", "1", "46"])).unwrap(),
            "a.b.c.d"
        );
        assert_eq!(
            call("reptok", &v(&["a.b.c.d", "b", "e", "1", "46"])).unwrap(),
            "a.e.c.d"
        );
        assert_eq!(
            call("reptok", &v(&["a.b.c.d", "f", "e", "1", "46"])).unwrap(),
            "a.b.c.d"
        );
    }

    #[test]
    fn zero_applies_to_every_match() {
        assert_eq!(
            call("remtok", &v(&["a.b.a.b", "a", "0", "46"])).unwrap(),
            "b.b"
        );
        assert_eq!(
            call("reptok", &v(&["a.b.a", "a", "x", "0", "46"])).unwrap(),
            "x.b.x"
        );
    }

    #[test]
    fn errors_stop_instead_of_guessing() {
        assert!(matches!(
            call("findtok", &v(&["a", "a", "0", "46"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("puttok", &v(&["a.b", "x", "3", "46"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("addtok", &v(&["a", "b"])),
            Err(CallError::Arguments(_))
        ));
    }
}
