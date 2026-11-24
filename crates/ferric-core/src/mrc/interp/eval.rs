//! Statement evaluator (milestone M1).
//!
//! Walks the statement tree of an alias. It performs no I/O: output goes into
//! `effects` (data) and `notices` (text). Every error ends the whole trigger
//! with one notice; `halt` ends it silently.
//!
//! Evaluated text is never parsed again: statements are split by the parser
//! and operators are found in the source text before values are substituted.
//!
//! Choices where the rules gave no answer (marked Ferric choice):
//! * `echo` switches (`-a`, `-s`, ...) are dropped, the target is not modelled.
//! * Locals belong to one alias call; callees do not see them.
//! * A `goto` finds a label in its own block or an enclosing block of the same
//!   alias; it never jumps into a nested block or out of an alias.
//! * `return` ignores its value (identifiers like `$result` arrive in M3).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::irc_effects::{self, BroadcastKind, BroadcastScope, IrcEffect};
use super::limits::{LimitKind, Limits};
use super::runtime::{wildcard_match_mode, EchoTarget, Effect, HaltStatus, ScriptContext, Scripts};
use super::text_funcs::{self, CallError};
use super::time_funcs;
use super::timers::{Timer, MIN_PERIOD};
use super::value::{clean_line, int_strict, safe_name, split_args, split_word, to_int};
use crate::mrc::cond::{self, Cond, CondError, Op};
use crate::mrc::expr::{self, ArithError, Part};
use crate::mrc::{Statement, StatementKind};

/// What a statement tells its block to do next.
pub(crate) enum Flow {
    Next,
    Break,
    Continue,
    Return,
    Halt,
    Goto(String),
}

/// Why a trigger ended early.
pub(crate) enum Stop {
    Limit(LimitKind),
    Error(String),
}

type R<T> = Result<T, Stop>;

fn fail<T>(msg: impl Into<String>) -> R<T> {
    Err(Stop::Error(msg.into()))
}

fn limit<T>(kind: LimitKind) -> R<T> {
    Err(Stop::Limit(kind))
}

/// The nick part of a ban mask (`nick!user@host`), when it has no wildcard.
fn ban_nick(mask: &str) -> Option<&str> {
    let (nick, _) = mask.split_once('!')?;
    (!nick.is_empty() && !nick.contains(['*', '?'])).then_some(nick)
}

/// The host part of a `user@host` address; empty when there is none.
fn host_of(address: Option<&str>) -> &str {
    address.and_then(|a| a.split_once('@')).map_or("", |(_, host)| host)
}

/// Ends the run for a failed built-in identifier call.
fn call_stop(e: CallError) -> Stop {
    match e {
        CallError::Arguments(msg) => Stop::Error(format!("{msg}; run stopped")),
        CallError::TooLong => Stop::Limit(LimitKind::StringBytes),
    }
}

#[derive(Default)]
struct Frame {
    args: Vec<String>,
    locals: HashMap<String, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Local,
    Global,
}

pub(crate) struct Exec<'a> {
    scripts: &'a Scripts,
    globals: &'a mut HashMap<String, String>,
    limits: &'a Limits,
    /// Generator state for `$rand`, owned by the `Runtime`.
    rng: &'a mut u64,
    /// Hash tables of the `Runtime`.
    hashes: &'a mut super::hash_tables::Hashes,
    /// Results of the regular expression calls, owned by the `Runtime`.
    regex: &'a mut super::regex_funcs::RegexState,
    context: ScriptContext,
    halt_status: HaltStatus,
    frames: Vec<Frame>,
    /// Names of the aliases being run, outermost first.
    stack: Vec<String>,
    statements: u64,
    start: Instant,
    nesting: usize,
    echo_lines: usize,
    /// IRC effects made by this trigger, against `Limits::max_outbound`.
    outbound: usize,
    unknown_idents: HashSet<String>,
    effects: Vec<Effect>,
    notices: Vec<String>,
    /// The timers of the runtime; `/timer` adds to them and `$timer` reads them.
    pub(crate) timers: &'a mut super::timers::TimerTable,
    /// The time this run started; timers made here are due from it.
    pub(crate) now: Instant,
    /// The script the run belongs to: the owner of the timers it makes.
    pub(crate) owner: String,
    /// The clock of a timer body, read when the timer was made. `None` reads the clock now.
    pub(crate) frozen: Option<time_funcs::Clock>,
    /// The context `$!name` reads: the timer's context with our live nick.
    pub(crate) live: Option<ScriptContext>,
    /// True once a limit or an error stopped the run.
    pub(crate) stopped: bool,
    /// The answers the caller gave to the prompts (`$?`) of the run, in order.
    pub(crate) answers: Vec<String>,
    /// How many answers the run has used.
    pub(crate) answer_index: usize,
}

fn find_label(stmts: &[Statement], label: &str) -> Option<usize> {
    stmts
        .iter()
        .position(|s| s.kind == StatementKind::Label && s.name == label)
}

/// Parses leading `-letters` words (`echo -a text`) from raw echo text.
fn parse_echo_switches(args: &str) -> (EchoTarget, &str) {
    let mut rest = args.trim_start();
    let mut target = EchoTarget::Default;
    loop {
        let (w, after) = split_word(rest);
        if w.is_empty() {
            return (target, rest);
        }
        if w.len() >= 2 && w.starts_with('-') && w[1..].chars().all(|c| c.is_ascii_alphabetic()) {
            for c in w[1..].chars() {
                match c {
                    's' | 'S' => target = EchoTarget::Status,
                    'a' | 'A' => target = EchoTarget::Active,
                    _ => {}
                }
            }
            rest = after;
            continue;
        }
        if w.chars().all(|c| c.is_ascii_digit()) {
            let (next_w, _) = split_word(after);
            if next_w.starts_with('-')
                && next_w.len() >= 2
                && next_w[1..].chars().all(|c| c.is_ascii_alphabetic())
            {
                rest = after;
                continue;
            }
        }
        break;
    }
    (target, rest)
}

/// Leading `-l` / `-g` switches of a variable command.
fn take_flags(args: &str) -> R<(Option<Scope>, &str)> {
    let mut rest = args.trim_start();
    let mut scope = None;
    loop {
        let (w, after) = split_word(rest);
        if w.len() < 2 || !w.starts_with('-') {
            return Ok((scope, rest));
        }
        for c in w[1..].chars() {
            match c {
                'l' => scope = Some(Scope::Local),
                'g' => scope = Some(Scope::Global),
                _ => return fail("unsupported variable switch; run stopped"),
            }
        }
        rest = after;
    }
}

fn var_word(w: &str) -> R<String> {
    match w.strip_prefix('%') {
        Some(n) if !n.is_empty() && expr::var_name_len(n) == n.len() => Ok(n.to_ascii_lowercase()),
        _ => fail("expected a variable name such as %name; run stopped"),
    }
}

fn arith_error(e: ArithError) -> Stop {
    Stop::Error(match e {
        ArithError::DivByZero => "division by zero; run stopped".to_string(),
        ArithError::Overflow => "number outside the 64-bit range; run stopped".to_string(),
    })
}

/// The mode letters and parameters of a MODE line are balanced:
/// when every mode letter is `o`, `h` or `v`, the letters and the parameters are both cut to the smaller
/// count. The gate of that branch is not established; the rule is applied to every such line.
fn balance_modes(modes: &str) -> String {
    let mut parts = modes.split_whitespace();
    let Some(letters) = parts.next() else {
        return modes.to_string();
    };
    let params: Vec<&str> = parts.collect();
    let mut count = 0usize;
    let mut only_ohv = true;
    for c in letters.chars().filter(|c| *c != '+' && *c != '-') {
        count += 1;
        only_ohv &= matches!(c, 'o' | 'h' | 'v');
    }
    if !only_ohv || count == 0 || params.is_empty() {
        return modes.to_string();
    }
    let keep = count.min(params.len());
    let mut kept = String::new();
    let mut seen = 0usize;
    for c in letters.chars() {
        if c == '+' || c == '-' {
            kept.push(c);
        } else if seen < keep {
            kept.push(c);
            seen += 1;
        }
    }
    // A sign with no letter after it is dropped.
    let kept = kept.trim_end_matches(['+', '-']).to_string();
    let mut out = kept;
    for p in params.iter().take(keep) {
        out.push(' ');
        out.push_str(p);
    }
    out
}

