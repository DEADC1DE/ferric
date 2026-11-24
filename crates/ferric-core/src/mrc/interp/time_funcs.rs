//! Time and date identifiers, batch three: `$time`, `$date`, `$gmt`, `$ctime`,
//! `$fulldate`, `$day` and `$asctime`, with the optional format argument.
//!
//! Semantics follow the scripting reference `time_date_identifiers`.
//!
//! Ferric choices (the help gives no default there, or no exact output):
//! * `$time` without a format is `HH:nn:ss` and `$date` is `dd/mm/yyyy`,
//!   both in local time.
//! * `$fulldate` is `ddd mmm dd HH:nn:ss yyyy`, which matches the help example
//!   `Wed Jun 26 21:41:02 1996`. `$asctime(N)` without a format uses it too.
//! * `z`, `zz` and `zzz` give the zone offset as `+1`, `+0100` and `+0100 GMT`;
//!   the minutes of a half-hour zone are only in `zz` and `zzz`.
//! * `oo` gives the ordinal suffix of the day (`st`, `nd`, `rd`, `th`), not the
//!   day number; write `d oo` for `26 th`.
//! * `$timezone` is the standard offset in seconds west of UTC, and `$daylight`
//!   the extra seconds of daylight time now, 0 otherwise. Standard time is the
//!   smaller of the January and July offsets of the current year.
//! * `$ctime(text)`, which parses a date text, is not implemented and stops the run.
//! * Local time comes from the system time zone through `chrono`. The clock is
//!   read once per identifier evaluation.

use super::text_funcs::CallError;

const NAMES: &[&str] = &[
    "time", "date", "gmt", "ctime", "fulldate", "day", "asctime", "timezone", "daylight",
];
const FULLDATE: &str = "ddd mmm dd HH:nn:ss yyyy";
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Format items, longest first within each length so that greedy matching works.
const TOKENS: &[&str] = &[
    "yyyy", "mmmm", "dddd", "mmm", "ddd", "zzz", "yy", "mm", "dd", "hh", "HH", "nn", "ss", "tt",
    "TT", "oo", "zz", "m", "d", "h", "H", "n", "s", "t", "T", "z",
];

/// True for the names this module answers.
pub(crate) fn is_time(name: &str) -> bool {
    NAMES.contains(&name)
}

/// One reading of the clock: seconds since the epoch in UTC, the local offset
/// from UTC in seconds now, and the standard (non-daylight) offset in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Clock {
    pub(crate) utc: i64,
    pub(crate) offset: i32,
    pub(crate) standard: i32,
}

impl Clock {
    pub(crate) fn now() -> Clock {
        use chrono::{Datelike, TimeZone};
        let utc = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let now = chrono::Local::now();
        let offset = now.offset().local_minus_utc();
        let year = now.year();
        // Standard time is the smaller of the January and July offsets.
        let at = |month: u32| {
            chrono::Local
                .with_ymd_and_hms(year, month, 1, 12, 0, 0)
                .single()
                .map_or(offset, |t| t.offset().local_minus_utc())
        };
        let standard = at(1).min(at(7));
        Clock {
            utc,
            offset,
            standard,
        }
    }
}

/// Calendar fields of one moment, in the zone of `offset`.
/// `weekday` counts from Sunday = 0.
struct Fields {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    weekday: usize,
    offset: i32,
}

/// `local_secs` is the time in the zone of `offset`: UTC seconds plus `offset`.
fn fields(local_secs: i64, offset: i32) -> Fields {
    let days = local_secs.div_euclid(86_400);
    let rem = local_secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil(days);
    Fields {
        year,
        month,
        day,
        hour: rem / 3600,
        minute: (rem % 3600) / 60,
        second: rem % 60,
        weekday: (days + 4).rem_euclid(7) as usize,
        offset,
    }
}

/// `+` or `-` for a zone offset.
fn sign(offset: i32) -> char {
    if offset < 0 {
        '-'
    } else {
        '+'
    }
}

