//! The limits table of the interpreter (plan section 5).
//!
//! Every limit lives here; the evaluator reads nothing else. A limit stop
//! ends the current trigger with one notice and leaves the runtime usable.
//! Notices carry the limit and its value, never script or event data.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Limits {
    /// Statements executed per trigger.
    pub max_statements: u64,
    /// Nested alias calls.
    pub max_call_depth: usize,
    /// Wall clock time per trigger.
    pub wall_time: Duration,
    /// The clock is read once per this many statements.
    pub time_check_every: u64,
    /// Longest string value, in bytes.
    pub max_string_bytes: usize,
    /// Variables, globals plus the locals of the running trigger.
    pub max_variables: usize,
    /// `echo` lines per trigger.
    pub max_echo_lines: usize,
    /// IRC lines a trigger may send (msg, notice, join, mode, ...).
    pub max_outbound: usize,
    /// Nested blocks over all active calls. This bounds Rust stack use; it is
    /// a Ferric addition to the plan table.
    pub max_nesting: usize,
    /// Largest script text `Runtime::load` accepts (the 5 MiB cap of
    /// `script_files.rs`).
    pub max_script_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_statements: 20_000,
            max_call_depth: 16,
            wall_time: Duration::from_millis(250),
            time_check_every: 256,
            max_string_bytes: 64 * 1024,
            max_variables: 10_000,
            max_echo_lines: 200,
            max_outbound: 50,
            max_nesting: 128,
            max_script_bytes: 5 * 1024 * 1024,
        }
    }
}

/// Which limit stopped a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LimitKind {
    Statements,
    CallDepth,
    WallTime,
    StringBytes,
    Variables,
    EchoLines,
    Outbound,
    Nesting,
}

impl LimitKind {
    pub(crate) fn notice(self, l: &Limits) -> String {
        let what = match self {
            LimitKind::Statements => format!("{} statements per trigger", l.max_statements),
            LimitKind::CallDepth => format!("call depth {}", l.max_call_depth),
            LimitKind::WallTime => format!("{} ms wall time per trigger", l.wall_time.as_millis()),
            LimitKind::StringBytes => format!("string value of {} bytes", l.max_string_bytes),
            LimitKind::Variables => format!("{} variables", l.max_variables),
            LimitKind::EchoLines => format!("{} echo lines per trigger", l.max_echo_lines),
            LimitKind::Outbound => format!("{} IRC lines per trigger", l.max_outbound),
            LimitKind::Nesting => format!("block nesting {}", l.max_nesting),
        };
        format!("script limit reached: {what}; run stopped")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_plan_section_5() {
        let l = Limits::default();
        assert_eq!(l.max_statements, 20_000);
        assert_eq!(l.max_call_depth, 16);
        assert_eq!(l.wall_time, Duration::from_millis(250));
        assert_eq!(l.time_check_every, 256);
        assert_eq!(l.max_string_bytes, 65_536);
        assert_eq!(l.max_variables, 10_000);
        assert_eq!(l.max_echo_lines, 200);
        assert_eq!(l.max_outbound, 50);
    }

    #[test]
    fn notices_name_the_limit() {
        let l = Limits::default();
        assert!(LimitKind::Statements.notice(&l).contains("20000 statements"));
        assert!(LimitKind::CallDepth.notice(&l).contains("call depth 16"));
        assert!(LimitKind::WallTime.notice(&l).contains("250 ms"));
        assert!(LimitKind::StringBytes.notice(&l).contains("65536"));
        assert!(LimitKind::Variables.notice(&l).contains("10000 variables"));
        assert!(LimitKind::EchoLines.notice(&l).contains("200 echo lines"));
        assert!(LimitKind::Nesting.notice(&l).contains("nesting 128"));
        assert!(LimitKind::Outbound.notice(&l).contains("50 IRC lines"));
    }
}
