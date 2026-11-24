//! Condition parser for `if`, `elseif` and `while` (milestone M1).
//!
//! Grammar, lowest binding first: `a || b`, `a && b`, `! a`, `( group )`,
//! and a comparison `left operator right`. Operands stay raw text: the
//! operator is found in the source before any value is substituted, so a
//! value can never add an operator.
//!
//! Operators of M1: `== === != < > <= >=`. The other if/then/else operators
//! are recognised and reported as unsupported instead of being read as text. An operand with no
//! operator is a truth test.

const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cond {
    Or(Vec<Cond>),
    And(Vec<Cond>),
    Not(Box<Cond>),
    Cmp { left: String, op: Op, right: String },
    /// A lone operand.
    Truth(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// `==`: ASCII case-insensitive text compare.
    Eq,
    /// `===`: exact text compare.
    EqCs,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    /// `iswm`: wildcard match, ASCII case-insensitive.
    IsWm,
    /// `iswmcs`: wildcard match, case-sensitive.
    IsWmCs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CondError {
    Empty,
    Unbalanced,
    TooDeep,
    /// Trailing text after a complete condition.
    Trailing,
    /// A known script operator that M1 does not implement.
    Unsupported(String),
}

enum Word {
    Op(Op),
    Unsupported,
    Text,
}

fn classify(w: &str) -> Word {
    match w {
        "==" => return Word::Op(Op::Eq),
        "===" => return Word::Op(Op::EqCs),
        "!=" => return Word::Op(Op::Ne),
        "<" => return Word::Op(Op::Lt),
        ">" => return Word::Op(Op::Gt),
        "<=" => return Word::Op(Op::Le),
        ">=" => return Word::Op(Op::Ge),
        "//" | "\\\\" | "&" | "!==" => return Word::Unsupported,
        _ => {}
    }
    match w.to_ascii_lowercase().as_str() {
        "iswm" => Word::Op(Op::IsWm),
        "iswmcs" => Word::Op(Op::IsWmCs),
        "isin" | "isincs" | "isnum" | "isletter" | "islower" | "isupper"
        | "ison" | "isop" | "ishop" | "isvoice" | "isreg" | "ischan" | "isaop" | "isavoice"
        | "isignore" | "isprotect" | "isnotify" | "isban" | "ischanban" => Word::Unsupported,
        _ => Word::Text,
    }
}

/// Parses a condition, with or without the outer parentheses.
pub(crate) fn parse(text: &str) -> Result<Cond, CondError> {
    let mut p = P { s: text, i: 0 };
    let c = p.or(0)?;
    p.skip_ws();
    if p.i < p.s.len() {
        return Err(CondError::Trailing);
    }
    Ok(c)
}

struct P<'a> {
    s: &'a str,
    i: usize,
}

impl P<'_> {
    fn skip_ws(&mut self) {
        let b = self.s.as_bytes();
        while self.i < b.len() && b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, tok: &str) -> bool {
        self.skip_ws();
        if self.s[self.i..].starts_with(tok) {
            self.i += tok.len();
            true
        } else {
            false
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.as_bytes().get(self.i).copied()
    }

    fn or(&mut self, depth: usize) -> Result<Cond, CondError> {
        let mut v = vec![self.and(depth)?];
        while self.eat("||") {
            v.push(self.and(depth)?);
        }
        Ok(if v.len() == 1 { v.remove(0) } else { Cond::Or(v) })
    }

    fn and(&mut self, depth: usize) -> Result<Cond, CondError> {
        let mut v = vec![self.unary(depth)?];
        while self.eat("&&") {
            v.push(self.unary(depth)?);
        }
        Ok(if v.len() == 1 { v.remove(0) } else { Cond::And(v) })
    }

    fn unary(&mut self, depth: usize) -> Result<Cond, CondError> {
        match self.peek() {
            Some(b'!') => {
                // `!` is a prefix here; an operand cannot start with an operator.
                if depth >= MAX_DEPTH {
                    return Err(CondError::TooDeep);
                }
                self.i += 1;
                Ok(Cond::Not(Box::new(self.unary(depth + 1)?)))
            }
            Some(b'(') => {
                if depth >= MAX_DEPTH {
                    return Err(CondError::TooDeep);
                }
                self.i += 1;
                let c = self.or(depth + 1)?;
                if self.peek() != Some(b')') {
                    return Err(CondError::Unbalanced);
                }
                self.i += 1;
                Ok(c)
            }
            _ => self.comparison(),
        }
    }

    /// Reads raw operand text up to `&&`, `||` or an unmatched `)`.
    fn comparison(&mut self) -> Result<Cond, CondError> {
        let b = self.s.as_bytes();
        let start = self.i;
        let mut d = 0usize;
        let mut i = start;
        while i < b.len() {
            match b[i] {
                b'(' => d += 1,
                b')' => {
                    if d == 0 {
                        break;
                    }
                    d -= 1;
                }
                b'&' | b'|' if d == 0 && i + 1 < b.len() && b[i + 1] == b[i] => break,
                _ => {}
            }
            i += 1;
        }
        self.i = i;
        let seg = self.s[start..i].trim();
        if seg.is_empty() {
            return Err(CondError::Empty);
        }
        // First whitespace separated word, outside parentheses, that is an operator.
        let sb = seg.as_bytes();
        let mut d = 0usize;
        let mut w_start: Option<usize> = None;
        let mut k = 0usize;
        while k <= sb.len() {
            let c = sb.get(k).copied();
            let is_space = d == 0 && c.map_or(true, |c| c.is_ascii_whitespace());
            if is_space {
                if let Some(ws) = w_start.take() {
                    let word = &seg[ws..k];
                    match classify(word) {
                        Word::Op(op) => {
                            return Ok(Cond::Cmp {
                                left: seg[..ws].trim().to_string(),
                                op,
                                right: seg[k..].trim().to_string(),
                            });
                        }
                        Word::Unsupported => {
                            return Err(CondError::Unsupported(word.to_string()));
                        }
                        Word::Text => {}
                    }
                }
            } else if w_start.is_none() {
                w_start = Some(k);
            }
            match c {
                Some(b'(') => d += 1,
                Some(b')') => d = d.saturating_sub(1),
                _ => {}
            }
            k += 1;
        }
        Ok(Cond::Truth(seg.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmp(l: &str, op: Op, r: &str) -> Cond {
        Cond::Cmp {
            left: l.into(),
            op,
            right: r.into(),
        }
    }

    #[test]
    fn simple_comparisons_with_and_without_parentheses() {
        assert_eq!(parse("($1 == 1)").unwrap(), cmp("$1", Op::Eq, "1"));
        assert_eq!(parse("$1 == 1").unwrap(), cmp("$1", Op::Eq, "1"));
        assert_eq!(parse("(%x != a b)").unwrap(), cmp("%x", Op::Ne, "a b"));
        assert_eq!(parse("(%x >= 3)").unwrap(), cmp("%x", Op::Ge, "3"));
        assert_eq!(parse("(a === B)").unwrap(), cmp("a", Op::EqCs, "B"));
        assert_eq!(parse("(%x == )").unwrap(), cmp("%x", Op::Eq, ""));
    }

    #[test]
    fn truth_test() {
        assert_eq!(parse("(1)").unwrap(), Cond::Truth("1".into()));
        assert_eq!(parse("(%flag)").unwrap(), Cond::Truth("%flag".into()));
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(
            parse("(1 == 1 || 2 == 2 && 3 == 3)").unwrap(),
            Cond::Or(vec![
                cmp("1", Op::Eq, "1"),
                Cond::And(vec![cmp("2", Op::Eq, "2"), cmp("3", Op::Eq, "3")]),
            ])
        );
    }

    #[test]
    fn not_and_groups() {
        assert_eq!(
            parse("(!(1 < 2) && (3 > 2))").unwrap(),
            Cond::And(vec![
                Cond::Not(Box::new(cmp("1", Op::Lt, "2"))),
                cmp("3", Op::Gt, "2"),
            ])
        );
    }

    #[test]
    fn operator_inside_parentheses_of_an_operand_is_ignored() {
        assert_eq!(
            parse("($foo(a == b) == 1)").unwrap(),
            cmp("$foo(a == b)", Op::Eq, "1")
        );
    }

    #[test]
    fn unsupported_operators_are_reported() {
        assert_eq!(
            parse("(a isin abc)"),
            Err(CondError::Unsupported("isin".into()))
        );
        // `iswm` is a supported operator now; `isnum` is not.
        assert_eq!(
            parse("(a ISWM b*)"),
            Ok(Cond::Cmp {
                left: "a".into(),
                op: Op::IsWm,
                right: "b*".into(),
            })
        );
        assert_eq!(
            parse("(a isnum b)"),
            Err(CondError::Unsupported("isnum".into()))
        );
        assert_eq!(parse("(a // b)"), Err(CondError::Unsupported("//".into())));
    }

    #[test]
    fn malformed_conditions() {
        assert_eq!(parse(""), Err(CondError::Empty));
        assert_eq!(parse("()"), Err(CondError::Empty));
        assert_eq!(parse("(1 == 1"), Err(CondError::Unbalanced));
        assert_eq!(parse("(1 == 1))"), Err(CondError::Trailing));
        assert_eq!(parse("(1 == 1) &&"), Err(CondError::Empty));
    }

    #[test]
    fn nesting_is_capped() {
        let ok = format!("{}1{}", "(".repeat(16), ")".repeat(16));
        assert!(parse(&ok).is_ok());
        let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
        assert_eq!(parse(&deep), Err(CondError::TooDeep));
        let nots = format!("{}1", "!".repeat(5000));
        assert_eq!(parse(&nots), Err(CondError::TooDeep));
    }
}
