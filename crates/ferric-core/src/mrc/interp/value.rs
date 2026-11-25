//! Small text and number helpers shared by the evaluator.

/// Integer value of `s`; text that is not a whole integer counts as 0 (an
/// unset variable is 0 in arithmetic; the rest is a Ferric choice).
pub(crate) fn to_int(s: &str) -> i64 {
    int_strict(s).unwrap_or(0)
}

pub(crate) fn int_strict(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// Replaces CR, LF and NUL, so an `Echo` is always a single clean line.
pub(crate) fn clean_line(s: String) -> String {
    let bad = |c: char| matches!(c, '\r' | '\n' | '\0');
    if s.contains(bad) {
        s.replace(bad, " ")
    } else {
        s
    }
}

/// Splits evaluated text into parameters at U+0020, skipping empty pieces.
pub(crate) fn split_args(text: &str) -> Vec<String> {
    text.split(' ')
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// First whitespace delimited word and the rest with leading space removed.
pub(crate) fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// A name that is safe to put in a notice: printable ASCII, at most 32 chars.
pub(crate) fn safe_name(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_graphic()).take(32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints() {
        assert_eq!(to_int("42"), 42);
        assert_eq!(to_int(" -7 "), -7);
        assert_eq!(to_int(""), 0);
        assert_eq!(to_int("abc"), 0);
        assert_eq!(to_int("2.5"), 0);
        assert_eq!(int_strict("x"), None);
        assert_eq!(int_strict("99999999999999999999"), None);
    }

    #[test]
    fn lines_are_cleaned() {
        assert_eq!(clean_line("a\r\nb\0c".into()), "a  b c");
        assert_eq!(clean_line("plain".into()), "plain");
    }

    #[test]
    fn args_split_at_single_spaces_only() {
        assert_eq!(split_args("  a   b "), ["a", "b"]);
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("a\tb c"), ["a\tb", "c"]);
    }

    #[test]
    fn words() {
        assert_eq!(split_word("  set   %x  v w"), ("set", "%x  v w"));
        assert_eq!(split_word("one"), ("one", ""));
        assert_eq!(split_word(""), ("", ""));
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("ab\r\n\u{1b}[1mc"), "ab[1mc");
        assert_eq!(safe_name(&"x".repeat(100)).len(), 32);
    }
}