/// True when the source of a `/set` value has an operator of its own. `+`, `*` and `/` count inside a
/// word (`5/2`); `-` only as a word of its own, so a date such as 2026-11-21 stays text. Words that
/// begin with `$` or `%` are identifiers or variables and do not count, so `$1+` is not arithmetic.
fn is_arithmetic_source(raw: &str) -> bool {
    raw.split_whitespace()
        .filter(|w| !w.starts_with('$') && !w.starts_with('%'))
        .any(|w| w.contains(['+', '*', '/']) || w == "-")
}

/// The value `/set` stores. Arithmetic on numbers is computed on the scalar path
///: six decimal places, trailing zeros removed, and division by zero gives 0.
/// A plain number or any other text is stored as written.
fn set_value(text: String) -> String {
    if text.trim().parse::<f64>().is_ok() {
        return text;
    }
    match super::float_calc::compute_scalar(&text) {
        Some(value) if value == 0.0 => "0".to_string(),
        Some(value) => format!("{value:.6}").trim_end_matches('0').trim_end_matches('.').to_string(),
        None => text,
    }
}

fn compare(op: Op, l: &str, r: &str) -> bool {
    match op {
        Op::Eq => l.eq_ignore_ascii_case(r),
        Op::EqCs => l == r,
        Op::IsWm => wildcard_match_mode(r, l, false),
        Op::IsWmCs => wildcard_match_mode(r, l, true),
        Op::Ne => !l.eq_ignore_ascii_case(r),
        _ => match (int_strict(l), int_strict(r)) {
            (Some(a), Some(b)) => match op {
                Op::Lt => a < b,
                Op::Gt => a > b,
                Op::Le => a <= b,
                _ => a >= b,
            },
            _ => false,
        },
    }
}

impl<'a> Exec<'a> {
    pub(crate) fn new_with_context(
        scripts: &'a Scripts,
        globals: &'a mut HashMap<String, String>,
        limits: &'a Limits,
        rng: &'a mut u64,
        hashes: &'a mut super::hash_tables::Hashes,
        regex: &'a mut super::regex_funcs::RegexState,
        timers: &'a mut super::timers::TimerTable,
        now: Instant,
        context: ScriptContext,
    ) -> Self {
        Exec {
            scripts,
            globals,
            limits,
            rng,
            hashes,
            regex,
            context,
            halt_status: HaltStatus::None,
            frames: Vec::new(),
            stack: Vec::new(),
            statements: 0,
            start: Instant::now(),
            nesting: 0,
            echo_lines: 0,
            outbound: 0,
            unknown_idents: HashSet::new(),
            effects: Vec::new(),
            notices: Vec::new(),
            timers,
            now,
            owner: String::new(),
            frozen: None,
            live: None,
            stopped: false,
            answers: Vec::new(),
            answer_index: 0,
        }
    }

    pub(crate) fn finish(self) -> (Vec<Effect>, Vec<String>, HaltStatus) {
        (self.effects, self.notices, self.halt_status)
    }

    /// Runs an alias as the outermost call and turns a stop into one notice.
    pub(crate) fn run_top(&mut self, name: &str, body: Arc<Vec<Statement>>, args: Vec<String>) {
        let total: usize = args.iter().map(String::len).sum();
        let r = if total > self.limits.max_string_bytes {
            limit(LimitKind::StringBytes)
        } else {
            self.call_alias(name, body, args)
        };
        if let Err(stop) = r {
            self.stopped = true;
            let msg = match stop {
                Stop::Limit(k) => k.notice(self.limits),
                Stop::Error(m) => m,
            };
            self.notices.push(msg);
        }
    }

    /// Runs an event handler as the outermost call and turns a stop into one notice.
    pub(crate) fn run_top_event(
        &mut self,
        event_name: &str,
        body: Arc<Vec<Statement>>,
        args: Vec<String>,
    ) {
        let total: usize = args.iter().map(String::len).sum();
        let r = if total > self.limits.max_string_bytes {
            limit(LimitKind::StringBytes)
        } else {
            self.call_event_handler(event_name, body, args)
        };
        if let Err(stop) = r {
            self.stopped = true;
            let msg = match stop {
                Stop::Limit(k) => k.notice(self.limits),
                Stop::Error(m) => m,
            };
            self.notices.push(msg);
        }
    }

    fn call_event_handler(
        &mut self,
        name: &str,
        body: Arc<Vec<Statement>>,
        args: Vec<String>,
    ) -> R<Flow> {
        let label = format!("event:{name}");
        self.stack.push(label);
        self.frames.push(Frame {
            args,
            locals: HashMap::new(),
        });
        let r = self.exec_block(&body);
        self.frames.pop();
        self.stack.pop();
        match r? {
            Flow::Halt => Ok(Flow::Halt),
            Flow::Next | Flow::Return => Ok(Flow::Next),
            Flow::Break | Flow::Continue => fail("break or continue outside a loop; run stopped"),
            Flow::Goto(_) => fail("goto label not found; run stopped"),
        }
    }

    // ------------------------------------------------------------- limits

    fn tick(&mut self) -> R<()> {
        self.statements += 1;
        if self.statements > self.limits.max_statements {
            return limit(LimitKind::Statements);
        }
        if self.statements % self.limits.time_check_every.max(1) == 0
            && self.start.elapsed() >= self.limits.wall_time
        {
            return limit(LimitKind::WallTime);
        }
        Ok(())
    }

    fn push(&self, out: &mut String, s: &str) -> R<()> {
        if out.len() + s.len() > self.limits.max_string_bytes {
            return limit(LimitKind::StringBytes);
        }
        out.push_str(s);
        Ok(())
    }

    // -------------------------------------------------------------- calls

    fn find_alias(&self, name: &str) -> Option<Arc<Vec<Statement>>> {
        if !name.is_ascii() {
            return None;
        }
        self.scripts.values().find_map(|m| m.get(name)).cloned()
    }

    fn call_alias(&mut self, name: &str, body: Arc<Vec<Statement>>, args: Vec<String>) -> R<Flow> {
        // an alias that calls an alias already running re-enters it;
        // no other guard was found. The depth limit below ends such a cycle.
        if self.stack.len() >= self.limits.max_call_depth {
            return limit(LimitKind::CallDepth);
        }
        self.stack.push(name.to_string());
        self.frames.push(Frame {
            args,
            locals: HashMap::new(),
        });
        let r = self.exec_block(&body);
        self.frames.pop();
        self.stack.pop();
        match r? {
            Flow::Halt => Ok(Flow::Halt),
            Flow::Next | Flow::Return => Ok(Flow::Next),
            Flow::Break | Flow::Continue => fail("break or continue outside a loop; run stopped"),
            Flow::Goto(_) => fail("goto label not found; run stopped"),
        }
    }

    // ------------------------------------------------------------- blocks

    fn exec_block(&mut self, stmts: &[Statement]) -> R<Flow> {
        if self.nesting >= self.limits.max_nesting {
            return limit(LimitKind::Nesting);
        }
        self.nesting += 1;
        let r = self.exec_seq(stmts);
        self.nesting -= 1;
        r
    }

