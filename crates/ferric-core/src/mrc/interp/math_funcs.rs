//! Numeric identifiers, batch four: `$abs`, `$int`, `$floor`, `$ceil`, `$round`,
//! `$sqrt`, `$pi`, `$gcd`, `$lcm`, `$min`, `$max` and the bit and logic
//! identifiers `$and`, `$or`, `$xor`, `$not`, `$biton`, `$bitoff`, `$isbit`.
//!
//! Semantics follow the scripting reference `text_number_identifiers`.
//!
//! Ferric choices (the help gives no output format or error case):
//! * A whole number is printed without a decimal point; any other value in its
//!   shortest decimal form. `$round` drops trailing zeros.
//! * Values that are not numbers, a negative `$sqrt`, a decimal count outside
//!   0 to 15 and bit positions outside 0 to 63 stop the run with a notice.
//! * `$gcd`, `$lcm` and the bit identifiers take integers only.
//! * Not in this batch: the trigonometric, logarithmic and hyperbolic
//!   functions and the `.property` forms (`$sin(1).deg`, `$min(...).alpha`).

use super::text_funcs::CallError;

/// `$pi` to 20 decimal places, as the help states.
pub(crate) const PI: &str = "3.14159265358979323846";

const NAMES: &[&str] = &[
    "abs", "int", "floor", "ceil", "round", "sqrt", "gcd", "lcm", "min", "max", "and", "or", "xor",
    "not", "biton", "bitoff", "isbit",
];

/// True for the names this module answers.
pub(crate) fn is_math(name: &str) -> bool {
    NAMES.contains(&name)
}

fn number(s: &str) -> Result<f64, CallError> {
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|x| x.is_finite())
        .ok_or_else(|| CallError::Arguments(format!("'{s}' is not a number")))
}

fn integer(s: &str) -> Result<i64, CallError> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| CallError::Arguments(format!("'{s}' is not an integer")))
}

/// Text of a number: whole numbers without a point, others in shortest form.
fn text(x: f64) -> String {
    let x = if x == 0.0 { 0.0 } else { x };
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Answers one numeric identifier. `args` are the evaluated parameters.
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
    let out = match name {
        "abs" => {
            arity(1, 1)?;
            text(number(&args[0])?.abs())
        }
        "int" => {
            arity(1, 1)?;
            text(number(&args[0])?.trunc())
        }
        "floor" => {
            arity(1, 1)?;
            text(number(&args[0])?.floor())
        }
        "ceil" => {
            arity(1, 1)?;
            text(number(&args[0])?.ceil())
        }
        "round" => {
            arity(1, 2)?;
            let x = number(&args[0])?;
            let decimals = match args.get(1) {
                Some(d) => integer(d)?,
                None => 0,
            };
            if !(0..=15).contains(&decimals) {
                return Err(CallError::Arguments("$round takes 0 to 15 decimals".into()));
            }
            let factor = 10f64.powi(decimals as i32);
            text((x * factor).round() / factor)
        }
        "sqrt" => {
            arity(1, 1)?;
            let x = number(&args[0])?;
            if x < 0.0 {
                return Err(CallError::Arguments("$sqrt of a negative number".into()));
            }
            text(x.sqrt())
        }
        "gcd" => {
            arity(1, usize::MAX)?;
            let mut g: i128 = 0;
            for a in args {
                g = gcd(g, i128::from(integer(a)?));
            }
            g.to_string()
        }
        "lcm" => {
            arity(1, usize::MAX)?;
            let mut l: i128 = 1;
            for a in args {
                let v = i128::from(integer(a)?).abs();
                if v == 0 {
                    l = 0;
                } else if l != 0 {
                    l = (l / gcd(l, v))
                        .checked_mul(v)
                        .ok_or_else(|| CallError::Arguments("$lcm is too large".into()))?;
                }
            }
            l.to_string()
        }
        "min" | "max" => {
            arity(1, usize::MAX)?;
            let mut values = args.iter().map(|a| number(a));
            let first = values.next().ok_or_else(|| {
                CallError::Arguments(format!("${name} needs at least one number"))
            })??;
            let pick = if name == "min" { f64::min } else { f64::max };
            let mut best = first;
            for v in values {
                best = pick(best, v?);
            }
            text(best)
        }
        "and" | "or" | "xor" => {
            arity(2, 2)?;
            let (a, b) = (integer(&args[0])?, integer(&args[1])?);
            let r = match name {
                "and" => a & b,
                "or" => a | b,
                _ => a ^ b,
            };
            r.to_string()
        }
        "not" => {
            arity(1, 1)?;
            (!integer(&args[0])?).to_string()
        }
        "biton" | "bitoff" | "isbit" => {
            arity(2, 2)?;
            let a = integer(&args[0])?;
            let n = integer(&args[1])?;
            if !(0..=63).contains(&n) {
                return Err(CallError::Arguments(format!(
                    "${name} takes a bit position from 0 to 63"
                )));
            }
            let mask = 1i64 << n;
            match name {
                "biton" => (a | mask).to_string(),
                "bitoff" => (a & !mask).to_string(),
                _ => i64::from(a & mask != 0).to_string(),
            }
        }
        _ => {
            return Err(CallError::Arguments(format!(
                "${name} is not a numeric identifier"
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
        assert_eq!(call("abs", &v(&["-1"])).unwrap(), "1");
        assert_eq!(call("int", &v(&["3.14159"])).unwrap(), "3");
        assert_eq!(call("int", &v(&["-3.7"])).unwrap(), "-3");
        assert_eq!(call("round", &v(&["3.14159", "2"])).unwrap(), "3.14");
        assert_eq!(call("round", &v(&["2.5"])).unwrap(), "3");
        assert_eq!(call("floor", &v(&["-1.5"])).unwrap(), "-2");
        assert_eq!(call("ceil", &v(&["1.2"])).unwrap(), "2");
        assert_eq!(call("sqrt", &v(&["16"])).unwrap(), "4");
        assert_eq!(call("gcd", &v(&["12", "18"])).unwrap(), "6");
        assert_eq!(call("lcm", &v(&["4", "6"])).unwrap(), "12");
        assert_eq!(call("min", &v(&["3", "1", "2"])).unwrap(), "1");
        assert_eq!(call("max", &v(&["3", "1", "2"])).unwrap(), "3");
        assert_eq!(call("and", &v(&["6", "3"])).unwrap(), "2");
        assert_eq!(call("or", &v(&["6", "3"])).unwrap(), "7");
        assert_eq!(call("xor", &v(&["6", "3"])).unwrap(), "5");
        assert_eq!(call("not", &v(&["0"])).unwrap(), "-1");
        assert_eq!(call("biton", &v(&["0", "3"])).unwrap(), "8");
        assert_eq!(call("bitoff", &v(&["15", "0"])).unwrap(), "14");
        assert_eq!(call("isbit", &v(&["5", "2"])).unwrap(), "1");
        assert_eq!(call("isbit", &v(&["5", "1"])).unwrap(), "0");
        assert!(PI.starts_with("3.14159265358979"));
    }

    #[test]
    fn errors_stop_instead_of_guessing() {
        assert!(matches!(
            call("sqrt", &v(&["-1"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("abs", &v(&["abc"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("gcd", &v(&["1.5"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("isbit", &v(&["1", "64"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("round", &v(&["1", "16"])),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(call("min", &v(&[])), Err(CallError::Arguments(_))));
    }
}