/// The English ordinal suffix of a day: 1st, 2nd, 3rd, 4th, 11th, 21st.
fn ordinal_suffix(day: u32) -> &'static str {
    match (day % 100, day % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

/// Year, month and day of a day count since 1970-01-01 (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe as i64 + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Replaces each format item in `pattern` with its value; other text stays.
fn format(f: &Fields, pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().take(4).collect();
        match TOKENS.iter().copied().find(|t| rest.starts_with(*t)) {
            Some(token) => {
                out.push_str(&value(f, token));
                i += token.len();
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn value(f: &Fields, token: &str) -> String {
    let hour12 = if f.hour % 12 == 0 { 12 } else { f.hour % 12 };
    let pm = f.hour >= 12;
    let month = MONTHS[f.month as usize - 1];
    let day = DAYS[f.weekday];
    match token {
        "yyyy" => format!("{:04}", f.year),
        "yy" => format!("{:02}", f.year.rem_euclid(100)),
        "mmmm" => month.to_string(),
        "mmm" => month[..3].to_string(),
        "mm" => format!("{:02}", f.month),
        "m" => f.month.to_string(),
        "dddd" => day.to_string(),
        "ddd" => day[..3].to_string(),
        "dd" => format!("{:02}", f.day),
        "d" => f.day.to_string(),
        "hh" => format!("{hour12:02}"),
        "h" => hour12.to_string(),
        "HH" => format!("{:02}", f.hour),
        "H" => f.hour.to_string(),
        "nn" => format!("{:02}", f.minute),
        "n" => f.minute.to_string(),
        "ss" => format!("{:02}", f.second),
        "s" => f.second.to_string(),
        "tt" => (if pm { "pm" } else { "am" }).to_string(),
        "TT" => (if pm { "PM" } else { "AM" }).to_string(),
        "t" => (if pm { "p" } else { "a" }).to_string(),
        "T" => (if pm { "P" } else { "A" }).to_string(),
        "oo" => ordinal_suffix(f.day).to_string(),
        "z" => format!("{}{}", sign(f.offset), f.offset.abs() / 3600),
        "zz" => format!(
            "{}{:02}{:02}",
            sign(f.offset),
            f.offset.abs() / 3600,
            (f.offset.abs() % 3600) / 60
        ),
        "zzz" => format!("{} GMT", value(f, "zz")),
        _ => String::new(),
    }
}

/// Answers one time identifier. `args` are the evaluated parameters.
pub(crate) fn call(name: &str, args: &[String], clock: &Clock) -> Result<String, CallError> {
    let local = fields(clock.utc + i64::from(clock.offset), clock.offset);
    let gmt = fields(clock.utc, 0);
    let wrong =
        |want: &str| CallError::Arguments(format!("${name} takes {want}, got {}", args.len()));
    let out = match (name, args.len()) {
        ("time", 0) => format(&local, "HH:nn:ss"),
        ("time", 1) => format(&local, &args[0]),
        ("date", 0) => format(&local, "dd/mm/yyyy"),
        ("date", 1) => format(&local, &args[0]),
        ("gmt", 0) | ("ctime", 0) => clock.utc.to_string(),
        ("gmt", 1) => format(&gmt, &args[0]),
        ("fulldate", 0) => format(&local, FULLDATE),
        ("day", 0) => DAYS[local.weekday].to_string(),
        // Ferric choice: `$timezone` is seconds west of UTC, as the C library
        // `timezone` variable and the Windows bias are; `$daylight` is the extra
        // seconds of daylight time now, 0 otherwise.
        ("timezone", 0) => (-clock.standard).to_string(),
        ("daylight", 0) => (clock.offset - clock.standard).max(0).to_string(),
        ("asctime", 1) => match args[0].trim().parse::<i64>() {
            Ok(n) => format(&fields(n + i64::from(clock.offset), clock.offset), FULLDATE),
            Err(_) => format(&local, &args[0]),
        },
        ("asctime", 2) => {
            let n = args[0]
                .trim()
                .parse::<i64>()
                .map_err(|_| CallError::Arguments(format!("'{}' is not a time value", args[0])))?;
            format(&fields(n + i64::from(clock.offset), clock.offset), &args[1])
        }
        ("ctime", 1) => {
            return Err(CallError::Arguments(
                "$ctime(text) is not implemented; run stopped".into(),
            ))
        }
        ("time" | "date" | "gmt", _) => return Err(wrong("0 or 1 parameter")),
        ("fulldate" | "day" | "ctime" | "timezone" | "daylight", _) => {
            return Err(wrong("no parameter"))
        }
        ("asctime", _) => return Err(wrong("1 or 2 parameters")),
        _ => {
            return Err(CallError::Arguments(format!(
                "${name} is not a time identifier"
            )))
        }
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1996-06-26 21:41:02 UTC, a Wednesday.
    const WED: i64 = 835_825_262;

    fn at(utc: i64, offset: i32) -> Clock {
        Clock {
            utc,
            offset,
            standard: offset,
        }
    }

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(9673), (1996, 6, 26));
        assert_eq!(civil(11016), (2000, 2, 29));
        assert_eq!(fields(WED, 0).weekday, 3);
    }

    #[test]
    fn help_example_for_fulldate() {
        assert_eq!(
            call("fulldate", &[], &at(WED, 0)).unwrap(),
            "Wed Jun 26 21:41:02 1996"
        );
    }

    #[test]
    fn local_offset_shifts_the_clock_fields() {
        assert_eq!(
            call("time", &v(&["HH:nn:ss"]), &at(WED, 3600)).unwrap(),
            "22:41:02"
        );
        assert_eq!(call("time", &[], &at(WED, 0)).unwrap(), "21:41:02");
        assert_eq!(call("date", &[], &at(WED, 0)).unwrap(), "26/06/1996");
    }

    #[test]
    fn format_items() {
        let c = at(WED, 0);
        assert_eq!(call("time", &v(&["h:nn tt"]), &c).unwrap(), "9:41 pm");
        assert_eq!(call("time", &v(&["h:nn TT"]), &c).unwrap(), "9:41 PM");
        assert_eq!(
            call("time", &v(&["mmmm d, yyyy"]), &c).unwrap(),
            "June 26, 1996"
        );
        assert_eq!(call("date", &v(&["dddd"]), &c).unwrap(), "Wednesday");
        assert_eq!(call("day", &[], &c).unwrap(), "Wednesday");
    }

    #[test]
    fn plain_numbers_and_gmt() {
        let c = at(WED, 7200);
        assert_eq!(call("ctime", &[], &c).unwrap(), WED.to_string());
        assert_eq!(call("gmt", &[], &c).unwrap(), WED.to_string());
        assert_eq!(call("gmt", &v(&["HH:nn"]), &c).unwrap(), "21:41");
    }

    #[test]
    fn asctime_takes_a_time_value_and_a_format() {
        let c = at(WED, 0);
        assert_eq!(
            call("asctime", &v(&["835825262", "dd/mm/yy"]), &c).unwrap(),
            "26/06/96"
        );
        assert_eq!(
            call("asctime", &v(&["835825262"]), &c).unwrap(),
            "Wed Jun 26 21:41:02 1996"
        );
        assert!(call("asctime", &v(&["HH:nn"]), &c).unwrap().contains(':'));
    }

    #[test]
    fn timezone_and_daylight_are_seconds() {
        let winter = Clock {
            utc: WED,
            offset: 3600,
            standard: 3600,
        };
        assert_eq!(call("timezone", &[], &winter).unwrap(), "-3600");
        assert_eq!(call("daylight", &[], &winter).unwrap(), "0");
        let summer = Clock {
            utc: WED,
            offset: 7200,
            standard: 3600,
        };
        assert_eq!(call("daylight", &[], &summer).unwrap(), "3600");
        assert_eq!(call("timezone", &[], &summer).unwrap(), "-3600");
        assert!(matches!(
            call("daylight", &v(&["x"]), &summer),
            Err(CallError::Arguments(_))
        ));
    }

    #[test]
    fn the_system_clock_gives_a_plausible_zone() {
        let now = Clock::now();
        let timezone: i64 = call("timezone", &[], &now).unwrap().parse().unwrap();
        assert!(timezone.abs() <= 14 * 3600 && timezone % 900 == 0);
        let daylight: i64 = call("daylight", &[], &now).unwrap().parse().unwrap();
        assert!((0..=2 * 3600).contains(&daylight));
    }

    #[test]
    fn zone_format_items() {
        let at_offset = |offset| Clock {
            utc: WED,
            offset,
            standard: offset,
        };
        let f = "z|zz|zzz";
        assert_eq!(
            call("time", &v(&[f]), &at_offset(3600)).unwrap(),
            "+1|+0100|+0100 GMT"
        );
        assert_eq!(
            call("time", &v(&[f]), &at_offset(-18000)).unwrap(),
            "-5|-0500|-0500 GMT"
        );
        assert_eq!(
            call("time", &v(&[f]), &at_offset(0)).unwrap(),
            "+0|+0000|+0000 GMT"
        );
        assert_eq!(
            call("time", &v(&["z|zz"]), &at_offset(19800)).unwrap(),
            "+5|+0530"
        );
    }

    #[test]
    fn ordinal_suffixes() {
        let on_day = |day: i64| Clock {
            utc: (day - 1) * 86_400 + 12 * 3_600,
            offset: 0,
            standard: 0,
        };
        let suffix = |day: i64| call("date", &v(&["oo"]), &on_day(day)).unwrap();
        assert_eq!(suffix(1), "st");
        assert_eq!(suffix(2), "nd");
        assert_eq!(suffix(3), "rd");
        assert_eq!(suffix(4), "th");
        assert_eq!(suffix(11), "th");
        assert_eq!(suffix(12), "th");
        assert_eq!(suffix(13), "th");
        assert_eq!(suffix(21), "st");
        assert_eq!(suffix(22), "nd");
        assert_eq!(suffix(23), "rd");
    }

    #[test]
    fn unsupported_and_wrong_calls_are_errors() {
        let c = at(WED, 0);
        assert!(matches!(
            call("ctime", &v(&["3rd August 1987"]), &c),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("day", &v(&["x"]), &c),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("asctime", &[], &c),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            call("asctime", &v(&["x", "HH"]), &c),
            Err(CallError::Arguments(_))
        ));
    }
}