    fn exec_seq(&mut self, stmts: &[Statement]) -> R<Flow> {
        let mut i = 0;
        while i < stmts.len() {
            self.tick()?;
            let st = &stmts[i];
            let (flow, used) = if st.kind == StatementKind::If {
                // The `if` and the `elseif`/`else` statements that follow it.
                let mut end = i + 1;
                while end < stmts.len()
                    && matches!(stmts[end].kind, StatementKind::ElseIf | StatementKind::Else)
                {
                    let was_else = stmts[end].kind == StatementKind::Else;
                    end += 1;
                    if was_else {
                        break;
                    }
                }
                (self.exec_chain(&stmts[i..end])?, end - i)
            } else {
                (self.exec_stmt(st)?, 1)
            };
            match flow {
                Flow::Next => i += used,
                Flow::Goto(label) => match find_label(stmts, &label) {
                    Some(pos) => i = pos + 1,
                    None => return Ok(Flow::Goto(label)),
                },
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn exec_chain(&mut self, chain: &[Statement]) -> R<Flow> {
        for (k, st) in chain.iter().enumerate() {
            let take = match st.kind {
                StatementKind::Else => true,
                _ => {
                    if k > 0 {
                        self.tick()?;
                    }
                    self.eval_condition(&st.args)?
                }
            };
            if take {
                return self.exec_block(&st.body);
            }
        }
        Ok(Flow::Next)
    }

    fn exec_while(&mut self, st: &Statement) -> R<Flow> {
        loop {
            self.tick()?;
            if !self.eval_condition(&st.args)? {
                return Ok(Flow::Next);
            }
            match self.exec_block(&st.body)? {
                Flow::Next | Flow::Continue => {}
                Flow::Break => return Ok(Flow::Next),
                other => return Ok(other),
            }
        }
    }

    fn exec_stmt(&mut self, st: &Statement) -> R<Flow> {
        match st.kind {
            StatementKind::Label => Ok(Flow::Next),
            StatementKind::Block => self.exec_block(&st.body),
            StatementKind::Break => Ok(Flow::Break),
            StatementKind::Continue => Ok(Flow::Continue),
            StatementKind::Return => Ok(Flow::Return),
            StatementKind::Goto => {
                let label = st.args.trim().to_ascii_lowercase();
                if label.is_empty() {
                    return fail("goto needs a label; run stopped");
                }
                Ok(Flow::Goto(label))
            }
            StatementKind::While => self.exec_while(st),
            StatementKind::If | StatementKind::ElseIf | StatementKind::Else => {
                // `If` is handled with its chain; these are stray.
                fail("elseif or else without a matching if; run stopped")
            }
            StatementKind::Set
            | StatementKind::Var
            | StatementKind::Unset
            | StatementKind::Inc
            | StatementKind::Dec => self.exec_var_command(st).map(|()| Flow::Next),
            StatementKind::Eval => self.exec_assignment(st).map(|()| Flow::Next),
            StatementKind::Command => self.exec_command(st),
        }
    }

    // ----------------------------------------------------------- commands

    /// Adds one IRC effect. A field that could carry a second command or parameter
    /// stops the run; so does the line count past `max_outbound`.
    fn push_irc(&mut self, effect: IrcEffect) -> R<Flow> {
        if self.outbound >= self.limits.max_outbound {
            return limit(LimitKind::Outbound);
        }
        if let Some(line) = effect.line() {
            if line.len() > irc_effects::MAX_LINE_BYTES {
                return fail("IRC line is longer than 510 bytes; run stopped");
            }
        }
        self.outbound += 1;
        self.effects.push(Effect::Irc(effect));
        Ok(Flow::Next)
    }

    /// The parameters of an IRC command: the evaluated text, with CR, LF and NUL turned into
    /// spaces. An empty text stops the run.
    fn irc_args(&mut self, st: &Statement, name: &str) -> R<String> {
        let text = self.irc_text(st)?;
        if text.is_empty() {
            return fail(format!("/{name} needs parameters; run stopped"));
        }
        Ok(text)
    }

    /// Like `irc_args`, but an empty text is fine. The text is trimmed.
    fn irc_text(&mut self, st: &Statement) -> R<String> {
        let text = self.eval_text(&st.args, false)?;
        let text = clean_line(text).trim().to_string();
        // A word `#` names the channel of the run.
        let text = match self.context.chan.as_deref() {
            Some(chan) if text.split(' ').any(|w| w == "#") => {
                text.split(' ').map(|w| if w == "#" { chan } else { w }).collect::<Vec<_>>().join(" ")
            }
            _ => text,
        };
        Ok(text)
    }

    /// The target of the event or alias run, when it is a channel.
    fn context_channel(&self) -> Option<String> {
        self.context.target.clone().filter(|t| crate::text::is_channel_name(t))
    }

    fn exec_command(&mut self, st: &Statement) -> R<Flow> {
        // a user
        // alias is looked up before the built-in commands, and a `!` in front of the name skips
        // the lookup and runs the built-in command.
        let (bypass, name) = match st.name.strip_prefix('!') {
            Some(rest) if !rest.is_empty() => (true, rest),
            _ => (false, st.name.as_str()),
        };
        // a call to the alias that is running now takes the built-in branch.
        let running_now = self.stack.last().is_some_and(|n| n.eq_ignore_ascii_case(name));
        if !bypass && !running_now {
            if let Some(body) = self.find_alias(name) {
                let text = self.eval_text(&st.args, false)?;
                return self.call_alias(name, body, split_args(&text));
            }
        }
        match name {
            "echo" => {
                // Switches are read from the source, never from a value.
                let (target, text_raw) = parse_echo_switches(&st.args);
                let text = self.eval_text(text_raw, false)?;
                if self.echo_lines >= self.limits.max_echo_lines {
                    return limit(LimitKind::EchoLines);
                }
                self.echo_lines += 1;
                let clean = clean_line(text);
                if target == EchoTarget::Default {
                    self.effects.push(Effect::Echo(clean));
                } else {
                    self.effects.push(Effect::EchoTarget {
                        target,
                        text: clean,
                    });
                }
                Ok(Flow::Next)
            }
            "halt" => {
                self.halt_status = HaltStatus::Halt;
                Ok(Flow::Halt)
            }
            "haltdef" => {
                if self.halt_status != HaltStatus::Halt {
                    self.halt_status = HaltStatus::HaltDef;
                }
                Ok(Flow::Next)
            }
            "hmake" | "hfree" | "hadd" | "hdel" | "hinc" | "hdec" => {
                let text = self.eval_text(&st.args, false)?;
                super::hash_tables::command(&mut *self.hashes, name, &text)
                    .map_err(|m| Stop::Error(format!("{m}; run stopped")))?;
                Ok(Flow::Next)
            }
            "msg" | "privmsg" | "notice" | "describe" => {
                let text = self.irc_args(st, name)?;
                let (target, rest) = split_word(&text);
                if !irc_effects::targets_ok(target) {
                    return fail(format!("/{name} needs a valid target; run stopped"));
                }
                let body = rest.trim_start();
                if body.is_empty() {
                    return fail(format!("/{name} needs text; run stopped"));
                }
                let (target, body) = (target.to_string(), body.to_string());
                let effect = match name {
                    "msg" | "privmsg" => IrcEffect::Msg { target, text: body },
                    "notice" => IrcEffect::Notice { target, text: body },
                    _ => IrcEffect::Action { target, text: body },
                };
                self.push_irc(effect)
            }
            "me" => {
                // The target is the one of the event; a run without one has no window to use.
                // No window to act in: a silent return.
                let Some(target) = self.context.target.clone() else {
                    return Ok(Flow::Next);
                };
                let body = self.irc_args(st, name)?;
                self.push_irc(IrcEffect::Action { target, text: body })
            }
            "join" => {
                let text = self.irc_args(st, name)?;
                let (channel, key) = split_word(&text);
                if !irc_effects::channel_ok(channel) {
                    return fail("/join needs a channel name; run stopped");
                }
                let key = match key.trim() {
                    "" => None,
                    k if irc_effects::name_ok(k) => Some(k.to_string()),
                    _ => return fail("/join key must be one word; run stopped"),
                };
                let channel = channel.to_string();
                self.push_irc(IrcEffect::Join { channel, key })
            }
            "part" => {
                let text = self.irc_args(st, name)?;
                let (channel, reason) = split_word(&text);
                if !irc_effects::channel_ok(channel) {
                    return fail("/part needs a channel name; run stopped");
                }
                let reason = reason.trim();
                let (channel, reason) = (
                    channel.to_string(),
                    (!reason.is_empty()).then(|| reason.to_string()),
                );
                self.push_irc(IrcEffect::Part { channel, reason })
            }
            "nick" => {
                let text = self.irc_args(st, name)?;
                let new_nick = text.trim();
                if !irc_effects::nick_ok(new_nick) {
                    return fail("/nick needs one nick; run stopped");
                }
                self.push_irc(IrcEffect::Nick(new_nick.to_string()))
            }
            "mode" => {
                let text = self.irc_args(st, name)?;
                let (target, modes) = split_word(&text);
                if !irc_effects::name_ok(target) || modes.trim().is_empty() {
                    return fail("/mode needs a target and modes; run stopped");
                }
                let (target, modes) = (target.to_string(), balance_modes(modes.trim()));
                self.push_irc(IrcEffect::Mode { target, modes })
            }
            "kick" => {
                let text = self.irc_args(st, name)?;
                let (channel, rest) = split_word(&text);
                let (nick, reason) = split_word(rest);
                if !irc_effects::channel_ok(channel) || !irc_effects::nick_ok(nick) {
                    return fail("/kick needs a channel and a nick; run stopped");
                }
                let reason = reason.trim();
                let (channel, nick) = (channel.to_string(), nick.to_string());
                let reason = (!reason.is_empty()).then(|| reason.to_string());
                self.push_irc(IrcEffect::Kick { channel, nick, reason })
            }
            "whois" => {
                let text = self.irc_args(st, name)?;
                let nick = text.trim();
                if !irc_effects::nick_ok(nick) {
                    return fail("/whois needs one nick; run stopped");
                }
                self.push_irc(IrcEffect::Whois(nick.to_string()))
            }
            "query" => {
                let text = self.irc_args(st, name)?;
                let (nick, rest) = split_word(&text);
                if !irc_effects::query_ok(nick) {
                    return fail("/query needs one nick, not a channel; run stopped");
                }
                let body = rest.trim();
                let nick = nick.to_string();
                let text = (!body.is_empty()).then(|| body.to_string());
                self.push_irc(IrcEffect::Query { nick, text })
            }
            "raw" | "quote" => {
                let text = self.irc_args(st, name)?;
                let line = text.trim().to_string();
                if let Some(why) = irc_effects::raw_refusal(&line) {
                    return fail(format!("{why}; run stopped"));
                }
                self.push_irc(IrcEffect::Raw(line))
            }
            "say" => {
                let Some(target) = self.context.target.clone() else {
                    return fail("/say needs a target from an event; use /msg <target> <text>; run stopped");
                };
                let body = self.irc_args(st, name)?;
                self.push_irc(IrcEffect::Msg { target, text: body })
            }
            "omsg" | "onotice" => {
                let text = self.irc_args(st, name)?;
                let (first, rest) = split_word(&text);
                let (channel, body) = if irc_effects::channel_ok(first) {
                    (first.to_string(), rest.trim().to_string())
                } else {
                    match self.context_channel() {
                        Some(channel) => (channel, text.clone()),
                        None => return fail(format!("/{name} needs a channel; run stopped")),
                    }
                };
                if body.is_empty() {
                    return fail(format!("/{name} needs text; run stopped"));
                }
                // `@#chan` sends to the channel operators only.
                let target = format!("@{channel}");
                let effect = if name == "omsg" {
                    IrcEffect::Msg { target, text: body }
                } else {
                    IrcEffect::Notice { target, text: body }
                };
                self.push_irc(effect)
            }
            "hop" => {
                let text = self.irc_text(st)?;
                if text.starts_with('-') {
                    return fail("/hop switches are not supported by Ferric; run stopped");
                }
                let (first, rest) = split_word(&text);
                let (channel, reason) = if irc_effects::channel_ok(first) {
                    (first.to_string(), rest.to_string())
                } else {
                    match self.context_channel() {
                        Some(channel) => (channel, text.clone()),
                        None => return fail("/hop needs a channel; run stopped"),
                    }
                };
                let reason = (!reason.is_empty()).then_some(reason);
                self.push_irc(IrcEffect::Part { channel: channel.clone(), reason })?;
                self.push_irc(IrcEffect::Join { channel, key: None })
            }
            "ban" => {
                let text = self.irc_args(st, name)?;
                if text.starts_with('-') {
                    return fail("/ban switches are not supported by Ferric; run stopped");
                }
                let (first, rest) = split_word(&text);
                let (channel, mask_text) = if irc_effects::channel_ok(first) {
                    (first.to_string(), rest)
                } else {
                    match self.context_channel() {
                        Some(channel) => (channel, text.as_str()),
                        None => return fail("/ban needs a channel; run stopped"),
                    }
                };
                let (mask, extra) = split_word(mask_text);
                if !extra.is_empty() {
                    return fail("/ban kick messages are not supported by Ferric; run stopped");
                }
                // A nick would need a USERHOST lookup first, which scripts do not get here.
                let is_address = mask.contains('!') || mask.contains('@') || mask.contains('*');
                if !is_address || !irc_effects::name_ok(mask) {
                    return fail("/ban needs an address such as *!*@host; run stopped");
                }
                let modes = format!("+b {mask}");
                self.push_irc(IrcEffect::Mode { target: channel, modes })
            }
            "partall" => {
                let text = self.irc_text(st)?;
                let text = (!text.is_empty()).then_some(text);
                self.push_irc(IrcEffect::Broadcast {
                    scope: BroadcastScope::Channels,
                    kind: BroadcastKind::Part,
                    text,
                })
            }
            "amsg" | "ame" => {
                let text = self.irc_args(st, name)?;
                let kind = if name == "amsg" { BroadcastKind::Msg } else { BroadcastKind::Action };
                self.push_irc(IrcEffect::Broadcast {
                    scope: BroadcastScope::ChannelsEveryNetwork,
                    kind,
                    text: Some(text),
                })
            }
            "qmsg" | "qme" => {
                let text = self.irc_args(st, name)?;
                let kind = if name == "qmsg" { BroadcastKind::Msg } else { BroadcastKind::Action };
                self.push_irc(IrcEffect::Broadcast {
                    scope: BroadcastScope::Queries,
                    kind,
                    text: Some(text),
                })
            }
            "away" => {
                let text = self.irc_text(st)?;
                let trailing = (!text.is_empty()).then_some(text);
                self.push_irc(IrcEffect::Line { verb: "AWAY", params: Vec::new(), trailing })
            }
            "quit" => {
                let text = self.irc_text(st)?;
                let trailing = (!text.is_empty()).then_some(text);
                self.push_irc(IrcEffect::Line { verb: "QUIT", params: Vec::new(), trailing })
            }
            "invite" => {
                let text = self.irc_args(st, name)?;
                let (nick, rest) = split_word(&text);
                let (channel, extra) = split_word(rest);
                if !irc_effects::nick_ok(nick) || !irc_effects::channel_ok(channel) || !extra.is_empty() {
                    return fail("/invite needs a nick and a channel; run stopped");
                }
                let params = vec![nick.to_string(), channel.to_string()];
                self.push_irc(IrcEffect::Line { verb: "INVITE", params, trailing: None })
            }
            "topic" => {
                let text = self.irc_args(st, name)?;
                let (channel, rest) = split_word(&text);
                if !irc_effects::channel_ok(channel) {
                    return fail("/topic needs a channel; run stopped");
                }
                let body = rest.trim();
                let trailing = (!body.is_empty()).then(|| body.to_string());
                let params = vec![channel.to_string()];
                self.push_irc(IrcEffect::Line { verb: "TOPIC", params, trailing })
            }
            "list" => {
                let text = self.irc_text(st)?;
                if text.starts_with('-') {
                    return fail("/list -MIN and -MAX are not supported by Ferric; run stopped");
                }
                let params = if text.is_empty() {
                    Vec::new()
                } else if irc_effects::channel_ok(&text) {
                    vec![text]
                } else {
                    return fail("/list takes one channel; run stopped");
                };
                self.push_irc(IrcEffect::Line { verb: "LIST", params, trailing: None })
            }
            "links" => {
                if !self.irc_text(st)?.is_empty() {
                    return fail("/links switches are not supported by Ferric; run stopped");
                }
                self.push_irc(IrcEffect::Line { verb: "LINKS", params: Vec::new(), trailing: None })
            }
            "userhost" => {
                let text = self.irc_args(st, name)?;
                if !irc_effects::nick_ok(&text) {
                    return fail("/userhost needs one nick; run stopped");
                }
                let params = vec![text];
                self.push_irc(IrcEffect::Line { verb: "USERHOST", params, trailing: None })
            }
            "mnick" | "tnick" => {
                let text = self.irc_args(st, name)?;
                if !irc_effects::nick_ok(&text) {
                    return fail(format!("/{name} needs one nick; run stopped"));
                }
                self.push_irc(IrcEffect::Nick(text))
            }
            "uwho" => {
                let text = self.irc_args(st, name)?;
                if !irc_effects::nick_ok(&text) {
                    return fail("/uwho needs one nick; run stopped");
                }
                self.push_irc(IrcEffect::Whois(text))
            }
            "ctcpreply" => {
                let text = self.irc_args(st, name)?;
                let (nick, rest) = split_word(&text);
                let (ctcp, message) = split_word(rest);
                let ctcp_ok = !ctcp.is_empty()
                    && ctcp.len() <= 16
                    && ctcp.bytes().all(|b| b.is_ascii_alphanumeric());
                if !irc_effects::nick_ok(nick) || !ctcp_ok {
                    return fail("/ctcpreply needs a nick and a CTCP name; run stopped");
                }
                // A \u{1} in the message would end the CTCP frame early.
                let message = message.replace('\u{1}', " ");
                let message = message.trim();
                let framed = if message.is_empty() {
                    format!("\u{1}{ctcp}\u{1}")
                } else {
                    format!("\u{1}{ctcp} {message}\u{1}")
                };
                self.push_irc(IrcEffect::Notice { target: nick.to_string(), text: framed })
            }
            "ajinvite" | "anick" | "queryrn" | "pvoice" => fail(format!(
                "/{name} is not supported by Ferric; run stopped"
            )),
            "timers" => self.timers_command(st),
            "timer" => self.timer_command(st, None),
            "timershow" => fail("/timershow is not supported by Ferric; run stopped"),
            n if n.len() > 5 && n.starts_with("timer") && n[5..].bytes().all(|b| b.is_ascii_digit()) => {
                let id = n[5..].to_string();
                self.timer_command(st, Some(id))
            }
            "run" | "dll" => fail(format!(
                "command '{name}' is not supported by Ferric; run stopped"
            )),
            _ => fail(format!(
                "unknown or unsupported command '{}'; run stopped",
                safe_name(name)
            )),
        }
    }

    // ------------------------------------------------------------- timers

    /// `/timer[N] [-om] [name] <reps> <interval> <command>`, or `/timer[N] off`. `id` is the
    /// number in the command name (`/timer3`), when there is one. The command is kept as
    /// written and read when the timer runs; reps and interval are read now.
    fn timer_command(&mut self, st: &Statement, id: Option<String>) -> R<Flow> {
        let mut rest = st.args.trim_start();
        let mut online = self.context.network.is_some();
        let mut milliseconds = false;
        let mut immediate = false;
        loop {
            let (w, after) = split_word(rest);
            if w.len() < 2 || !w.starts_with('-') || !w[1..].chars().all(|c| c.is_ascii_alphabetic()) {
                break;
            }
            for c in w[1..].chars() {
                match c {
                    'o' => online = false,
                    // -m and -h: the interval is in milliseconds.
                    'm' | 'h' => milliseconds = true,
                    // -e: an existing timer of this name is due now.
                    'e' => immediate = true,
                    other => return fail(format!("/timer -{other} is not supported by Ferric; run stopped")),
                }
            }
            rest = after;
        }

        let (first, after_first) = split_word(rest);
        let (name, rest) = match id {
            Some(id) => (id, rest),
            None => {
                let value = self.eval_text(first, false)?;
                if value.is_empty() {
                    return fail("/timer needs a repetition count or a name; run stopped");
                }
                if value.parse::<u64>().is_ok() {
                    // No name: the lowest free number, and the repetition count is the first word.
                    match self.timers.first_free_id() {
                        Some(free) => (free.to_string(), rest),
                        None => return fail("no free timer number; run stopped"),
                    }
                } else {
                    (value, after_first)
                }
            }
        };

        let (word, after_word) = split_word(rest);
        let word_value = self.eval_text(word, false)?;
        if word_value.eq_ignore_ascii_case("off") {
            if name.contains(['?', '*']) {
                self.timers.remove_matching(&name);
            } else {
                self.timers.take(&name);
            }
            return Ok(Flow::Next);
        }

        let reps: u64 = word_value
            .parse()
            .map_err(|_| Stop::Error("/timer needs a whole number of repetitions; run stopped".into()))?;
        let (interval_word, body_raw) = split_word(after_word);
        let interval_text = self.eval_text(interval_word, false)?;
        if interval_text.contains(':') {
            return fail("/timer with a time of day is not supported by Ferric; run stopped");
        }
        let raw: i64 = interval_text
            .parse()
            .map_err(|_| Stop::Error("/timer needs a whole number interval; run stopped".into()))?;
        let command = body_raw.trim();
        if command.is_empty() {
            return fail("/timer needs a command; run stopped");
        }
        let body = super::runtime::parse_timer_body(command).map_err(|m| Stop::Error(format!("{m}; run stopped")))?;

        // An interval below 1 is 1 millisecond, whatever the unit option.
        // The first run waits one period; Ferric raises every period to MIN_PERIOD.
        let (interval, milliseconds) = if raw < 1 { (1, true) } else { (raw as u64, milliseconds) };
        let period_ms = if milliseconds { interval } else { interval.saturating_mul(1000) };
        let period = Duration::from_millis(period_ms).max(MIN_PERIOD);
        let next_due = self.now + period;
        let timer = Timer {
            name: name.to_ascii_lowercase(),
            owner: self.owner.clone(),
            network: self.context.network.clone(),
            online,
            reps_left: (reps > 0).then_some(reps),
            period,
            next_due,
            body: Arc::new(body),
            command: command.to_string(),
            context: self.context.clone(),
            clock: time_funcs::Clock::now(),
            seq: 0,
        };
        let mut timer = timer;
        if immediate && self.timers.contains(&name) {
            timer.next_due = self.now;
        }
        self.timers.insert(timer).map_err(|m| Stop::Error(format!("{m}; run stopped")))?;
        Ok(Flow::Next)
    }

    /// `/timers` lists the timers as echo lines; `/timers off` switches them all off.
    fn timers_command(&mut self, st: &Statement) -> R<Flow> {
        let text = self.eval_text(&st.args, false)?;
        match text.trim() {
            "" => {}
            t if t.eq_ignore_ascii_case("off") => {
                self.timers.clear();
                return Ok(Flow::Next);
            }
            _ => return fail("/timers takes off or nothing; run stopped"),
        }
        let lines: Vec<String> = self
            .timers
            .iter()
            .map(|t| {
                let reps = t.reps_left.map_or("until off".to_string(), |n| format!("{n} left"));
                format!("timer {}: {} ms, {reps}, {}", t.name, t.period.as_millis(), t.command)
            })
            .collect();
        for line in lines {
            self.echo_line(line)?;
        }
        Ok(Flow::Next)
    }

    /// One echo line of a command, counted against the echo limit.
    fn echo_line(&mut self, text: String) -> R<()> {
        if self.echo_lines >= self.limits.max_echo_lines {
            return limit(LimitKind::EchoLines);
        }
        self.echo_lines += 1;
        self.effects.push(Effect::Echo(clean_line(text)));
        Ok(())
    }

    /// The clock the time identifiers read: the one of the timer body, or now.
    fn clock_now(&self) -> time_funcs::Clock {
        self.frozen.unwrap_or_else(time_funcs::Clock::now)
    }

    /// `$!name`: read from the live context of a timer run; the same as `$name` when there is none.
    fn live_value(&mut self, name: &str) -> R<String> {
        let Some(live) = self.live.clone() else {
            return self.eval_part_inner(&Part::Ident(name.to_string()));
        };
        let saved_context = std::mem::replace(&mut self.context, live);
        let saved_clock = self.frozen.take();
        let value = self.eval_part_inner(&Part::Ident(name.to_string()));
        self.context = saved_context;
        self.frozen = saved_clock;
        value
    }

    /// The answer to the next prompt of the run; a prompt without an answer stops the run.
    fn next_answer(&mut self) -> R<String> {
        match self.answers.get(self.answer_index) {
            Some(answer) => {
                self.answer_index += 1;
                Ok(answer.clone())
            }
            None => fail("a prompt ($?) has no answer; run stopped"),
        }
    }

    /// Popup-menu text (a label, or a command): evaluated with `args` as `$1`, `$2`, ...
    pub(crate) fn menu_text(&mut self, raw: &str, args: &[String]) -> Result<String, String> {
        self.frames.push(Frame {
            args: args.to_vec(),
            locals: HashMap::new(),
        });
        let result = self.eval_text(raw, false);
        self.frames.pop();
        result.map_err(|stop| match stop {
            Stop::Error(message) => message,
            Stop::Limit(kind) => kind.notice(self.limits),
        })
    }

    fn exec_assignment(&mut self, st: &Statement) -> R<()> {
        // `%x = expr`. The name is the statement token; nothing is printed.
        let value_raw = match st.args.strip_prefix('=') {
            Some(v) if st.name.starts_with('%') && !v.starts_with('=') => v.trim_start(),
            _ => {
                return fail(
                    "unsupported statement (assign with '%x = value'); run stopped",
                )
            }
        };
        let name = var_word(&st.name)?;
        let value = self.eval_value_raw(value_raw)?;
        let scope = self.write_scope(&name, None);
        self.set_var(scope, &name, value)
    }

    fn exec_var_command(&mut self, st: &Statement) -> R<()> {
        let (forced, rest) = take_flags(&st.args)?;
        match st.kind {
            StatementKind::Set => {
                let (w, value_raw) = split_word(rest);
                let name = var_word(w)?;
                let text = self.eval_text(value_raw, false)?;
                // Arithmetic only when the source itself has an operator word, such as `1 + 1`. A date
                // such as 2026-11-21, or text from a chat line through `$1-`, stays as written.
                let value = if is_arithmetic_source(value_raw) { set_value(text) } else { text };
                let scope = self.write_scope(&name, forced);
                self.set_var(scope, &name, value)
            }
            StatementKind::Var => {
                let (w, after) = split_word(rest);
                let name = var_word(w)?;
                let value_raw = if after.is_empty() {
                    ""
                } else if let Some(v) = after.strip_prefix('=') {
                    v.trim_start()
                } else {
                    return fail("var needs '=' between name and value; run stopped");
                };
                let value = self.eval_value_raw(value_raw)?;
                self.set_var(forced.unwrap_or(Scope::Local), &name, value)
            }
            StatementKind::Unset => {
                let mut any = false;
                for w in rest.split_whitespace() {
                    let name = var_word(w)?;
                    let scope = self.write_scope(&name, forced);
                    self.unset_var(scope, &name);
                    any = true;
                }
                if any {
                    Ok(())
                } else {
                    fail("unset needs a variable name; run stopped")
                }
            }
            _ => {
                // inc / dec
                let (w, amount_raw) = split_word(rest);
                let name = var_word(w)?;
                let current_text = self.get_var(&name).cloned().unwrap_or_default();
                let amount_text = if amount_raw.is_empty() {
                    "1".to_string()
                } else {
                    self.eval_value_raw(amount_raw)?
                };
                // /inc and /dec take atof values. A missing variable is 0,
                // a missing or invalid step is 1, and the result has six decimals with zeros removed.
                let current = current_text.trim().parse::<f64>().unwrap_or(0.0);
                let amount = amount_text.trim().parse::<f64>().unwrap_or(1.0);
                let result = if st.kind == StatementKind::Inc {
                    current + amount
                } else {
                    current - amount
                };
                if !result.is_finite() {
                    return fail("inc/dec result is outside the number range; run stopped");
                }
                // -0 is written as 0, as in $calc.
                let value = if result == 0.0 {
                    "0".to_string()
                } else {
                    format!("{result:.6}").trim_end_matches('0').trim_end_matches('.').to_string()
                };
                let scope = self.write_scope(&name, forced);
                self.set_var(scope, &name, value)
            }
        }
    }

    // ---------------------------------------------------------- variables

    fn get_var(&self, name: &str) -> Option<&String> {
        self.frames
            .last()
            .and_then(|f| f.locals.get(name))
            .or_else(|| self.globals.get(name))
    }

    /// Local precedence unless `-g` (plan M1): an existing local is written,
    /// otherwise the global.
    fn write_scope(&self, name: &str, forced: Option<Scope>) -> Scope {
        forced.unwrap_or_else(|| {
            let local = self
                .frames
                .last()
                .is_some_and(|f| f.locals.contains_key(name));
            if local {
                Scope::Local
            } else {
                Scope::Global
            }
        })
    }

    fn set_var(&mut self, scope: Scope, name: &str, value: String) -> R<()> {
        if value.len() > self.limits.max_string_bytes {
            return limit(LimitKind::StringBytes);
        }
        let exists = match scope {
            Scope::Local => self
                .frames
                .last()
                .is_some_and(|f| f.locals.contains_key(name)),
            Scope::Global => self.globals.contains_key(name),
        };
        if !exists {
            let total =
                self.globals.len() + self.frames.iter().map(|f| f.locals.len()).sum::<usize>();
            if total >= self.limits.max_variables {
                return limit(LimitKind::Variables);
            }
        }
        match scope {
            Scope::Local => {
                if let Some(f) = self.frames.last_mut() {
                    f.locals.insert(name.to_string(), value);
                }
            }
            Scope::Global => {
                self.globals.insert(name.to_string(), value);
            }
        }
        Ok(())
    }

    fn unset_var(&mut self, scope: Scope, name: &str) {
        match scope {
            Scope::Local => {
                if let Some(f) = self.frames.last_mut() {
                    f.locals.remove(name);
                }
            }
            Scope::Global => {
                self.globals.remove(name);
            }
        }
    }

    // --------------------------------------------------------- evaluation

    fn parse_parts(&self, raw: &str, brackets: bool) -> R<Vec<Part>> {
        if raw.len() > self.limits.max_string_bytes {
            return limit(LimitKind::StringBytes);
        }
        expr::parse(raw, brackets)
            .map_err(|_| Stop::Error("nested [ ] is not supported; run stopped".to_string()))
    }

    /// Plain text: substitutes `%var`, `$N` and identifiers, no arithmetic.
    fn eval_text(&mut self, raw: &str, brackets: bool) -> R<String> {
        let parts = self.parse_parts(raw, brackets)?;
        self.eval_parts(&parts)
    }

    /// Value of `%x = raw`, `var` and `inc`: arithmetic when the source is an
    /// arithmetic expression, else text.
    fn eval_value_raw(&mut self, raw: &str) -> R<String> {
        let parts = self.parse_parts(raw, true)?;
        self.eval_value(&parts)
    }

    fn eval_value(&mut self, parts: &[Part]) -> R<String> {
        let Some(shape) = expr::shape(parts) else {
            return self.eval_parts(parts);
        };
        let mut vals = Vec::with_capacity(shape.atoms.len());
        for &idx in &shape.atoms {
            let text = self.eval_part(&parts[idx])?;
            vals.push(to_int(&text));
        }
        expr::compute(&shape, &vals)
            .map(|v| v.to_string())
            .map_err(arith_error)
    }

    /// `$iif(C,T,F)`. The condition picks one branch; `F` may be left out, and
    /// then a false condition gives nothing. Leading spaces of a branch are
    /// dropped.
    fn eval_iif(&mut self, raw: &str) -> R<String> {
        let parts = text_funcs::split_call_args(raw);
        if parts.len() < 2 || parts.len() > 3 {
            return fail("$iif takes a condition, a true text and an optional false text; run stopped");
        }
        let branch = if self.eval_condition(&parts[0])? {
            Some(&parts[1])
        } else {
            parts.get(2)
        };
        let Some(text) = branch else {
            return Ok(String::new());
        };
        let value = self.eval_text(text.trim_start(), false)?;
        let mut out = String::new();
        self.push(&mut out, &value)?;
        Ok(out)
    }

    /// Evaluates one argument text of `$regsub` or `$regsubex`.
    fn eval_raw(&mut self, raw: &str) -> R<String> {
        let parts = self.parse_parts(raw, false)?;
        self.eval_parts(&parts)
    }

    /// Evaluates a parsed subtext for one `$regsubex` match. The markers are
    /// replaced in the plain text only; identifier and variable values are used as they are.
    fn eval_marked(&mut self, parts: &[Part], k: usize, groups: &super::regex_funcs::Groups) -> R<String> {
        let mut out = String::new();
        for part in parts {
            let value = match part {
                Part::Lit(s) => {
                    // Markers expand into the text: the expansion may only fill what the string limit still allows.
                    let room = self.limits.max_string_bytes.saturating_sub(out.len());
                    match super::regex_funcs::replace_markers(s, k, groups, room) {
                        Some(value) => value,
                        None => return limit(LimitKind::StringBytes),
                    }
                }
                other => self.eval_part(other)?,
            };
            self.push(&mut out, &value)?;
        }
        Ok(out)
    }

    /// `$regsub` and `$regsubex`: `[name], text, pattern, subtext, %variable`. The
    /// result goes to the variable and the number of substitutions is returned.
    /// `$regsub` takes its subtext as plain text; `$regsubex` evaluates it per match.
    fn eval_regsub(&mut self, name: &str, raw: &[String]) -> R<String> {
        let named = match (name, raw.len()) {
            (_, 5) => true,
            ("regsub", 4) => false,
            ("regsub", _) => {
                return fail("$regsub takes [name], text, pattern, subtext and %variable; run stopped")
            }
            _ => {
                return fail("$regsubex takes name, text, pattern, subtext and %variable; run stopped")
            }
        };
        let target = match raw[raw.len() - 1].trim().strip_prefix('%') {
            Some(var) if !var.is_empty() => var_word(raw[raw.len() - 1].trim())?,
            _ => {
                return fail(format!(
                    "${name} writes to a %variable; a &binvar is not supported; run stopped"
                ))
            }
        };
        // An existing global is written as a global, so a local of the same name cannot shadow it.
        // A new name stays local, as it always was here.
        let scope = match self.write_scope(&target, None) {
            Scope::Global if self.get_var(&target).is_none() => Scope::Local,
            other => other,
        };
        let base = usize::from(named);
        let rname = if named {
            self.eval_raw(&raw[0])?
        } else {
            super::regex_funcs::DEFAULT_NAME.to_string()
        };
        let text = self.eval_raw(&raw[base])?;
        let pattern = self.eval_raw(&raw[base + 1])?;
        let subtext = raw[base + 2].clone();
        let found = match super::regex_funcs::find(&text, &pattern) {
            Ok(found) => found,
            Err(message) => {
                self.regex.last_error = message;
                super::regex_funcs::record_empty(&mut *self.regex, &rname);
                self.set_var(scope, &target, text)?;
                return Ok("0".to_string());
            }
        };
        if name == "regsub" {
            let Some((count, out)) =
                super::regex_funcs::plain_replace(&found, &subtext, self.limits.max_string_bytes)
            else {
                return limit(LimitKind::StringBytes);
            };
            super::regex_funcs::record(&mut *self.regex, &rname, &found);
            self.set_var(scope, &target, out)?;
            return Ok(count.to_string());
        }
        let mut out = String::new();
        let mut at = 0;
        let mut count = 0usize;
        // The subtext is parsed once, and only when there is a match to fill in.
        let parts = if found.matches.is_empty() { Vec::new() } else { self.parse_parts(&subtext, false)? };
        for (k, groups) in found.matches.iter().enumerate() {
            self.tick()?;
            let Some((start, whole)) = groups[0].clone() else { continue };
            self.push(&mut out, &found.text[at..start])?;
            let value = self.eval_marked(&parts, k + 1, groups)?;
            self.push(&mut out, &value)?;
            at = start + whole.len();
            count += 1;
        }
        self.push(&mut out, &found.text[at..])?;
        super::regex_funcs::record(&mut *self.regex, &rname, &found);
        self.set_var(scope, &target, out)?;
        Ok(count.to_string())
    }

    fn eval_parts(&mut self, parts: &[Part]) -> R<String> {
        let mut out = String::new();
        for p in parts {
            match p {
                Part::Lit(s) => self.push(&mut out, s)?,
                other => {
                    let v = self.eval_part(other)?;
                    self.push(&mut out, &v)?;
                }
            }
        }
        Ok(out)
    }

    fn args(&self) -> &[String] {
        let none: &[String] = &[];
        self.frames.last().map_or(none, |f| f.args.as_slice())
    }

    /// Runs `f` one expression level deeper. Nested calls such as `$len($len(...))` share the block
    /// nesting budget, so a deep expression stops with a notice instead of overflowing the stack.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        // Each call also takes a step of the statement budget and the wall-time check.
        self.tick()?;
        if self.nesting >= self.limits.max_nesting {
            return limit(LimitKind::Nesting);
        }
        self.nesting += 1;
        let r = f(self);
        self.nesting -= 1;
        r
    }

    fn eval_part(&mut self, part: &Part) -> R<String> {
        match part {
            Part::Call { .. } => self.nested(|s| s.eval_part_inner(part)),
            _ => self.eval_part_inner(part),
        }
    }

    fn eval_part_inner(&mut self, part: &Part) -> R<String> {
        Ok(match part {
            Part::Lit(s) => s.clone(),
            // an unset variable is empty in text.
            Part::Var(n) => self.get_var(n).cloned().unwrap_or_default(),
            Part::Param(0) => self.args().len().to_string(),
            Part::Param(n) => self.args().get(n - 1).cloned().unwrap_or_default(),
            Part::Range { from, to } => {
                // inclusive end, slots joined by one space.
                let a = self.args();
                let end = to.map_or(a.len(), |t| t.min(a.len()));
                if *from == 0 || *from > end {
                    String::new()
                } else {
                    a[from - 1..end].join(" ")
                }
            }
            Part::Prompt(_) => self.next_answer()?,
            Part::Ident(name) if name.starts_with('!') => self.live_value(&name[1..])?,
            Part::Ident(name) => {
                match name.as_str() {
                    "null" => String::new(),
                    // $false is false: 0 in a condition.
                    "false" => "0".to_string(),
                    "true" => "1".to_string(),
                    "nick" => self.context.nick.clone().unwrap_or_default(),
                    "chan" => self.context.chan.clone().unwrap_or_default(),
                    "snicks" => {
                        let value = self.context.snicks.clone().unwrap_or_default();
                        if value.encode_utf16().count() > 0x2864 {
                            return fail("$snicks is too long; run stopped");
                        }
                        value
                    }
                    "knick" | "opnick" | "vnick" | "hnick" => self.context.knick.clone().unwrap_or_default(),
                    "banmask" => self.context.banmask.clone().unwrap_or_default(),
                    "bnick" => self.context.banmask.as_deref().and_then(ban_nick).unwrap_or_default().to_string(),
                    "newnick" => self.context.newnick.clone().unwrap_or_default(),
                    "me" => self.context.me.clone().unwrap_or_default(),
                    "regerrstr" => self.regex.last_error.clone(),
                    "pi" => super::math_funcs::PI.to_string(),
                    "cr" => "\r".to_string(),
                    "lf" => "\n".to_string(),
                    "crlf" => "\r\n".to_string(),
                    "network" => self.context.network.clone().unwrap_or_default(),
                    // Host data of the triggering event. Without an address these are $null.
                    "address" => self.context.address.clone().unwrap_or_default(),
                    "fulladdress" => match (&self.context.nick, &self.context.address) {
                        (Some(nick), Some(address)) => format!("{nick}!{address}"),
                        _ => String::new(),
                    },
                    "site" => host_of(self.context.address.as_deref()).to_string(),
                    "wildsite" => match host_of(self.context.address.as_deref()) {
                        "" => String::new(),
                        host => format!("*!*@{host}"),
                    },
                    "target" => self.context.target.clone().unwrap_or_default(),
                    // Bare time identifiers, such as `$time` or `$fulldate`.
                    other if time_funcs::is_time(other) => {
                        time_funcs::call(other, &[], &self.clock_now()).map_err(call_stop)?
                    }
                    _ => {
                        if self.unknown_idents.insert(name.clone()) {
                            self.notices.push(format!(
                                "unsupported identifier ${} evaluates to empty",
                                safe_name(name)
                            ));
                        }
                        String::new()
                    }
                }
            }
            Part::Calc(raw) => {
                // shortcut: nested identifier calls are unsupported until their evaluation order is known.
                let parts = self.parse_parts(raw, false)?;
                let mut input = String::new();
                for part in &parts {
                    match part {
                        Part::Lit(text) => self.push(&mut input, text)?,
                        Part::Calc(_) | Part::Ident(_) => return fail("nested identifiers in $calc are not supported; run stopped"),
                        _ => {
                            let value = self.eval_part(part)?;
                            let number = if value.is_empty() { 0.0 } else {
                                value.trim().parse::<f64>().map_err(|_| Stop::Error("non-numeric $calc operand; run stopped".into()))?
                            };
                            if !number.is_finite() { return Err(arith_error(ArithError::Overflow)); }
                            self.push(&mut input, &format!("({number:.17})"))?;
                        }
                    }
                }
                let value = super::float_calc::compute(&input)
                    .ok_or_else(|| Stop::Error("unsupported or invalid $calc expression; run stopped".into()))?;
                // Signed zero is written as 0.
                if value == 0.0 {
                    "0".to_string()
                } else {
                    format!("{value:.6}").trim_end_matches('0').trim_end_matches('.').to_string()
                }
            }
            // `$iif` evaluates only the branch its condition picks.
            Part::Call { name, args, .. } if name == "iif" => self.eval_iif(args)?,
            Part::Call { name, args, prop } => {
                let raw_args = text_funcs::split_call_args(args);
                // The regsub calls write a variable, and regsubex evaluates its subtext per match.
                if name == "regsub" || name == "regsubex" {
                    return self.eval_regsub(name, &raw_args);
                }
                let mut values = Vec::with_capacity(raw_args.len());
                for raw in &raw_args {
                    let parts = self.parse_parts(raw, false)?;
                    values.push(self.eval_parts(&parts)?);
                }
                // Lookups that take properties: hash tables and regular expression results.
                let looked_up = match name.as_str() {
                    "hget" | "hfind" => Some(super::hash_tables::ident(&*self.hashes, name, &values, prop.as_deref())),
                    "regml" | "regmlex" => Some(super::regex_funcs::ident(&*self.regex, name, &values, prop.as_deref())),
                    "regex" => Some(super::regex_funcs::regex_call(&mut *self.regex, &values)),
                    "timer" => Some(super::timers::ident(&*self.timers, self.now, &values, prop.as_deref())),
                    _ => None,
                };
                if let Some(result) = looked_up {
                    let value = result.map_err(call_stop)?;
                    let mut out = String::new();
                    self.push(&mut out, &value)?;
                    return Ok(out);
                }
                if prop.is_some() {
                    return fail(format!("properties of ${name} are not supported; run stopped"));
                }
                let result = if name == "rand" {
                    text_funcs::rand(&values, &mut *self.rng)
                } else if time_funcs::is_time(name) {
                    time_funcs::call(name, &values, &self.clock_now())
                } else {
                    text_funcs::call(name, &values, self.limits.max_string_bytes)
                };
                match result {
                    Ok(value) => {
                        let mut out = String::new();
                        self.push(&mut out, &value)?;
                        out
                    }
                    Err(CallError::Arguments(msg)) => {
                        return fail(format!("{msg}; run stopped"));
                    }
                    Err(CallError::TooLong) => return limit(LimitKind::StringBytes),
                }
            }
            Part::Bracket(inner) => self.eval_value(inner)?,
        })
    }

    // --------------------------------------------------------- conditions

    fn eval_condition(&mut self, raw: &str) -> R<bool> {
        if raw.len() > self.limits.max_string_bytes {
            return limit(LimitKind::StringBytes);
        }
        let c = cond::parse(raw).map_err(|e| {
            Stop::Error(match e {
                CondError::Unsupported(op) => format!(
                    "condition operator '{}' is not supported; run stopped",
                    safe_name(&op)
                ),
                CondError::TooDeep => "condition nested too deeply; run stopped".to_string(),
                CondError::Empty | CondError::Unbalanced | CondError::Trailing => {
                    "malformed condition; run stopped".to_string()
                }
            })
        })?;
        self.eval_cond(&c)
    }

    fn eval_cond(&mut self, c: &Cond) -> R<bool> {
        match c {
            Cond::Or(v) => {
                for x in v {
                    if self.eval_cond(x)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Cond::And(v) => {
                for x in v {
                    if !self.eval_cond(x)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Cond::Not(x) => Ok(!self.eval_cond(x)?),
            Cond::Cmp { left, op, right } => {
                let l = self.eval_text(left, false)?;
                let r = self.eval_text(right, false)?;
                Ok(compare(*op, l.trim(), r.trim()))
            }
            // Ferric choice: a lone operand is true when it is a non-zero integer.
            Cond::Truth(t) => Ok(int_strict(&self.eval_text(t, false)?).is_some_and(|n| n != 0)),
        }
    }
}
