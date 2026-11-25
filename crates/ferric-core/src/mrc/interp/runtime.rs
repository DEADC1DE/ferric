//! The script runtime: alias registry, global variables and the public API.
//!
//! Pure library code. No file, network or process access: a trigger returns
//! its output as data ([`Effect`] values and notice text) and the caller
//! decides what to do with it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::eval::Exec;
use super::irc_effects::IrcEffect;
use super::limits::Limits;
use super::value::safe_name;
use crate::mrc::{parse_str, Event, Item, Statement, UnclassifiedKind};

/// Text for the user about the interpreter itself (never script data).
pub type Notice = String;

/// Target for an echo statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EchoTarget {
    #[default]
    Default,
    Active,
    Status,
}

/// Something a script asked for. The caller applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A line for the local window. Never contains CR, LF or NUL.
    Echo(String),
    /// A line targeted to a specific window (-a or -s).
    EchoTarget {
        target: EchoTarget,
        text: String,
    },
    /// A line to send to IRC through the connection.
    Irc(IrcEffect),
}

impl Effect {
    /// The text of an echo line; empty for an IRC effect.
    pub fn text(&self) -> &str {
        match self {
            Effect::Echo(s) => s.as_str(),
            Effect::EchoTarget { text, .. } => text.as_str(),
            Effect::Irc(_) => "",
        }
    }

    pub fn target(&self) -> EchoTarget {
        match self {
            Effect::Echo(_) | Effect::Irc(_) => EchoTarget::Default,
            Effect::EchoTarget { target, .. } => *target,
        }
    }

    /// True for an echo line; false for an IRC effect.
    pub fn is_echo(&self) -> bool {
        !matches!(self, Effect::Irc(_))
    }
}

/// Status code returned after running a script handler.
/// The rules distinguish halt (status 2) from haltdef (status 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HaltStatus {
    #[default]
    None = 0,
    HaltDef = 1,
    Halt = 2,
}

impl HaltStatus {
    pub fn is_halted(&self) -> bool {
        matches!(self, HaltStatus::HaltDef | HaltStatus::Halt)
    }

    pub fn code(&self) -> u8 {
        *self as u8
    }
}

/// Execution context for events and aliases ($nick, $chan, $snicks).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptContext {
    pub nick: Option<String>,
    pub chan: Option<String>,
    pub snicks: Option<String>,
    /// New nick for on NICK ($newnick); `nick` holds the old one.
    pub newnick: Option<String>,
    /// `user@host` of the event's user ($address), if known.
    pub address: Option<String>,
    /// Target of the event ($target): the channel or nick a message went to.
    pub target: Option<String>,
    /// Our own nick ($me), when the caller knows it.
    pub me: Option<String>,
    /// Network name ($network), when the caller knows it.
    pub network: Option<String>,
    /// The nick an event is about: the kicked nick, or the one of an OP, VOICE or HELP change
    /// ($knick, $opnick, $vnick, $hnick).
    pub knick: Option<String>,
    /// The mask of a BAN or UNBAN event ($banmask).
    pub banmask: Option<String>,
}

impl ScriptContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_nick(mut self, nick: impl Into<String>) -> Self {
        self.nick = Some(nick.into());
        self
    }

    pub fn with_chan(mut self, chan: impl Into<String>) -> Self {
        self.chan = Some(chan.into());
        self
    }

    pub fn with_snicks(mut self, snicks: impl Into<String>) -> Self {
        self.snicks = Some(snicks.into());
        self
    }

    pub fn with_me(mut self, me: impl Into<String>) -> Self {
        self.me = Some(me.into());
        self
    }

    pub fn with_network(mut self, network: impl Into<String>) -> Self {
        self.network = Some(network.into());
        self
    }
}

/// Supported remote IRC events: on TEXT, on JOIN, on PART, on QUIT, on NICK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptEvent {
    /// A lifecycle event that is not an IRC line: `start`, `exit`, `logon`, `connectfail` or
    /// `appactive`. `text` is what the handler sees as `$1-`; `network` and `me` are set only
    /// when the event belongs to a network.
    Lifecycle {
        event: &'static str,
        network: Option<String>,
        me: Option<String>,
        text: String,
    },
    Text {
        nick: String,
        target: String,
        text: String,
        /// `user@host` of the sender, if known.
        address: Option<String>,
        /// Network the event came from ($network).
        network: String,
        /// Our own nick on that network ($me).
        me: String,
    },
    Join {
        nick: String,
        channel: String,
        address: Option<String>,
        /// Network the event came from ($network).
        network: String,
        /// Our own nick on that network ($me).
        me: String,
    },
    Part {
        nick: String,
        channel: String,
        message: Option<String>,
        address: Option<String>,
        /// Network the event came from ($network).
        network: String,
        /// Our own nick on that network ($me).
        me: String,
    },
    Quit {
        nick: String,
        message: Option<String>,
        address: Option<String>,
        /// Network the event came from ($network).
        network: String,
        /// Our own nick on that network ($me).
        me: String,
    },
    Nick {
        nick: String,
        new_nick: String,
        address: Option<String>,
        /// Network the event came from ($network).
        network: String,
        /// Our own nick on that network ($me).
        me: String,
    },
    /// on ACTION: a CTCP action from a user; `text` has no frame.
    Action {
        nick: String,
        target: String,
        text: String,
        address: Option<String>,
        network: String,
        me: String,
    },
    /// on NOTICE: a notice from a user.
    Notice {
        nick: String,
        target: String,
        text: String,
        address: Option<String>,
        network: String,
        me: String,
    },
    /// on CTCPREPLY: a notice framed with \x01; `text` is what is inside the frame.
    CtcpReply {
        nick: String,
        text: String,
        address: Option<String>,
        network: String,
        me: String,
    },
    /// on CONNECT: the connection is up.
    Connect { network: String, me: String },
    /// on DISCONNECT: the connection ended; `reason` is the text for $1-.
    Disconnect { reason: String, network: String, me: String },
    /// on ERROR: an error line from the server or from the connection.
    Error { message: String, network: String, me: String },
    /// on KICK: `nick` kicked `knick` from `channel`.
    Kick {
        nick: String,
        knick: String,
        channel: String,
        reason: Option<String>,
        network: String,
        me: String,
    },
    /// on TOPIC: `nick` set the topic of `channel` to `text`.
    Topic {
        nick: String,
        channel: String,
        text: String,
        network: String,
        me: String,
    },
    /// on MODE: `nick` changed the modes of `channel`; `modes` is the mode string with its parameters.
    Mode {
        nick: String,
        channel: String,
        modes: String,
        network: String,
        me: String,
    },
    /// on USERMODE: our own user modes changed.
    UserMode { modes: String, network: String, me: String },
    /// on OP, DEOP, VOICE, DEVOICE, HELP, DEHELP, BAN and UNBAN: one change of a MODE line.
    /// `param` is the nick, or for BAN and UNBAN the mask.
    ModeChange {
        event: &'static str,
        nick: String,
        channel: String,
        param: String,
        network: String,
        me: String,
    },
}

impl ScriptEvent {
    pub fn context(&self) -> ScriptContext {
        let mut ctx = ScriptContext::default();
        match self {
            ScriptEvent::Text { nick, target, address, network, me, .. }
            | ScriptEvent::Action { nick, target, address, network, me, .. }
            | ScriptEvent::Notice { nick, target, address, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.target = Some(target.clone());
                ctx.chan = chan_of(target);
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Join { nick, channel, address, network, me } => {
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Part { nick, channel, address, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Quit { nick, address, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Nick { nick, new_nick, address, network, me } => {
                ctx.nick = Some(nick.clone());
                ctx.newnick = Some(new_nick.clone());
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::CtcpReply { nick, address, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.address = address.clone();
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Connect { network, me } => {
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Lifecycle { network, me, .. } => {
                ctx.network = network.clone();
                ctx.me = me.clone();
            }
            ScriptEvent::Disconnect { network, me, .. } => {
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Error { network, me, .. } => {
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Kick { nick, knick, channel, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.knick = Some(knick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Topic { nick, channel, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::Mode { nick, channel, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::UserMode { network, me, .. } => {
                ctx.nick = Some(me.clone());
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
            ScriptEvent::ModeChange { event, nick, channel, param, network, me, .. } => {
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                if *event == "ban" || *event == "unban" {
                    ctx.banmask = Some(param.clone());
                } else {
                    ctx.knick = Some(param.clone());
                }
                ctx.network = Some(network.clone());
                ctx.me = Some(me.clone());
            }
        }
        ctx
    }
}

/// Output of one trigger.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunResult {
    pub effects: Vec<Effect>,
    pub notices: Vec<Notice>,
    pub halt_status: HaltStatus,
}

/// What one timer run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerRun {
    /// The network the timer was made on.
    pub network: Option<String>,
    /// The timer's name or number.
    pub name: String,
    pub result: RunResult,
}

/// The statements of a timer command, read as the body of a one-line alias. A command that
/// does not parse is refused.
pub(crate) fn parse_timer_body(command: &str) -> Result<Vec<Statement>, String> {
    let script = parse_str(&format!("alias timer {{ {command} }}"));
    if script.unclassified.iter().any(|u| breaks_aliases(u.kind)) {
        return Err("the timer command does not parse".into());
    }
    script
        .items
        .iter()
        .find_map(|item| match item {
            Item::Alias(a) => Some(a.body.clone()),
            _ => None,
        })
        .ok_or_else(|| "the timer command does not parse".to_string())
}

/// Alias bodies per script, keyed by script name then lowercase alias name.
pub(crate) type Scripts = BTreeMap<String, HashMap<String, Arc<Vec<Statement>>>>;

/// Parse problems that break statement structure. Anything else the parser
/// reports concerns events, menus or dialogs, which M1 does not run.
fn breaks_aliases(kind: UnclassifiedKind) -> bool {
    matches!(
        kind,
        UnclassifiedKind::UnbalancedClose
            | UnclassifiedKind::StrayOpenBrace
            | UnclassifiedKind::UnterminatedBlock
            | UnclassifiedKind::NestingTooDeep
            | UnclassifiedKind::MalformedAliasHeader
            | UnclassifiedKind::MalformedEventHeader
    )
}

/// The text that an on TEXT, ACTION or NOTICE matchtext selects, or `None` when it does not match.
/// A level with `$` makes the matchtext a regular expression: `m/body/switches` or `/body/switches`.
/// `S` keeps the text as it is, control codes included. Otherwise the pattern is a wildcard.
fn matched_text(ev: &Event, pattern: &str, text: &str) -> Option<String> {
    let regex_level = ev.levels.iter().any(|l| l.starts_with('$'));
    let regex = pattern.strip_prefix('m').unwrap_or(pattern);
    if regex_level && regex.starts_with('/') {
        let found = super::regex_funcs::find(text, regex).ok()?;
        return (!found.matches.is_empty()).then_some(found.text);
    }
    wildcard_match(pattern, text).then(|| text.to_string())
}

/// The matchtext and target fields of on TEXT, on ACTION and on NOTICE: the text that matched.
fn text_fields_match(ev: &Event, text: &str, target: &str) -> Option<String> {
    let match_pat = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
    let target_pat = ev.fields.get(1).map(|s| s.as_str()).unwrap_or("*");
    if !target_matches(target_pat, target) {
        return None;
    }
    matched_text(ev, match_pat, text)
}

/// The target field of on TEXT: `*` any, `#` a channel, `?` a nick, else a wildcard.
fn target_matches(pattern: &str, target: &str) -> bool {
    match pattern {
        "*" => true,
        "#" => crate::text::is_channel_name(target),
        // `?`: a nick target, never null and never an `@` context. `=` is not
        // characterised yet and falls through to the plain wildcard.
        "?" => !target.is_empty() && !target.starts_with('@') && !crate::text::is_channel_name(target),
        // `@`: a target that begins with `@`.
        "@" => target.starts_with('@'),
        _ => wildcard_match(pattern, target),
    }
}

/// The channel field of on JOIN, KICK, TOPIC, MODE and the mode changes.
fn channel_fields_match(ev: &Event, channel: &str) -> bool {
    let pattern = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
    match pattern {
        "*" => true,
        "#" => crate::text::is_channel_name(channel),
        _ => wildcard_match(pattern, channel),
    }
}

/// A context with the network, our nick and the snicks list; the other fields are unset.
fn base_context(network: &str, me: &str, snicks: Option<&str>) -> ScriptContext {
    ScriptContext {
        network: Some(network.to_string()),
        me: Some(me.to_string()),
        snicks: snicks.map(str::to_string),
        ..Default::default()
    }
}

fn chan_of(target: &str) -> Option<String> {
    crate::text::is_channel_name(target).then(|| target.to_string())
}

/// Compares two characters: ASCII case-insensitively, or exactly when `case_sensitive`.
fn same_char(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a.to_ascii_lowercase() == b.to_ascii_lowercase()
    }
}

/// Case-insensitive wildcard matching with `*` (zero or more chars) and `?` (single char).
pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_mode(pattern, text, false)
}

/// The wildcard matcher with its case rule: `iswm` compares ASCII case-insensitively, `iswmcs` exactly.
pub(crate) fn wildcard_match_mode(pattern: &str, text: &str, case_sensitive: bool) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();
    let mut p = 0;
    let mut t = 0;
    let mut star_p = None;
    let mut star_t = 0;

    while t < t_chars.len() {
        // `&` is a word: one or more characters up to a space or the end, and only when a space
        // or the end of the pattern follows it.
        if p < p_chars.len()
            && p_chars[p] == '&'
            && (p + 1 == p_chars.len() || p_chars[p + 1] == ' ')
            && t_chars[t] != ' '
        {
            while t < t_chars.len() && t_chars[t] != ' ' {
                t += 1;
            }
            p += 1;
        } else if p < p_chars.len()
            && (p_chars[p] == '?'
                || same_char(p_chars[p], t_chars[t], case_sensitive))
        {
            p += 1;
            t += 1;
        } else if p < p_chars.len() && p_chars[p] == '*' {
            star_p = Some(p);
            p += 1;
            star_t = t;
        } else if let Some(sp) = star_p {
            p = sp + 1;
            star_t += 1;
            t = star_t;
        } else {
            return false;
        }
    }

    while p < p_chars.len() && p_chars[p] == '*' {
        p += 1;
    }

    p == p_chars.len()
}

pub struct Runtime {
    pub(super) scripts: Scripts,
    events: BTreeMap<String, Vec<Arc<Event>>>,
    /// Load order of the scripts (first loaded, smallest): the order their handlers run in.
    load_seq: BTreeMap<String, u64>,
    next_load_seq: u64,
    pub(super) globals: HashMap<String, String>,
    pub(super) limits: Limits,
    /// Generator state for `$rand`.
    pub(super) rng: u64,
    /// Hash tables, kept across alias runs.
    pub(super) hashes: super::hash_tables::Hashes,
    /// Results of the regular expression calls.
    pub(super) regex: super::regex_funcs::RegexState,
    /// Timers made by aliases and events; the driver runs the due ones.
    pub(super) timers: super::timers::TimerTable,
    /// The clock. Timers and runs read it; tests put their own clock here.
    pub(super) clock: Box<dyn Fn() -> Instant + Send>,
    /// The popup menus of the loaded scripts, in load order.
    pub(super) menus: Vec<super::menus::RegisteredMenu>,
    /// Counts the changes to the menus; an entry address from another version is refused.
    pub(super) menu_version: u64,
}

/// Seed for `$rand`, from the clock. Odd, so the generator state is never zero.
fn seed_from_clock() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos | 1
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Self {
        Runtime {
            scripts: Scripts::new(),
            events: BTreeMap::new(),
            load_seq: BTreeMap::new(),
            next_load_seq: 0,
            globals: HashMap::new(),
            limits,
            rng: seed_from_clock(),
            hashes: Default::default(),
            regex: Default::default(),
            timers: Default::default(),
            clock: Box::new(Instant::now),
            menus: Vec::new(),
            menu_version: 0,
        }
    }

    /// Replaces the aliases and events of `script_name` with those in `text`.
    /// A script that is too large or has a structural parse error keeps its old entries.
    pub fn load(&mut self, script_name: &str, text: &str) -> Vec<Notice> {
        self.load_accepted(script_name, text).1
    }

    /// Like `load`, and also says whether the script was taken. `false` means it was not loaded
    /// (too large, or a parse problem) and the old entries of that name stay.
    pub fn load_accepted(&mut self, script_name: &str, text: &str) -> (bool, Vec<Notice>) {
        let who = safe_name(script_name);
        if text.len() > self.limits.max_script_bytes {
            return (false, vec![format!(
                "script '{who}' is larger than {} bytes; not loaded",
                self.limits.max_script_bytes
            )]);
        }
        let script = parse_str(text);
        let broken: Vec<_> = script
            .unclassified
            .iter()
            .filter(|u| breaks_aliases(u.kind))
            .collect();
        if let Some(first) = broken.first() {
            return (false, vec![format!(
                "script '{who}' not loaded: {} parse problem(s), first '{}' at line {}; previous aliases kept",
                broken.len(),
                first.kind.as_str(),
                first.line
            )]);
        }
        let mut notices = Vec::new();
        let mut aliases = HashMap::new();
        let mut events = Vec::new();
        let mut menus = Vec::new();
        for item in &script.items {
            match item {
                Item::Alias(a) => {
                    if a.name.is_ascii() {
                        // Duplicates in one script: the first definition in line order is the one that runs.
                        aliases
                            .entry(a.name.to_ascii_lowercase())
                            .or_insert_with(|| Arc::new(a.body.clone()));
                    } else {
                        notices.push(format!(
                            "script '{who}': alias at line {} skipped, names must be ASCII",
                            a.line
                        ));
                    }
                }
                Item::Event(e) => {
                    events.push(Arc::new(e.clone()));
                }
                Item::Menu(m) => {
                    for window in m.windows.iter().filter(|w| w.starts_with('@')) {
                        notices.push(format!(
                            "script '{who}': menu for custom window {window} at line {} is not supported",
                            m.line
                        ));
                    }
                    menus.push(super::menus::RegisteredMenu {
                        script: script_name.to_ascii_lowercase(),
                        menu: Arc::new(m.clone()),
                    });
                }
                _ => {}
            }
        }
        // Script names are not case-sensitive (Windows files): the key is the lowercase name.
        let key = script_name.to_ascii_lowercase();
        self.scripts.insert(key.clone(), aliases);
        self.events.insert(key.clone(), events);
        self.next_load_seq += 1;
        self.load_seq.insert(key.clone(), self.next_load_seq);
        self.menus.retain(|m| m.script != key);
        self.menus.extend(menus);
        self.menu_version += 1;
        // A reloaded script's timers belong to the old version.
        self.timers.remove_owner(&key);
        (true, notices)
    }

    /// True when a loaded alias has this name (ASCII case-insensitive).
    pub fn has_alias(&self, name: &str) -> bool {
        self.find_alias(name).is_some()
    }

    /// Removes one script's aliases and events while retaining the runtime's global variables.
    pub fn unload(&mut self, script_name: &str) -> bool {
        let key = script_name.to_ascii_lowercase();
        let a = self.scripts.remove(&key).is_some();
        let b = self.events.remove(&key).is_some();
        self.load_seq.remove(&key);
        self.timers.remove_owner(&key);
        let menus_before = self.menus.len();
        self.menus.retain(|m| m.script != key);
        if self.menus.len() != menus_before {
            self.menu_version += 1;
        }
        a || b
    }

    fn find_alias(&self, name: &str) -> Option<Arc<Vec<Statement>>> {
        if !name.is_ascii() {
            return None;
        }
        let key = name.to_ascii_lowercase();
        self.scripts.values().find_map(|m| m.get(&key)).cloned()
    }

    /// The alias and the name of the script it is in: the owner of the timers its run makes.
    fn find_alias_owned(&self, name: &str) -> Option<(String, Arc<Vec<Statement>>)> {
        if !name.is_ascii() {
            return None;
        }
        let key = name.to_ascii_lowercase();
        self.scripts
            .iter()
            .find_map(|(script, m)| m.get(&key).map(|body| (script.clone(), Arc::clone(body))))
    }

    /// Runs one alias with an explicit execution context ($nick, $chan, $snicks).
    pub fn run_alias_with_context(
        &mut self,
        name: &str,
        args: &[String],
        context: &ScriptContext,
    ) -> RunResult {
        let Some((owner, body)) = self.find_alias_owned(name) else {
            return RunResult {
                effects: Vec::new(),
                notices: vec![format!("unknown alias '{}'", safe_name(name))],
                halt_status: HaltStatus::None,
            };
        };
        let now = (self.clock)();
        let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
        let mut exec = Exec::new_with_context(scripts, globals, limits, rng, hashes, regex, timers, now, context.clone());
        exec.owner = owner;
        exec.run_top(&name.to_ascii_lowercase(), body, args.to_vec());
        let (effects, notices, halt_status) = exec.finish();
        RunResult {
            effects,
            notices,
            halt_status,
        }
    }

    /// Runs one alias. Each element of `args` is one parameter (`$1`, `$2`, ...).
    /// A limit stop or an error ends the run with one notice, keeps the effects
    /// produced so far and leaves the runtime usable.
    pub fn run_alias(&mut self, name: &str, args: &[String]) -> RunResult {
        self.run_alias_with_context(name, args, &ScriptContext::default())
    }

    /// True when at least one timer is set. The app checks this every 100 ms and skips the work without timers.
    pub fn has_timers(&self) -> bool {
        self.timers.len() > 0
    }

    /// Removes the online timers of a network that disconnected.
    pub fn remove_network_timers(&mut self, network: &str) {
        self.timers.remove_network(network);
    }

    /// Runs the timers due now, earliest first, and returns what each run produced. A repeating
    /// timer waits for its next period. A run that a limit or an error stopped switches its timer
    /// off and adds one notice. `own_nick` gives our nick on a network, for `$!me`.
    pub fn fire_timers(&mut self, own_nick: &dyn Fn(&str) -> Option<String>) -> Vec<TimerRun> {
        let now = (self.clock)();
        let mut runs = Vec::new();
        for name in self.timers.due(now) {
            let Some(mut timer) = self.timers.take(&name) else {
                continue;
            };
            self.timers.start_run(&name);
            let network = timer.network.clone();
            let mut live = timer.context.clone();
            if let Some(me) = timer.network.as_deref().and_then(own_nick) {
                live.me = Some(me);
            }
            let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
            let mut exec = Exec::new_with_context(
                scripts,
                globals,
                limits,
                rng,
                hashes,
                regex,
                timers,
                now,
                timer.context.clone(),
            );
            exec.owner = timer.owner.clone();
            exec.frozen = Some(timer.clock);
            exec.live = Some(live);
            exec.run_top(&format!("timer {name}"), Arc::clone(&timer.body), Vec::new());
            let stopped = exec.stopped;
            let (effects, mut notices, _) = exec.finish();
            let switched_off = self.timers.end_run();

            if stopped {
                notices.push(format!("timer {name} is switched off after its run stopped"));
            } else {
                let more = match timer.reps_left {
                    None => true,
                    Some(n) if n > 1 => {
                        timer.reps_left = Some(n - 1);
                        true
                    }
                    Some(_) => false,
                };
                // A body that made a timer with this name has replaced this one.
                if more && !switched_off && !self.timers.contains(&name) {
                    let next = timer.next_due + timer.period;
                    timer.next_due = if next > now { next } else { now + timer.period };
                    self.timers.put(timer);
                }
            }
            runs.push(TimerRun {
                network,
                name,
                result: RunResult {
                    effects,
                    notices,
                    halt_status: HaltStatus::None,
                },
            });
        }
        runs
    }

    fn match_event(
        ev: &Event,
        event: &ScriptEvent,
        snicks: Option<&str>,
    ) -> Option<(ScriptContext, Vec<String>)> {
        match event {
            ScriptEvent::Text { nick, target, text, address, network, me } => {
                if ev.event != "text" {
                    return None;
                }
                let match_pat = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
                let target_pat = ev.fields.get(1).map(|s| s.as_str()).unwrap_or("*");

                let Some(matched) = matched_text(ev, match_pat, text) else {
                    return None;
                };

                if !target_matches(target_pat, target) {
                    return None;
                }

                let is_chan = crate::text::is_channel_name(target);
                let ctx = ScriptContext {
                    nick: Some(nick.clone()),
                    chan: if is_chan { Some(target.clone()) } else { None },
                    snicks: snicks.map(|s| s.to_string()),
                    newnick: None,
                    knick: None,
                    banmask: None,
                    address: address.clone(),
                    target: Some(target.clone()),
                    me: Some(me.clone()),
                    network: Some(network.clone()),
                };
                let args = super::value::split_args(&matched);
                Some((ctx, args))
            }
            ScriptEvent::Join { nick, channel, address, network, me } => {
                if ev.event != "join" {
                    return None;
                }
                let target_pat = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
                let target_matches = match target_pat {
                    "*" => true,
                    "#" => crate::text::is_channel_name(channel),
                    _ => wildcard_match(target_pat, channel),
                };
                if !target_matches {
                    return None;
                }

                let ctx = ScriptContext {
                    nick: Some(nick.clone()),
                    chan: Some(channel.clone()),
                    snicks: snicks.map(|s| s.to_string()),
                    newnick: None,
                    knick: None,
                    banmask: None,
                    address: address.clone(),
                    target: Some(channel.clone()),
                    me: Some(me.clone()),
                    network: Some(network.clone()),
                };
                Some((ctx, Vec::new()))
            }
            ScriptEvent::Part { nick, channel, message, address, network, me } => {
                if ev.event != "part" {
                    return None;
                }
                let target_pat = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
                let target_matches = match target_pat {
                    "*" => true,
                    "#" => crate::text::is_channel_name(channel),
                    _ => wildcard_match(target_pat, channel),
                };
                if !target_matches {
                    return None;
                }

                let ctx = ScriptContext {
                    nick: Some(nick.clone()),
                    chan: Some(channel.clone()),
                    snicks: snicks.map(|s| s.to_string()),
                    newnick: None,
                    knick: None,
                    banmask: None,
                    address: address.clone(),
                    target: Some(channel.clone()),
                    me: Some(me.clone()),
                    network: Some(network.clone()),
                };
                let args = message
                    .as_deref()
                    .map(super::value::split_args)
                    .unwrap_or_default();
                Some((ctx, args))
            }
            ScriptEvent::Nick { nick, new_nick, address, network, me } => {
                if ev.event != "nick" {
                    return None;
                }
                let ctx = ScriptContext {
                    nick: Some(nick.clone()),
                    chan: None,
                    snicks: snicks.map(|s| s.to_string()),
                    newnick: Some(new_nick.clone()),
                    knick: None,
                    banmask: None,
                    address: address.clone(),
                    target: None,
                    me: Some(me.clone()),
                    network: Some(network.clone()),
                };
                Some((ctx, Vec::new()))
            }
            ScriptEvent::Action { nick, target, text, address, network, me } => {
                if ev.event != "action" {
                    return None;
                }
                let Some(matched) = text_fields_match(ev, text, target) else {
                    return None;
                };
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.chan = chan_of(target);
                ctx.target = Some(target.clone());
                ctx.address = address.clone();
                Some((ctx, super::value::split_args(&matched)))
            }
            ScriptEvent::Notice { nick, target, text, address, network, me } => {
                if ev.event != "notice" {
                    return None;
                }
                let Some(matched) = text_fields_match(ev, text, target) else {
                    return None;
                };
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.chan = chan_of(target);
                ctx.target = Some(target.clone());
                ctx.address = address.clone();
                Some((ctx, super::value::split_args(&matched)))
            }
            ScriptEvent::CtcpReply { nick, text, address, network, me } => {
                if ev.event != "ctcpreply" {
                    return None;
                }
                let pattern = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
                if !wildcard_match(pattern, text) {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.address = address.clone();
                // $1- is what follows the command word: `VERSION Other 1.0` gives `Other 1.0`.
                let rest = text.split_once(' ').map_or("", |(_, rest)| rest);
                Some((ctx, super::value::split_args(rest)))
            }
            ScriptEvent::Lifecycle { event, network, me, text } => {
                if ev.event != *event {
                    return None;
                }
                let ctx = ScriptContext {
                    network: network.clone(),
                    me: me.clone(),
                    snicks: snicks.map(|s| s.to_string()),
                    ..ScriptContext::default()
                };
                Some((ctx, super::value::split_args(text)))
            }
            ScriptEvent::Connect { network, me } => {
                if ev.event != "connect" {
                    return None;
                }
                Some((base_context(network, me, snicks), Vec::new()))
            }
            ScriptEvent::Disconnect { reason, network, me } => {
                if ev.event != "disconnect" {
                    return None;
                }
                Some((base_context(network, me, snicks), super::value::split_args(reason)))
            }
            ScriptEvent::Error { message, network, me } => {
                if ev.event != "error" {
                    return None;
                }
                let pattern = ev.fields.first().map(|s| s.as_str()).unwrap_or("*");
                if !wildcard_match(pattern, message) {
                    return None;
                }
                Some((base_context(network, me, snicks), super::value::split_args(message)))
            }
            ScriptEvent::Kick { nick, knick, channel, reason, network, me } => {
                if ev.event != "kick" || !channel_fields_match(ev, channel) {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.knick = Some(knick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                let args = reason.as_deref().map(super::value::split_args).unwrap_or_default();
                Some((ctx, args))
            }
            ScriptEvent::Topic { nick, channel, text, network, me } => {
                if ev.event != "topic" || !channel_fields_match(ev, channel) {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                Some((ctx, super::value::split_args(text)))
            }
            ScriptEvent::Mode { nick, channel, modes, network, me } => {
                if ev.event != "mode" || !channel_fields_match(ev, channel) {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                Some((ctx, super::value::split_args(modes)))
            }
            ScriptEvent::UserMode { modes, network, me } => {
                if ev.event != "usermode" {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(me.clone());
                Some((ctx, super::value::split_args(modes)))
            }
            ScriptEvent::ModeChange { event, nick, channel, param, network, me } => {
                if ev.event.as_str() != *event || !channel_fields_match(ev, channel) {
                    return None;
                }
                let mut ctx = base_context(network, me, snicks);
                ctx.nick = Some(nick.clone());
                ctx.chan = Some(channel.clone());
                ctx.target = Some(channel.clone());
                if *event == "ban" || *event == "unban" {
                    ctx.banmask = Some(param.clone());
                } else {
                    ctx.knick = Some(param.clone());
                }
                Some((ctx, vec![param.clone()]))
            }
            ScriptEvent::Quit { nick, message, address, network, me } => {
                if ev.event != "quit" {
                    return None;
                }
                if let Some(pat) = ev.fields.first() {
                    if pat != "*" && !message.as_deref().is_some_and(|m| wildcard_match(pat, m)) {
                        return None;
                    }
                }
                let ctx = ScriptContext {
                    nick: Some(nick.clone()),
                    chan: None,
                    snicks: snicks.map(|s| s.to_string()),
                    newnick: None,
                    knick: None,
                    banmask: None,
                    address: address.clone(),
                    target: None,
                    me: Some(me.clone()),
                    network: Some(network.clone()),
                };
                let args = message
                    .as_deref()
                    .map(super::value::split_args)
                    .unwrap_or_default();
                Some((ctx, args))
            }
        }
    }

    /// Dispatches an IRC event through all loaded scripts' matching event handlers.
    /// Returns combined effects, notices, and the final HaltStatus.
    /// Status 2 (halt) ends the rest of the same script's handlers for this event; other scripts still run.
    /// The same-script part is not yet confirmed (test 1b is unexplained).
    /// Status 1 (haltdef) lets the remaining handlers run. It does not suppress the default display:
    /// a runtime test kept the line visible, and the app never reads `halt_status`.
    pub fn dispatch_event(&mut self, event: &ScriptEvent, snicks: Option<&str>) -> RunResult {
        let mut matching = Vec::new();
        for (script, events) in self.events.iter() {
            for ev in events {
                if let Some((ctx, args)) = Self::match_event(ev, event, snicks) {
                    matching.push((script.clone(), ev.clone(), ctx, args));
                }
            }
        }

        // Scripts run in the order they were loaded: the script loaded first runs first.
        // The handlers of one script keep their order.
        matching.sort_by_key(|(owner, _, _, _)| self.load_seq.get(owner).copied().unwrap_or(u64::MAX));

        let mut combined_effects = Vec::new();
        let mut combined_notices = Vec::new();
        let mut halt_status = HaltStatus::None;
        let mut halted_script: Option<String> = None;

        let now = (self.clock)();
        for (owner, ev, ctx, args) in matching {
            // A halt ends the rest of its own script's handlers for this event. Other scripts still run:
            // in a runtime test a halt did not stop a handler of a script loaded later.
            if halted_script.as_deref() == Some(owner.as_str()) {
                continue;
            }
            let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
            let mut exec = Exec::new_with_context(scripts, globals, limits, rng, hashes, regex, timers, now, ctx);
            exec.owner = owner.clone();
            exec.run_top_event(&ev.event, Arc::clone(&ev.body), args);
            let (effects, notices, status) = exec.finish();

            combined_effects.extend(effects);
            combined_notices.extend(notices);

            if status == HaltStatus::Halt {
                halt_status = HaltStatus::Halt;
                halted_script = Some(owner);
            } else if status == HaltStatus::HaltDef && halt_status != HaltStatus::Halt {
                halt_status = HaltStatus::HaltDef;
            }
        }

        RunResult {
            effects: combined_effects,
            notices: combined_notices,
            halt_status,
        }
    }
    /// Runs the handlers of one script for a lifecycle event (`load`, `unload`). Other scripts
    /// see nothing. `context` gives the network and our nick the handlers run for.
    pub fn dispatch_lifecycle(&mut self, script_name: &str, event: &str, context: &ScriptContext) -> RunResult {
        let key = script_name.to_ascii_lowercase();
        let handlers: Vec<Arc<Event>> = self
            .events
            .get(&key)
            .map(|list| list.iter().filter(|ev| ev.event == event).cloned().collect())
            .unwrap_or_default();
        let now = (self.clock)();
        let mut combined_effects = Vec::new();
        let mut combined_notices = Vec::new();
        for ev in handlers {
            let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
            let mut exec = Exec::new_with_context(
                scripts,
                globals,
                limits,
                rng,
                hashes,
                regex,
                timers,
                now,
                context.clone(),
            );
            exec.owner = script_name.to_ascii_lowercase();
            exec.run_top_event(&ev.event, Arc::clone(&ev.body), Vec::new());
            let (effects, notices, _status) = exec.finish();
            combined_effects.extend(effects);
            combined_notices.extend(notices);
        }
        RunResult {
            effects: combined_effects,
            notices: combined_notices,
            halt_status: HaltStatus::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn rt(script: &str) -> Runtime {
        let mut r = Runtime::new();
        let notices = r.load("t.mrc", script);
        assert!(notices.is_empty(), "load notices: {notices:?}");
        r
    }

    fn run(r: &mut Runtime, name: &str, args: &[&str]) -> RunResult {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        r.run_alias(name, &args)
    }

    fn echoes(res: &RunResult) -> Vec<&str> {
        res.effects.iter().map(|e| e.text()).collect()
    }

    // ---- plan M1 acceptance tests

    #[test]
    fn hi_alias_echoes_hello_there() {
        let mut r = rt("alias hi { echo Hello $1 }");
        let res = run(&mut r, "hi", &["there"]);
        assert_eq!(res.effects, vec![Effect::Echo("Hello there".into())]);
        assert!(res.notices.is_empty());
    }

    #[test]
    fn one_line_alias_form() {
        let mut r = rt("alias hi echo Hello $1\nalias bye { echo Bye $1 }\n");
        assert_eq!(echoes(&run(&mut r, "hi", &["you"])), ["Hello you"]);
        assert_eq!(echoes(&run(&mut r, "bye", &["you"])), ["Bye you"]);
    }

    #[test]
    fn number_example_block_form() {
        let mut r = rt("alias number {\n  if ($1 == 1) { echo One }\n  elseif ($1 == 2) { echo Two }\n  else { echo Unknown number! }\n}\n");
        for (arg, want) in [("1", "One"), ("2", "Two"), ("7", "Unknown number!")] {
            let res = run(&mut r, "number", &[arg]);
            assert_eq!(echoes(&res), [want], "arg {arg}");
            assert!(res.notices.is_empty());
        }
    }

    #[test]
    fn number_example_inline_form() {
        let mut r = rt("alias number {\n  if ($1 == 1) echo One\n  elseif ($1 == 2) echo Two\n  else echo Unknown number!\n}\n");
        for (arg, want) in [("1", "One"), ("2", "Two"), ("7", "Unknown number!")] {
            assert_eq!(echoes(&run(&mut r, "number", &[arg])), [want]);
        }
    }

    #[test]
    fn set_inc_echo_is_visible_to_a_second_alias() {
        let mut r = rt("alias setit { set %a 5 | inc %a 2 | echo %a }\nalias show { echo %a }\n");
        assert_eq!(echoes(&run(&mut r, "setit", &[])), ["7"]);
        assert_eq!(echoes(&run(&mut r, "show", &[])), ["7"]);
        let mut r = rt("alias s { set %a 5 }\nalias i { inc %a 2 }\nalias show { echo %a }\nalias d { dec %a 10 | echo %a }\n");
        run(&mut r, "s", &[]);
        run(&mut r, "i", &[]);
        assert_eq!(echoes(&run(&mut r, "show", &[])), ["7"]);
        assert_eq!(echoes(&run(&mut r, "d", &[])), ["-3"]);
    }

    #[test]
    fn unloading_one_script_keeps_other_aliases_and_globals() {
        let mut r = Runtime::new();
        assert!(r.load("first.mrc", "alias first { set %shared 42 }").is_empty());
        assert!(r.load("second.mrc", "alias second { echo %shared }").is_empty());
        run(&mut r, "first", &[]);
        assert!(r.unload("first.mrc"));
        assert!(!r.unload("first.mrc"));
        assert!(!r.has_alias("first"));
        assert_eq!(echoes(&run(&mut r, "second", &[])), ["42"]);
    }

    #[test]
    fn var_local_is_gone_after_return() {
        let mut r = rt("alias v { var %x = 3 | echo in:%x }\nalias w { echo out:%x }\n");
        assert_eq!(echoes(&run(&mut r, "v", &[])), ["in:3"]);
        assert_eq!(echoes(&run(&mut r, "w", &[])), ["out:"]);
        assert!(r.globals.is_empty());
    }

    #[test]
    fn local_takes_precedence_unless_g() {
        let mut r = rt("alias p {\n  set %x global\n  var %x = local\n  echo %x\n  set %x changed\n  echo %x\n  set -g %x forced\n  echo %x\n  unset -g %x\n  echo %x\n}\nalias q { echo %x }\n");
        assert_eq!(
            echoes(&run(&mut r, "p", &[])),
            ["local", "changed", "changed", "changed"]
        );
        // The -g write reached the global before it was unset; nothing is left.
        assert_eq!(echoes(&run(&mut r, "q", &[])), [""]);
        let mut r = rt("alias p { set %y 1 | var %y = 2 | set -g %y 3 }\nalias q { echo %y }\n");
        run(&mut r, "p", &[]);
        assert_eq!(echoes(&run(&mut r, "q", &[])), ["3"]);
    }

    #[test]
    fn while_forever_ends_with_a_limit_notice_and_runtime_still_works() {
        let mut r = rt("alias spin { while (1) { } }\nalias ok { echo fine }\n");
        let res = run(&mut r, "spin", &[]);
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("script limit reached"));
        assert_eq!(echoes(&run(&mut r, "ok", &[])), ["fine"]);
        // And the same alias can be triggered again.
        assert_eq!(run(&mut r, "spin", &[]).notices.len(), 1);
    }

    #[test]
    fn statement_limit_is_20000() {
        let mut r = Runtime::with_limits(Limits {
            wall_time: Duration::from_secs(60),
            ..Limits::default()
        });
        r.load("t", "alias spin { while (1) { } }");
        let res = run(&mut r, "spin", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("20000 statements"), "{:?}", res.notices);
    }

    #[test]
    fn wall_time_limit_is_checked_every_256_statements() {
        let mut r = Runtime::with_limits(Limits {
            wall_time: Duration::ZERO,
            ..Limits::default()
        });
        r.load(
            "t",
            "alias few { echo a | echo b }\nalias spin { while (1) { } }\n",
        );
        // Under 256 statements the clock is never read.
        let res = run(&mut r, "few", &[]);
        assert_eq!(echoes(&res), ["a", "b"]);
        assert!(res.notices.is_empty());
        let res = run(&mut r, "spin", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("wall time"), "{:?}", res.notices);
    }

    fn chain(n: usize) -> String {
        let mut s = String::new();
        for i in 1..n {
            s.push_str(&format!("alias a{i} {{ echo in{i} | a{} }}\n", i + 1));
        }
        s.push_str(&format!("alias a{n} {{ echo end }}\n"));
        s
    }

    #[test]
    fn forty_deep_chain_stops_at_depth_limit() {
        let mut r = rt(&chain(40));
        let res = run(&mut r, "a1", &[]);
        assert_eq!(res.effects.len(), 16, "one echo per allowed call");
        assert!(!echoes(&res).contains(&"end"));
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("call depth 16"), "{:?}", res.notices);
        // Usable afterwards; a short chain runs to its end.
        let mut r2 = rt(&chain(16));
        let res = run(&mut r2, "a1", &[]);
        assert_eq!(echoes(&res).last(), Some(&"end"));
        assert!(res.notices.is_empty());
        let res = run(&mut r, "a20", &[]);
        assert_eq!(res.notices.len(), 1);
    }

    #[test]
    fn run_and_dll_are_refused_with_one_notice_and_abort() {
        let mut r = rt("alias r1 { run notepad | echo after }\nalias d1 { dll x y | echo after }\nalias r2 { echo before | run notepad }\n");
        for name in ["r1", "d1"] {
            let res = run(&mut r, name, &[]);
            assert!(res.effects.is_empty(), "{name}");
            assert_eq!(res.notices.len(), 1, "{name}");
        }
        assert!(run(&mut r, "r1", &[]).notices[0].contains("'run'"));
        assert!(run(&mut r, "d1", &[]).notices[0].contains("'dll'"));
        // Effects before the refused command stay; the rest is aborted.
        let res = run(&mut r, "r2", &[]);
        assert_eq!(echoes(&res), ["before"]);
        assert_eq!(res.notices.len(), 1);
    }

    #[test]
    fn unknown_command_gives_a_notice_naming_it() {
        let mut r = rt("alias u { frobnicate now }\nalias v { echo a | $foo | echo b }\nalias w { %x=5 }\n");
        let res = run(&mut r, "u", &[]);
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("'frobnicate'"));
        // Statements that start with an identifier or variable are refused too.
        let res = run(&mut r, "v", &[]);
        assert_eq!(echoes(&res), ["a"]);
        assert_eq!(res.notices.len(), 1);
        assert_eq!(run(&mut r, "w", &[]).notices.len(), 1);
        // Unknown alias name at the top.
        let res = run(&mut r, "nope", &[]);
        assert_eq!(res.notices, vec!["unknown alias 'nope'".to_string()]);
    }

    // ---- more behaviour

    #[test]
    fn alias_names_ignore_ascii_case() {
        let mut r = rt("alias HeLLo { echo hi }\nalias caller { HELLO }\n");
        assert!(r.has_alias("hello") && r.has_alias("HELLO"));
        assert!(!r.has_alias("nope"));
        assert_eq!(echoes(&run(&mut r, "hEllO", &[])), ["hi"]);
        assert_eq!(echoes(&run(&mut r, "caller", &[])), ["hi"]);
    }

    #[test]
    fn variable_names_ignore_ascii_case_and_inc_dec_keep_fractions() {
        let mut r = rt("alias t { set %Amount 1.25 | inc %AMOUNT 0.5 | echo %amount | dec %aMoUnT 0.125 | echo %AMOUNT | unset %amount | echo <%AMOUNT> }");
        let result = run(&mut r, "t", &[]);
        assert!(result.notices.is_empty());
        assert_eq!(echoes(&result), ["1.75", "1.625", "<>"]);
    }

    #[test]
    fn calc_uses_floating_point_and_formats_six_places() {
        let mut r = rt("alias t { set %x 1.25 | echo $calc(%X * 2 + 1 / 4) | echo $calc(%missing + 2) | echo $calc(1 / 3) }");
        let result = run(&mut r, "t", &[]);
        assert!(result.notices.is_empty(), "{:?}", result.notices);
        assert_eq!(echoes(&result), ["2.75", "2", "0.333333"]);
    }

    #[test]
    fn calc_rejects_invalid_expressions_and_does_not_execute_expanded_operators() {
        let mut r = rt("alias t { set %x $1 | echo $calc(%x * 3) }\nalias d { echo $calc(1 / 0) }\nalias n { echo $calc($calc(1 + 1) + 1) }");
        for name in ["t", "d", "n"] {
            // `a+b` is text: `/set` computes numbers only, so $calc has no number to take.
            let result = run(&mut r, name, &["a+b"]);
            assert!(result.effects.is_empty());
            assert!(!result.notices.is_empty());
        }
    }

    #[test]
    fn self_and_indirect_calls_are_refused() {
        let mut r = rt("alias self { echo x | self }\nalias a { echo a | b }\nalias b { echo b | a }\n");
        // A direct self call takes the built-in branch; `self` is no built-in.
        let res = run(&mut r, "self", &[]);
        assert_eq!(res.effects.len(), 1);
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("unknown or unsupported command"));
        // a -> b -> a ... runs to the call depth, which ends the cycle.
        let res = run(&mut r, "a", &[]);
        assert!(res.effects.len() >= 2);
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("call depth 16"), "{:?}", res.notices);
    }

    #[test]
    fn params_follow_u5() {
        let mut r = rt("alias p { echo [$0] [$1] [$2-] [$2-3] [$1-] [$9] [$3-2] [$0-] }");
        let res = run(&mut r, "p", &["a", "b", "c", "d"]);
        assert_eq!(echoes(&res), ["[4] [a] [b c d] [b c] [a b c d] [] [] [4-]"]);
        let res = run(&mut r, "p", &[]);
        assert_eq!(echoes(&res), ["[0] [] [] [] [] [] [] [0-]"]);
        // Parameters are text slots: inner spacing is kept, slots join with one space.
        let mut r = rt("alias q { echo $1- }");
        assert_eq!(echoes(&run(&mut r, "q", &["a  b", "c"])), ["a  b c"]);
        // Calling from a script splits at single spaces and drops empty pieces.
        let mut r = rt("alias q { echo $0:$1-:$2 }\nalias c { q   x   y  z }\n");
        assert_eq!(echoes(&run(&mut r, "c", &[])), ["3:x y z:y"]);
    }

    #[test]
    fn arithmetic_in_assignments() {
        let mut r = rt("alias m {\n  %x = 2 + 3 * 4\n  echo %x\n  var %y = (%x - 4) / 5\n  echo %y\n  %z = %x * [ 1 + 1 ]\n  echo %z\n  var %w = hello world\n  echo %w\n  inc %x $1\n  echo %x\n  dec %x\n  echo %x\n  set %s 1 + 1\n  echo %s\n  %n = -5\n  echo %n %unset+1\n}\n");
        let res = run(&mut r, "m", &["10"]);
        assert_eq!(
            echoes(&res),
            ["14", "2", "28", "hello world", "24", "23", "2", "-5 +1"]
        );
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        // An unset variable is 0 in arithmetic and empty in text.
        let mut r = rt("alias u { %a = %nope + 1 | echo %a<%nope> }");
        assert_eq!(echoes(&run(&mut r, "u", &[])), ["1<>"]);
    }

    #[test]
    fn operand_text_is_not_arithmetic_when_the_source_is_not() {
        // A value that looks like a sum stays text unless the source is one.
        let mut r = rt("alias t { %a = $1 | echo %a }\nalias u { %a = $1 + 0 | echo %a }\n");
        assert_eq!(echoes(&run(&mut r, "t", &["2020-01-02"])), ["2020-01-02"]);
        assert_eq!(echoes(&run(&mut r, "u", &["2020-01-02"])), ["0"]);
    }

    #[test]
    fn division_by_zero_and_overflow_give_one_notice() {
        let mut r = rt("alias d { echo a | %x = 1 / 0 | echo b }\nalias o { %x = 9223372036854775807 | inc %x | echo b }\nalias p { %x = 1e308 | inc %x 1e308 | echo b }\nalias q { %x = 2 * [ 1 + [ 2 ] ] }\n");
        let res = run(&mut r, "d", &[]);
        assert_eq!(echoes(&res), ["a"]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("division by zero"));
        // inc and dec are float arithmetic: 2^63 - 1 plus one is no overflow.
        let res = run(&mut r, "o", &[]);
        assert_eq!(echoes(&res), ["b"]);
        let res = run(&mut r, "p", &[]);
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("outside the number range"));
        let res = run(&mut r, "q", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("nested"));
    }

    #[test]
    fn conditions() {
        let mut r = rt("alias c {\n  if ($1 == $2 && $3 != x) echo and\n  if ($1 == a || $2 > 5) echo or\n  if (!($1 == a)) echo not\n  if (%unset == $null) echo unset\n  if ($1 === A) echo cs\n  if ($1 == A) echo ci\n  if ($2 < $3) echo lt\n  if (1) echo one\n  if (0) echo zero\n  if (abc) echo text\n}\n");
        let res = run(&mut r, "c", &["a", "a", "y"]);
        assert_eq!(echoes(&res), ["and", "or", "unset", "ci", "one"]);
        let res = run(&mut r, "c", &["b", "7", "9"]);
        assert_eq!(echoes(&res), ["or", "not", "unset", "lt", "one"]);
    }

    #[test]
    fn unsupported_condition_operator_is_a_notice() {
        let mut r = rt("alias c { echo a | if ($1 isin abc) echo yes | echo b }");
        let res = run(&mut r, "c", &["a"]);
        assert_eq!(echoes(&res), ["a"]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("'isin'"));
    }

    #[test]
    fn halt_return_break_continue_and_goto() {
        let mut r = rt("alias h { echo a | halt | echo b }\nalias h2 { echo x | h | echo y }\nalias ret { echo a | return | echo b }\nalias ret2 { echo x | ret | echo y }\nalias lp {\n  set %i 0\n  while (%i < 6) {\n    inc %i\n    if (%i == 2) continue\n    if (%i == 5) break\n    echo i%i\n  }\n  echo done\n}\nalias gt {\n  set %n 0\n  :top\n  inc %n\n  if (%n < 3) goto top\n  echo n%n\n  goto end\n  echo skipped\n  :end\n  echo fin\n}\nalias gout {\n  while (1) {\n    echo in\n    goto out\n  }\n  echo not\n  :out\n  echo out\n}\nalias nolabel { goto nowhere }\nalias brk { break }\nalias retl { while (1) { return } }\n");
        assert_eq!(echoes(&run(&mut r, "h", &[])), ["a"]);
        // halt ends everything, return only the current alias.
        assert_eq!(echoes(&run(&mut r, "h2", &[])), ["x", "a"]);
        assert_eq!(echoes(&run(&mut r, "ret", &[])), ["a"]);
        assert_eq!(echoes(&run(&mut r, "ret2", &[])), ["x", "a", "y"]);
        assert_eq!(echoes(&run(&mut r, "lp", &[])), ["i1", "i3", "i4", "done"]);
        assert_eq!(echoes(&run(&mut r, "gt", &[])), ["n3", "fin"]);
        assert_eq!(echoes(&run(&mut r, "gout", &[])), ["in", "out"]);
        for name in ["nolabel", "brk"] {
            let res = run(&mut r, name, &[]);
            assert_eq!(res.notices.len(), 1, "{name}");
        }
        let res = run(&mut r, "retl", &[]);
        assert!(res.notices.is_empty());
        for name in ["h", "ret", "lp", "gt", "gout"] {
            assert!(run(&mut r, name, &[]).notices.is_empty(), "{name}");
        }
    }

    #[test]
    fn stray_else_is_a_notice() {
        let mut r = rt("alias e { else echo x }");
        assert_eq!(run(&mut r, "e", &[]).notices.len(), 1);
    }

    #[test]
    fn echo_switches_come_from_the_source_only() {
        let mut r = rt("alias e { echo -a $1 | echo -st x | echo - - | echo -a }");
        let res = run(&mut r, "e", &["-s hi"]);
        assert_eq!(echoes(&res), ["-s hi", "x", "- -", ""]);
    }

    #[test]
    fn unknown_identifier_is_empty_with_one_notice_per_name() {
        let mut r = rt("alias i { echo a$foo b$FOO c$null d$bar }");
        let res = run(&mut r, "i", &[]);
        assert_eq!(echoes(&res), ["a b c d"]);
        assert_eq!(res.notices.len(), 2);
    }

    // ---- limits

    #[test]
    fn echo_line_limit() {
        let mut r = rt("alias e { while (1) { echo x } }");
        let res = run(&mut r, "e", &[]);
        assert_eq!(res.effects.len(), 200);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("200 echo lines"));
    }

    #[test]
    fn string_limit() {
        let mut r = rt("alias s { %x = $1 | %x = %x $1 | echo ok }\nalias t { echo $1 $1 }\n");
        let big = "a".repeat(40 * 1024);
        let res = r.run_alias("s", &[big.clone()]);
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("string value"), "{:?}", res.notices);
        assert_eq!(r.run_alias("t", &[big.clone()]).notices.len(), 1);
        // A single oversize parameter is refused up front.
        let huge = "a".repeat(64 * 1024 + 1);
        assert_eq!(r.run_alias("t", &[huge]).notices.len(), 1);
        // Exactly at the limit is fine.
        let at = "a".repeat(64 * 1024);
        let res = r.run_alias("s", &[at]);
        assert_eq!(res.notices.len(), 1); // second append goes over
        assert!(r.run_alias("t", &["x".to_string()]).notices.is_empty());
    }

    #[test]
    fn variable_count_limit_and_runtime_stays_usable() {
        // Variable names cannot be computed, so fill the table with one
        // statement per name.
        let mut script = String::from("alias big {\n");
        for i in 0..60 {
            script.push_str(&format!("  set %v{i} 1\n"));
        }
        script.push_str("}\n");
        let mut r2 = Runtime::with_limits(Limits {
            max_variables: 50,
            ..Limits::default()
        });
        r2.load("t", &script);
        let res = r2.run_alias("big", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("50 variables"));
        assert_eq!(r2.globals.len(), 50);
        // Existing variables can still be updated and the runtime works.
        r2.load("u", "alias again { set %v0 2 | echo %v0 }");
        assert_eq!(echoes(&r2.run_alias("again", &[])), ["2"]);
        // Locals count too.
        let mut locals = String::from("alias loc {\n");
        for i in 0..60 {
            locals.push_str(&format!("  var %l{i} = 1\n"));
        }
        locals.push_str("}\n");
        let mut r3 = Runtime::with_limits(Limits {
            max_variables: 50,
            ..Limits::default()
        });
        r3.load("t", &locals);
        assert_eq!(r3.run_alias("loc", &[]).notices.len(), 1);
    }

    #[test]
    fn nesting_limit_bounds_the_stack() {
        // 16 aliases, each with several nested blocks, run without trouble.
        let mut script = String::new();
        for i in 1..=16 {
            script.push_str(&format!("alias n{i} {{\n"));
            for _ in 0..7 {
                script.push_str("if (1) {\n");
            }
            if i < 16 {
                script.push_str(&format!("n{}\n", i + 1));
            } else {
                script.push_str("echo bottom\n");
            }
            for _ in 0..7 {
                script.push_str("}\n");
            }
            script.push_str("}\n");
        }
        let mut r = rt(&script);
        let res = run(&mut r, "n1", &[]);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(echoes(&res), ["bottom"]);
        // A tighter limit reports a notice instead of overflowing.
        let mut tight = Runtime::with_limits(Limits {
            max_nesting: 10,
            ..Limits::default()
        });
        tight.load("t", &script);
        let res = tight.run_alias("n1", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("nesting"));
    }

    // ---- loading

    #[test]
    fn load_replaces_only_that_script() {
        let mut r = Runtime::new();
        assert!(r.load("a.mrc", "alias one { echo 1 }\nalias two { echo 2 }").is_empty());
        assert!(r.load("b.mrc", "alias three { echo 3 }").is_empty());
        assert!(r.load("a.mrc", "alias one { echo 11 }").is_empty());
        assert_eq!(echoes(&run(&mut r, "one", &[])), ["11"]);
        assert!(!r.has_alias("two"), "rebuilt: old alias of a.mrc is gone");
        assert_eq!(echoes(&run(&mut r, "three", &[])), ["3"]);
    }

    #[test]
    fn parse_failure_keeps_old_aliases() {
        let mut r = Runtime::new();
        r.load("a.mrc", "alias one { echo old }");
        let notices = r.load("a.mrc", "alias one { echo new\n");
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("previous aliases kept"));
        assert_eq!(echoes(&run(&mut r, "one", &[])), ["old"]);
        let notices = r.load("a.mrc", "alias one { echo new } }\n");
        assert_eq!(notices.len(), 1);
        assert_eq!(echoes(&run(&mut r, "one", &[])), ["old"]);
        // A first load that fails registers nothing.
        let notices = r.load("b.mrc", "alias two { echo x\n");
        assert_eq!(notices.len(), 1);
        assert!(!r.has_alias("two"));
    }

    #[test]
    fn oversize_script_is_not_loaded() {
        let mut r = Runtime::with_limits(Limits {
            max_script_bytes: 20,
            ..Limits::default()
        });
        let notices = r.load("a", "alias one { echo this is too long }");
        assert_eq!(notices.len(), 1);
        assert!(!r.has_alias("one"));
    }

    #[test]
    fn non_ascii_alias_names_are_skipped_with_a_notice() {
        let mut r = Runtime::new();
        let notices = r.load("a", "alias h\u{e9}llo { echo x }\nalias ok { echo y }\n");
        assert_eq!(notices.len(), 1);
        assert!(r.has_alias("ok"));
        assert!(!r.has_alias("h\u{e9}llo"));
    }

    #[test]
    fn events_and_menus_in_a_script_do_not_block_loading() {
        let mut r = Runtime::new();
        let text = "on *:TEXT:hello*:#:echo got it\nmenu channel {\n  Hi:/echo hi\n}\nalias ok { echo y }\n";
        assert!(r.load("a", text).is_empty());
        assert_eq!(echoes(&run(&mut r, "ok", &[])), ["y"]);
    }

    // ---- safety

    #[test]
    fn hostile_parameter_text_is_data_never_commands() {
        let mut r = rt("alias say { echo $1- }\nalias set2 { set %v $1- | echo %v }\n");
        let hostile = "| echo pwned { } $ % [ ] $1 %v $me ;x";
        let res = run(&mut r, "say", &[hostile]);
        assert_eq!(echoes(&res), [hostile]);
        assert!(res.notices.is_empty());
        let res = run(&mut r, "set2", &[hostile]);
        assert_eq!(echoes(&res), [hostile]);
        // The value is also not reparsed when read back into a condition.
        let mut r = rt("alias c { if ($1 == yes) echo y | else echo n }");
        assert_eq!(echoes(&run(&mut r, "c", &["yes || 1 == 1"])), ["n"]);
        assert_eq!(echoes(&run(&mut r, "c", &["x == x"])), ["n"]);
    }

    #[test]
    fn echo_lines_never_contain_cr_lf_or_nul() {
        let mut r = rt("alias e { echo $1 | set %v $1 | echo %v $1- }");
        let res = run(&mut r, "e", &["a\r\nb\0c"]);
        assert_eq!(res.effects.len(), 2);
        for e in &res.effects {
            let s = e.text();
            assert!(!s.contains(['\r', '\n', '\0']), "{s:?}");
        }
    }

    #[test]
    fn notices_do_not_carry_script_values() {
        let mut r = rt("alias secret { set %k hunter2 | frob %k }");
        let res = r.run_alias("secret", &[]);
        assert_eq!(res.notices.len(), 1);
        assert!(!res.notices[0].contains("hunter2"));
    }

    #[test]
    fn on_text_event_and_context_parameters() {
        let mut r = rt("on *:TEXT:*hello*:#lobby:{ echo -a <$nick:$chan> $1- }");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#lobby".into(),
            text: "say hello everyone".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "<Alice:#lobby> say hello everyone".into(),
            }]
        );
        assert_eq!(res.halt_status, HaltStatus::None);
        assert_eq!(res.halt_status.code(), 0);

        // Non-matching target
        let ev_wrong_chan = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#other".into(),
            text: "say hello everyone".into(),
        };
        assert!(r.dispatch_event(&ev_wrong_chan, None).effects.is_empty());

        // Non-matching text
        let ev_wrong_text = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#lobby".into(),
            text: "goodbye".into(),
        };
        assert!(r.dispatch_event(&ev_wrong_text, None).effects.is_empty());
    }

    #[test]
    fn on_text_query_and_channel_wildcards() {
        let mut r = rt("on *:TEXT:*:?:{ echo -s query from $nick: $1- }\non *:TEXT:*:*:{ echo -a Text: $1- }");
        let ev_pm = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Bob".into(),
            target: "ferric".into(),
            text: "secret message".into(),
        };
        let res_pm = r.dispatch_event(&ev_pm, None);
        assert_eq!(res_pm.effects.len(), 2);
        assert_eq!(
            res_pm.effects[0],
            Effect::EchoTarget {
                target: EchoTarget::Status,
                text: "query from Bob: secret message".into(),
            }
        );
        assert_eq!(
            res_pm.effects[1],
            Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Text: secret message".into(),
            }
        );
    }

    #[test]
    fn on_join_event_and_context() {
        let mut r = rt("on *:JOIN:#ferric:{ echo -a $nick joined $chan }");
        let ev = ScriptEvent::Join { network: String::new(), me: String::new(), address: None,
            nick: "Bob".into(),
            channel: "#ferric".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Bob joined #ferric".into(),
            }]
        );
    }

    #[test]
    fn on_part_event_and_reason() {
        let mut r = rt("on *:PART:#ferric:{ echo -a $nick parted $chan : $1- }");
        let ev = ScriptEvent::Part { network: String::new(), me: String::new(), address: None,
            nick: "Charlie".into(),
            channel: "#ferric".into(),
            message: Some("leaving now".into()),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Charlie parted #ferric : leaving now".into(),
            }]
        );
    }

    #[test]
    fn text_identifiers_batch_one() {
        let mut r = rt("alias t { echo $len(hello) $left(goodbye,4) $right(othello,5) $mid(othello,3,4) $upper(hello) $lower(HELLO) $str(ho,3) }");
        let res = run(&mut r, "t", &[]);
        assert_eq!(
            res.effects,
            vec![Effect::Echo("5 good hello hell HELLO hello hohoho".into())]
        );
    }

    #[test]
    fn token_and_replace_identifiers_batch_one() {
        let mut r = rt("alias t { echo $replace(abcdefg,CD,xyz) $gettok(a.b.c.d.e,3,46) $gettok(a.b.c.d.e,2-4,46) $numtok(a.b.c,46) }");
        let res = run(&mut r, "t", &[]);
        assert_eq!(
            res.effects,
            vec![Effect::Echo("abxyzefg c b.c.d 3".into())]
        );
    }

    #[test]
    fn bad_text_identifier_call_stops_the_run() {
        let mut r = rt("alias t { echo before | echo $len(a,b) | echo after }");
        let res = run(&mut r, "t", &[]);
        assert!(res.effects.contains(&Effect::Echo("before".into())));
        assert!(!res.effects.contains(&Effect::Echo("after".into())));
    }

    #[test]
    fn time_identifiers_batch_three() {
        let mut r = rt("alias t { echo $time }\nalias c { echo $ctime }\nalias f { echo $time(HH:nn) }\nalias bad { echo before | echo $ctime(3rd August 1987) | echo after }\n");
        let res = run(&mut r, "t", &[]);
        let Some(Effect::Echo(s)) = res.effects.first() else {
            panic!("no echo: {:?}", res.notices)
        };
        assert_eq!(s.len(), 8);
        assert_eq!(s.as_bytes()[2], b':');
        let res = run(&mut r, "c", &[]);
        let Some(Effect::Echo(s)) = res.effects.first() else {
            panic!("no echo: {:?}", res.notices)
        };
        assert!(s.parse::<i64>().expect("seconds") > 1_700_000_000);
        let res = run(&mut r, "f", &[]);
        let Some(Effect::Echo(s)) = res.effects.first() else {
            panic!("no echo: {:?}", res.notices)
        };
        assert_eq!(s.len(), 5);
        let res = run(&mut r, "bad", &[]);
        assert!(res.effects.contains(&Effect::Echo("before".into())));
        assert!(!res.effects.contains(&Effect::Echo("after".into())));
    }

    #[test]
    fn host_identifiers_from_a_text_event() {
        let mut r = rt("on *:TEXT:*:#:{ echo $address $fulladdress $site $wildsite $target }\n");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(),
            nick: "Alice".into(),
            target: "#test".into(),
            text: "hi".into(),
            address: Some("alice@example.org".into()),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::Echo(
                "alice@example.org Alice!alice@example.org example.org *!*@example.org #test".into()
            )]
        );
    }

    #[test]
    fn me_and_network_identifiers() {
        let mut r = rt("on *:JOIN:#test:{ echo $me on $network }\n");
        let ev = ScriptEvent::Join {
            nick: "Alice".into(),
            channel: "#test".into(),
            address: None,
            network: "efnet".into(),
            me: "Bob".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(res.effects, vec![Effect::Echo("Bob on efnet".into())]);

        let mut r = rt("alias t { echo $me $network }\n");
        let ctx = ScriptContext::new().with_me("Bob").with_network("efnet");
        let res = r.run_alias_with_context("t", &[], &ctx);
        assert_eq!(res.effects, vec![Effect::Echo("Bob efnet".into())]);
        let res = r.run_alias("t", &[]);
        assert_eq!(res.effects, vec![Effect::Echo(" ".into())]);
    }

    #[test]
    fn host_identifiers_are_empty_without_an_address() {
        let mut r = rt("on *:QUIT:{ echo x$address$fulladdress$site$wildsite }\n");
        let ev = ScriptEvent::Quit { network: String::new(), me: String::new(),
            nick: "Dave".into(),
            message: None,
            address: None,
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(res.effects, vec![Effect::Echo("x".into())]);
    }

    #[test]
    fn address_with_arguments_is_refused_not_guessed() {
        let mut r = rt("alias t { echo before | echo $address(bob,1) | echo after }\n");
        let res = run(&mut r, "t", &[]);
        assert!(res.effects.contains(&Effect::Echo("before".into())));
        assert!(!res.effects.contains(&Effect::Echo("after".into())));
    }

    #[test]
    fn math_bit_and_token_identifiers_end_to_end() {
        let mut r = rt("alias t { echo $abs(-3) $round(3.14159,2) $gcd(12,18) $biton(0,3) $pi }\nalias k { echo $addtok(a.b,c,46) $istok(a.b,B,46) $deltok(a.b.c,2,46) }\n");
        let res = run(&mut r, "t", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("3 3.14 6 8 3.14159265358979323846".into())]);
        let res = run(&mut r, "k", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("a.b.c 1 a.c".into())]);
    }

    #[test]
    fn regex_identifiers_end_to_end() {
        let mut r = rt("alias c { echo $regex(hello world,/o/g) $regml(1) $regml(2).pos }\nalias s { echo $regsub(a1b2,/\\d/g,X,%r) %r }\nalias e { echo $regsubex(n,abc,/(b)/,\\t!,%x) %x }\n");
        let res = run(&mut r, "c", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("2 o 8".into())]);
        let res = run(&mut r, "s", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("2 aXbX".into())]);
        let res = run(&mut r, "e", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("1 ab!c".into())]);
    }

    #[test]
    fn hash_tables_persist_across_runs() {
        let mut r = rt("alias put { hadd -m10 t $1 $2 }\nalias get { echo $hget(t,$1) }\nalias count { echo $hget(t,0).item }\nalias bad { hadd nope a b | echo after }\n");
        let res = run(&mut r, "put", &["k", "v"]);
        assert!(res.effects.is_empty());
        let res = run(&mut r, "get", &["k"]);
        assert_eq!(res.effects, vec![Effect::Echo("v".into())]);
        let res = run(&mut r, "count", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("1".into())]);
        let res = run(&mut r, "bad", &[]);
        assert!(res.effects.is_empty());
    }

    #[test]
    fn string_identifiers_end_to_end() {
        let mut r = rt("alias t { echo $count(hello,l) $qt(x) $ord(26) $isnum(5,,1,10) $remove(abcdefg,cd) }\n");
        let res = run(&mut r, "t", &[]);
        assert_eq!(res.effects, vec![Effect::Echo("2 \"x\" 26th 1 abefg".into())]);
    }

    #[test]
    fn iif_picks_one_branch() {
        let mut r = rt("alias t { echo a$iif(1 == 1,yes,no)b | echo a$iif(1 == 2,yes,no)b | echo a$iif(1 == 2,yes)b }");
        let res = run(&mut r, "t", &[]);
        assert_eq!(
            res.effects,
            vec![
                Effect::Echo("ayesb".into()),
                Effect::Echo("anob".into()),
                Effect::Echo("ab".into()),
            ]
        );
    }

    #[test]
    fn rand_gives_numbers_and_letters_in_range() {
        let mut r = rt("alias n { echo $rand(1,6) }\nalias l { echo $rand(a,c) }\nalias bad { echo $rand(5,1) | echo after }\n");
        for _ in 0..100 {
            let res = run(&mut r, "n", &[]);
            let Effect::Echo(s) = &res.effects[0] else { panic!("no echo") };
            let k: i64 = s.parse().expect("a number");
            assert!((1..=6).contains(&k));
            let res = run(&mut r, "l", &[]);
            let Some(Effect::Echo(s)) = res.effects.first() else {
                panic!("no echo: {:?}", res.notices)
            };
            assert!(["a", "b", "c"].contains(&s.as_str()));
        }
        let res = run(&mut r, "bad", &[]);
        assert!(res.effects.is_empty());
    }

    #[test]
    fn on_nick_event_has_old_and_new_nick() {
        let mut r = rt("on *:NICK:{ echo -s $nick is now $newnick }");
        let ev = ScriptEvent::Nick { network: String::new(), me: String::new(), address: None,
            nick: "Dave".into(),
            new_nick: "Dave2".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Status,
                text: "Dave is now Dave2".into(),
            }]
        );
    }

    #[test]
    fn on_quit_event_and_reason() {
        let mut r = rt("on *:QUIT:{ echo -s $nick quit: $1- }");
        let ev = ScriptEvent::Quit { network: String::new(), me: String::new(), address: None,
            nick: "Dave".into(),
            message: Some("Connection reset".into()),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Status,
                text: "Dave quit: Connection reset".into(),
            }]
        );
    }

    #[test]
    fn a_snicks_value_longer_than_the_buffer_stops_the_run() {
        let mut r = rt("alias s { echo -a $snicks }");
        let long = "a".repeat(0x2865);
        let res = r.run_alias_with_context("s", &[], &ScriptContext::default().with_snicks(long));
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("$snicks is too long"));
        let res = r.run_alias_with_context("s", &[], &ScriptContext::default().with_snicks("a".repeat(0x2864)));
        assert_eq!(res.effects.len(), 1);
    }

    #[test]
    fn snicks_context_evaluation() {
        let mut r = rt("alias checksnic { echo -a Selected: $snicks }\non *:TEXT:*who*:#:{ echo -a Who: $snicks }");
        let res_alias = r.run_alias_with_context(
            "checksnic",
            &[],
            &ScriptContext::default().with_snicks("Alice,Bob"),
        );
        assert_eq!(
            res_alias.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Selected: Alice,Bob".into(),
            }]
        );

        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#test".into(),
            text: "who is here".into(),
        };
        let res_ev = r.dispatch_event(&ev, Some("Eve,Mallory"));
        assert_eq!(
            res_ev.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Who: Eve,Mallory".into(),
            }]
        );
    }

    #[test]
    fn halt_does_not_stop_a_handler_of_another_script() {
        let mut r = Runtime::new();
        r.load("ordB.mrc", "on *:TEXT:haltX*:#:{\n  echo -a B halts\n  halt\n}");
        r.load("ordA.mrc", "on *:TEXT:haltX*:#:{\n  echo -a A runs\n}");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#test".into(),
            text: "haltX go".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![
                Effect::EchoTarget { target: EchoTarget::Active, text: "B halts".into() },
                Effect::EchoTarget { target: EchoTarget::Active, text: "A runs".into() },
            ]
        );
    }

    #[test]
    fn scripts_run_in_the_order_they_were_loaded() {
        let mut r = Runtime::new();
        r.load("zeta.mrc", "on *:TEXT:order*:#:{ echo -a zeta }");
        r.load("alpha.mrc", "on *:TEXT:order*:#:{ echo -a alpha }");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#test".into(),
            text: "order go".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![
                Effect::EchoTarget { target: EchoTarget::Active, text: "zeta".into() },
                Effect::EchoTarget { target: EchoTarget::Active, text: "alpha".into() },
            ]
        );
    }

    #[test]
    fn halt_status_2_stops_later_matching_handlers() {
        let mut r = rt("on *:TEXT:*:#:{\n  echo -a Handler 1\n  halt\n}\non *:TEXT:*:#:{\n  echo -a Handler 2\n}");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#test".into(),
            text: "ping".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "Handler 1".into(),
            }]
        );
        assert_eq!(res.halt_status, HaltStatus::Halt);
        assert_eq!(res.halt_status.code(), 2);
        assert!(res.halt_status.is_halted());
    }

    #[test]
    fn haltdef_status_1_continues_and_allows_later_handlers() {
        let mut r = rt("on *:TEXT:*:#:{\n  echo -a Before haltdef\n  haltdef\n  echo -a After haltdef\n}\non *:TEXT:*:#:{\n  echo -a Handler 2\n}");
        let ev = ScriptEvent::Text { network: String::new(), me: String::new(), address: None,
            nick: "Alice".into(),
            target: "#test".into(),
            text: "ping".into(),
        };
        let res = r.dispatch_event(&ev, None);
        assert_eq!(
            res.effects,
            vec![
                Effect::EchoTarget {
                    target: EchoTarget::Active,
                    text: "Before haltdef".into(),
                },
                Effect::EchoTarget {
                    target: EchoTarget::Active,
                    text: "After haltdef".into(),
                },
                Effect::EchoTarget {
                    target: EchoTarget::Active,
                    text: "Handler 2".into(),
                },
            ]
        );
        assert_eq!(res.halt_status, HaltStatus::HaltDef);
        assert_eq!(res.halt_status.code(), 1);
        assert!(res.halt_status.is_halted());
    }

    #[test]
    fn echo_switches_default_active_status() {
        let mut r = rt("alias test_echoes {\n  echo normal text\n  echo -a active text\n  echo -s status text\n}");
        let res = run(&mut r, "test_echoes", &[]);
        assert_eq!(
            res.effects,
            vec![
                Effect::Echo("normal text".into()),
                Effect::EchoTarget {
                    target: EchoTarget::Active,
                    text: "active text".into(),
                },
                Effect::EchoTarget {
                    target: EchoTarget::Status,
                    text: "status text".into(),
                },
            ]
        );
        assert_eq!(res.effects[0].target(), EchoTarget::Default);
        assert_eq!(res.effects[1].target(), EchoTarget::Active);
        assert_eq!(res.effects[2].target(), EchoTarget::Status);
    }
}

#[cfg(test)]
mod irc_command_tests {
    use super::*;
    use super::super::irc_effects::{BroadcastKind, BroadcastScope};

    fn run_text(script: &str, name: &str, args: &[&str]) -> RunResult {
        let mut r = Runtime::new();
        assert!(r.load("t.mrc", script).is_empty());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        r.run_alias(name, &args)
    }

    fn irc(res: &RunResult) -> Vec<IrcEffect> {
        res.effects
            .iter()
            .filter_map(|e| match e {
                Effect::Irc(i) => Some(i.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn messages_carry_target_and_text() {
        let res = run_text(
            "alias a { msg $1 hello $2- | notice $1 n | describe #c waves }",
            "a",
            &["alice", "big", "world"],
        );
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            irc(&res),
            vec![
                IrcEffect::Msg { target: "alice".into(), text: "hello big world".into() },
                IrcEffect::Notice { target: "alice".into(), text: "n".into() },
                IrcEffect::Action { target: "#c".into(), text: "waves".into() },
            ]
        );
    }

    #[test]
    fn commands_build_their_lines() {
        let res = run_text(
            "alias a { join #c key | part #c bye now | nick newme | mode #c +o alice | kick #c bob spam | whois alice | raw PRIVMSG #c :hi }",
            "a",
            &[],
        );
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        let lines: Vec<String> = irc(&res).iter().filter_map(IrcEffect::line).collect();
        assert_eq!(
            lines,
            [
                "JOIN #c key",
                "PART #c :bye now",
                "NICK newme",
                "MODE #c +o alice",
                "KICK #c bob :spam",
                "WHOIS alice",
                "PRIVMSG #c :hi",
            ]
        );
    }

    #[test]
    fn refused_fields_stop_the_run_with_no_effect() {
        for script in [
            "alias a { raw PASS secret }",
            "alias a { raw OPER name pw }",
            "alias a { msg a,,b hi }",
            "alias a { msg alice }",
            "alias a { query #chan }",
            "alias a { join alice }",
            "alias a { nick two words }",
            "alias a { kick #c }",
            "alias a { mode #c }",
        ] {
            let res = run_text(script, "a", &[]);
            assert!(irc(&res).is_empty(), "{script}");
            assert_eq!(res.notices.len(), 1, "{script}: {:?}", res.notices);
        }
    }

    #[test]
    fn query_opens_a_window_and_may_send_text() {
        let res = run_text("alias a { query alice | query bob hi there }", "a", &[]);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            irc(&res),
            vec![
                IrcEffect::Query { nick: "alice".into(), text: None },
                IrcEffect::Query { nick: "bob".into(), text: Some("hi there".into()) },
            ]
        );
        // A query opens a window; it is not a line for the server.
        assert!(irc(&res).iter().all(|e| e.line().is_none()));
    }

    #[test]
    fn me_without_a_target_is_silent() {
        let res = run_text("alias a { me waves }", "a", &[]);
        assert!(irc(&res).is_empty());
        assert!(res.notices.is_empty(), "{:?}", res.notices);
    }

    #[test]
    fn mode_lines_keep_matching_letters_and_parameters() {
        let mode = |text: &str| irc(&run_text(&format!("alias a {{ mode {text} }}"), "a", &[]));
        let set = |modes: &str| vec![IrcEffect::Mode { target: "#a".into(), modes: modes.into() }];
        assert_eq!(mode("#a +oo alice"), set("+o alice"));
        assert_eq!(mode("#a +o alice bob"), set("+o alice"));
        assert_eq!(mode("#a +ov alice bob"), set("+ov alice bob"));
        assert_eq!(mode("#a +ob alice"), set("+ob alice"));
        assert_eq!(mode("#a +o"), set("+o"));
    }

    #[test]
    fn outbound_lines_are_limited_per_trigger() {
        let mut r = Runtime::new();
        r.load("t.mrc", "alias a { set %i 0 | while (%i < 60) { inc %i | msg alice x } }");
        let res = r.run_alias("a", &[]);
        assert_eq!(irc(&res).len(), 50);
        assert_eq!(res.notices.len(), 1);
        assert!(res.notices[0].contains("50 IRC lines per trigger"));
    }

    #[test]
    fn the_remaining_commands_build_their_lines() {
        let res = run_text(
            "alias a { away gone | away | quit bye | invite bob #c | topic #c new topic | list #c | links | userhost alice | mnick newme | tnick temp | uwho alice | ban #c *!*@host | hop #c later }",
            "a",
            &[],
        );
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        let lines: Vec<String> = irc(&res).iter().filter_map(IrcEffect::line).collect();
        assert_eq!(
            lines,
            [
                "AWAY :gone",
                "AWAY",
                "QUIT :bye",
                "INVITE bob #c",
                "TOPIC #c :new topic",
                "LIST #c",
                "LINKS",
                "USERHOST alice",
                "NICK newme",
                "NICK temp",
                "WHOIS alice",
                "MODE #c +b *!*@host",
                "PART #c :later",
                "JOIN #c",
            ]
        );
    }

    #[test]
    fn ctcpreply_frames_the_reply_as_a_notice() {
        let res = run_text("alias a { ctcpreply alice VERSION 1.0 | ctcpreply bob PING }", "a", &[]);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            irc(&res),
            vec![
                IrcEffect::Notice { target: "alice".into(), text: "\u{1}VERSION 1.0\u{1}".into() },
                IrcEffect::Notice { target: "bob".into(), text: "\u{1}PING\u{1}".into() },
            ]
        );
    }

    #[test]
    fn broadcasts_keep_their_scope() {
        let res = run_text("alias a { amsg hi | ame waves | qmsg yo | partall bye | partall }", "a", &[]);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            irc(&res),
            vec![
                IrcEffect::Broadcast { scope: BroadcastScope::ChannelsEveryNetwork, kind: BroadcastKind::Msg, text: Some("hi".into()) },
                IrcEffect::Broadcast { scope: BroadcastScope::ChannelsEveryNetwork, kind: BroadcastKind::Action, text: Some("waves".into()) },
                IrcEffect::Broadcast { scope: BroadcastScope::Queries, kind: BroadcastKind::Msg, text: Some("yo".into()) },
                IrcEffect::Broadcast { scope: BroadcastScope::Channels, kind: BroadcastKind::Part, text: Some("bye".into()) },
                IrcEffect::Broadcast { scope: BroadcastScope::Channels, kind: BroadcastKind::Part, text: None },
            ]
        );
    }

    #[test]
    fn say_and_omsg_use_the_context_channel() {
        let mut r = Runtime::new();
        r.load("t.mrc", "alias a { say hi | omsg ops only | onotice #d note }");
        let ctx = ScriptContext::new().with_me("me").with_network("net");
        let ctx = ScriptContext { target: Some("#c".into()), ..ctx };
        let res = r.run_alias_with_context("a", &[], &ctx);
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            irc(&res),
            vec![
                IrcEffect::Msg { target: "#c".into(), text: "hi".into() },
                IrcEffect::Msg { target: "@#c".into(), text: "ops only".into() },
                IrcEffect::Notice { target: "@#d".into(), text: "note".into() },
            ]
        );
    }

    #[test]
    fn hop_without_a_channel_needs_the_context() {
        let res = run_text("alias a { hop }", "a", &[]);
        assert!(irc(&res).is_empty());
        assert_eq!(res.notices.len(), 1);
    }

    #[test]
    fn unsupported_and_unsafe_forms_stop_the_run() {
        for script in [
            "alias a { ajinvite on }",
            "alias a { anick other }",
            "alias a { queryrn alice bob }",
            "alias a { pvoice 5 #c alice }",
            "alias a { ban #c alice }",
            "alias a { ban #c *!*@host kicked }",
            "alias a { list -MIN 5 }",
            "alias a { list #a,#b }",
            "alias a { links -n }",
            "alias a { invite bob }",
            "alias a { topic }",
            "alias a { say hi }",
            "alias a { ctcpreply alice }",
            "alias a { userhost a b }",
            "alias a { hop -c #c }",
        ] {
            let res = run_text(script, "a", &[]);
            assert!(irc(&res).is_empty(), "{script}");
            assert_eq!(res.notices.len(), 1, "{script}: {:?}", res.notices);
        }
    }

    #[test]
    fn raw_line_past_the_limit_is_refused() {
        let long = "x".repeat(600);
        let res = run_text(&format!("alias a {{ raw PRIVMSG a :{long} }}"), "a", &[]);
        assert!(irc(&res).is_empty());
        assert_eq!(res.notices.len(), 1);
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    /// A runtime whose clock the test moves.
    fn clocked(start: Instant) -> (Runtime, Arc<Mutex<Instant>>) {
        let now = Arc::new(Mutex::new(start));
        let mut r = Runtime::new();
        let c = Arc::clone(&now);
        r.clock = Box::new(move || *c.lock().unwrap());
        (r, now)
    }

    fn at(now: &Arc<Mutex<Instant>>, t: Instant) {
        *now.lock().unwrap() = t;
    }

    fn nobody(_: &str) -> Option<String> {
        None
    }

    fn ctx() -> ScriptContext {
        ScriptContext::new().with_network("net").with_me("me").with_nick("alice")
    }

    fn texts(runs: &[TimerRun]) -> Vec<String> {
        runs.iter()
            .flat_map(|run| run.result.effects.iter().map(|e| e.text().to_string()))
            .collect()
    }

    #[test]
    fn a_timer_that_switches_itself_off_does_not_come_back() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        assert!(r.load("t.mrc", "alias a { timer t1 3 10 timer t1 off }").is_empty());
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(10));
        assert_eq!(r.fire_timers(&nobody).len(), 1);
        assert_eq!(r.timers.len(), 0);
        at(&now, start + Duration::from_secs(20));
        assert!(r.fire_timers(&nobody).is_empty());
    }

    #[test]
    fn the_e_flag_makes_an_existing_timer_due_now() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        assert!(r
            .load("t.mrc", "alias a { timer t1 3 10 echo tick }\nalias e { timer -e t1 3 10 echo tock }")
            .is_empty());
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(5));
        r.run_alias_with_context("e", &[], &ctx());
        // Due at once, not after the next period of the timer it replaces.
        assert_eq!(texts(&r.fire_timers(&nobody)), ["tock"]);
        at(&now, start + Duration::from_secs(15));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["tock"]);
    }

    #[test]
    fn a_timer_fires_each_period_for_its_repetitions() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        assert!(r.load("t.mrc", "alias a { timer t1 3 10 echo tick }").is_empty());
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(9));
        assert!(r.fire_timers(&nobody).is_empty());
        at(&now, start + Duration::from_secs(10));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["tick"]);
        at(&now, start + Duration::from_secs(20));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["tick"]);
        at(&now, start + Duration::from_secs(30));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["tick"]);
        at(&now, start + Duration::from_secs(40));
        assert!(r.fire_timers(&nobody).is_empty());
        assert_eq!(r.timers.len(), 0);
    }

    #[test]
    fn zero_repetitions_repeat_until_off_and_the_identifier_reads_null() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        r.load(
            "t.mrc",
            "alias a { timer t1 0 5 echo x }\nalias off { timer t1 off }\nalias q { echo $timer(t1) }\nalias n { timer9 0 5 echo z }\nalias noff { timer9 off }",
        );
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(5));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["x"]);
        at(&now, start + Duration::from_secs(10));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["x"]);
        r.run_alias_with_context("off", &[], &ctx());
        at(&now, start + Duration::from_secs(15));
        assert!(r.fire_timers(&nobody).is_empty());
        let shown = r.run_alias_with_context("q", &[], &ctx());
        assert_eq!(shown.effects.iter().map(|e| e.text().to_string()).collect::<Vec<_>>(), [""]);

        // The numbered form: /timer9 and /timer9 off.
        r.run_alias_with_context("n", &[], &ctx());
        assert_eq!(r.timers.len(), 1);
        r.run_alias_with_context("noff", &[], &ctx());
        assert_eq!(r.timers.len(), 0);
    }

    #[test]
    fn online_timers_stop_on_disconnect_and_offline_ones_do_not() {
        let start = Instant::now();
        let (mut r, _now) = clocked(start);
        r.load("t.mrc", "alias a { timer on1 0 5 echo a | timer -o off1 0 5 echo b }");
        r.run_alias_with_context("a", &[], &ctx());
        r.remove_network_timers("net");
        assert!(!r.timers.contains("on1"));
        assert!(r.timers.contains("off1"));
    }

    #[test]
    fn a_timer_that_makes_its_own_name_keeps_one_timer() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        r.load("t.mrc", "alias a { timer a 0 1 timer a 0 1 echo x }");
        r.run_alias_with_context("a", &[], &ctx());
        assert_eq!(r.timers.len(), 1);
        at(&now, start + Duration::from_secs(1));
        r.fire_timers(&nobody);
        assert_eq!(r.timers.len(), 1);
    }

    #[test]
    fn the_sixty_fifth_timer_is_refused_with_one_notice() {
        let (mut r, _now) = clocked(Instant::now());
        r.load("t.mrc", "alias a { timer n$1 0 1 echo x }");
        for i in 1..=64 {
            assert!(r.run_alias("a", &[i.to_string()]).notices.is_empty());
        }
        let res = r.run_alias("a", &["65".to_string()]);
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
        assert!(res.notices[0].contains("too many timers"));
        assert_eq!(r.timers.len(), 64);
    }

    #[test]
    fn dollar_bang_reads_the_live_nick_and_the_rest_the_creation_value() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        r.load("t.mrc", "alias a { timer t 1 1 echo $me $!me }");
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(1));
        let live = |net: &str| (net == "net").then(|| "new".to_string());
        assert_eq!(texts(&r.fire_timers(&live)), ["me new"]);
    }

    #[test]
    fn a_body_over_its_budget_is_switched_off_once() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        r.load("t.mrc", "alias a { timer hog 1 1 while (1) { } }");
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_secs(1));
        let runs = r.fire_timers(&nobody);
        let notices: Vec<&String> = runs.iter().flat_map(|run| run.result.notices.iter()).collect();
        assert_eq!(notices.iter().filter(|n| n.contains("switched off")).count(), 1, "{notices:?}");
        assert_eq!(r.timers.len(), 0);
    }

    #[test]
    fn milliseconds_below_the_floor_run_at_the_floor() {
        let start = Instant::now();
        let (mut r, now) = clocked(start);
        r.load("t.mrc", "alias a { timer -m f 2 1 echo f }");
        r.run_alias_with_context("a", &[], &ctx());
        at(&now, start + Duration::from_millis(50));
        assert!(r.fire_timers(&nobody).is_empty());
        at(&now, start + Duration::from_millis(100));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["f"]);
        at(&now, start + Duration::from_millis(200));
        assert_eq!(texts(&r.fire_timers(&nobody)), ["f"]);
        at(&now, start + Duration::from_millis(300));
        assert!(r.fire_timers(&nobody).is_empty());
    }

    #[test]
    fn reloading_a_script_removes_its_timers() {
        let (mut r, _now) = clocked(Instant::now());
        r.load("s.mrc", "alias a { timer t 0 1 echo x }");
        r.run_alias("a", &[]);
        assert_eq!(r.timers.len(), 1);
        r.load("s.mrc", "alias a { echo y }");
        assert_eq!(r.timers.len(), 0);
    }

    #[test]
    fn switches_and_forms_that_are_not_supported_are_refused() {
        for script in [
            "alias a { timer -c t 1 1 echo x }",
            "alias a { timer t 14:30 1 1 echo x }",
            "alias a { timer t x 1 echo y }",
            "alias a { timer t 1 1 }",
            "alias a { timershow 0 10 echo x }",
        ] {
            let mut r = Runtime::new();
            r.load("t.mrc", script);
            let res = r.run_alias("a", &[]);
            assert_eq!(res.notices.len(), 1, "{script}: {:?}", res.notices);
            assert_eq!(r.timers.len(), 0, "{script}");
        }
    }

    #[test]
    fn timers_lists_as_echo_and_timers_off_clears_them() {
        let (mut r, _now) = clocked(Instant::now());
        r.load(
            "t.mrc",
            "alias a { timer t 0 5 echo x | timer u 3 5 echo y }\nalias l { timers }\nalias off { timers off }",
        );
        r.run_alias("a", &[]);
        let listed = r.run_alias("l", &[]);
        assert_eq!(listed.effects.len(), 2);
        r.run_alias("off", &[]);
        assert_eq!(r.timers.len(), 0);
    }
}

#[cfg(test)]
mod static_trace_tests {
    use super::*;

    fn echoed(script: &str, name: &str) -> Vec<String> {
        let mut r = Runtime::new();
        assert!(r.load("t.mrc", script).is_empty());
        r.run_alias(name, &[])
            .effects
            .iter()
            .map(|e| e.text().to_string())
            .collect()
    }

    #[test]
    fn a_bang_in_front_of_a_command_skips_the_alias_of_that_name() {
        // An alias named `echo` shadows the command: `echo` runs the alias, `!echo` the command.
        let script = "alias echo { set %ran 1 }
alias plain { echo built-in }
alias bang { !echo built-in }";
        assert_eq!(echoed(script, "plain"), Vec::<String>::new());
        assert_eq!(echoed(script, "bang"), ["built-in"]);
    }

    #[test]
    fn a_bare_dollar_plus_joins_the_words_on_its_sides() {
        assert_eq!(echoed("alias a { echo Hel $+ lo th $+ ere $+ ! }", "a"), ["Hello there!"]);
    }
}

#[cfg(test)]
mod trace_u2_u7_u10_u13_tests {
    use super::*;
    use std::time::Duration;

    fn echoed(script: &str, name: &str) -> Vec<String> {
        let mut r = Runtime::new();
        assert!(r.load("t.mrc", script).is_empty());
        r.run_alias(name, &[])
            .effects
            .iter()
            .map(|e| e.text().to_string())
            .collect()
    }

    #[test]
    fn the_ampersand_is_a_word_before_a_space_or_the_end() {
        assert!(wildcard_match("hello &", "hello bob"));
        assert!(!wildcard_match("hello &", "hello"));
        assert!(wildcard_match("& there", "hi there"));
        assert!(!wildcard_match("&", "a b"));
        // Without a space or the end after it, `&` is a plain character.
        assert!(wildcard_match("&x", "&x"));
        assert!(!wildcard_match("&x", "bob"));
    }

    #[test]
    fn the_question_mark_is_a_nick_target_not_null_or_at() {
        assert!(target_matches("?", "alice"));
        assert!(!target_matches("?", "#c"));
        assert!(!target_matches("?", ""));
        assert!(!target_matches("?", "@alice"));
    }

    #[test]
    fn the_equals_marker_is_not_special_yet() {
        // Not characterised yet: a plain wildcard, so only the literal `=` matches.
        assert!(target_matches("=", "="));
        assert!(!target_matches("=", "alice"));
    }

    #[test]
    fn the_first_of_two_aliases_with_one_name_runs() {
        assert_eq!(echoed("alias a { echo first }\nalias a { echo second }\n", "a"), ["first"]);
    }

    #[test]
    fn a_script_that_is_not_taken_reports_it_and_keeps_its_old_entries() {
        let mut r = Runtime::new();
        assert!(r.load_accepted("a.mrc", "alias hi { echo first }\n").0);
        let (accepted, notices) = r.load_accepted("a.mrc", "alias hi { echo second\n");
        assert!(!accepted, "{notices:?}");
        assert!(!notices.is_empty());
        let res = r.run_alias_with_context("hi", &[], &ScriptContext::default());
        assert_eq!(res.effects.iter().map(|e| e.text().to_string()).collect::<Vec<_>>(), ["first"]);
    }

    #[test]
    fn lifecycle_events_and_timers_of_a_mixed_case_script_follow_its_name() {
        let mut r = Runtime::new();
        r.load("MyScript.mrc", "on *:LOAD:{ echo loaded }\nalias t { timer t1 3 10 echo tick }\n");
        let ctx = ScriptContext::default();
        let res = r.dispatch_lifecycle("MyScript.mrc", "load", &ctx);
        assert_eq!(res.effects.iter().map(|e| e.text().to_string()).collect::<Vec<_>>(), ["loaded"]);
        r.run_alias_with_context("t", &[], &ctx);
        assert_eq!(r.timers.len(), 1);
        // Reloading under another case removes the timers of the old version.
        r.load("myscript.MRC", "on *:LOAD:{ echo loaded }\nalias t { echo x }\n");
        assert_eq!(r.timers.len(), 0);
    }

    #[test]
    fn script_names_are_not_case_sensitive() {
        let mut r = Runtime::new();
        let menu = "alias hi { echo hi }\nmenu status { Hi:/echo x }\n";
        r.load("Test.mrc", menu);
        r.load("test.mrc", menu);
        assert_eq!(r.menu_nodes("status", &ScriptContext::default(), &[]).len(), 1);
        assert!(r.unload("TEST.mrc"));
        assert!(r.menu_nodes("status", &ScriptContext::default(), &[]).is_empty());
        assert!(!r.unload("test.mrc"));
    }

    #[test]
    fn set_computes_only_from_an_operator_in_the_source() {
        assert_eq!(echoed("alias d { set %d 2026-11-21 | echo [%d] }\n", "d"), ["[2026-11-21]"]);
        assert_eq!(echoed("alias s { set %x 1 + 1 | echo [%x] }\n", "s"), ["[2]"]);
        assert_eq!(echoed("alias m { set %x 2 * 3 | echo [%x] }\n", "m"), ["[6]"]);
    }

    #[test]
    fn true_is_one_and_a_negative_zero_increment_is_zero() {
        assert_eq!(echoed("alias t { if ($true) echo yes }\n", "t"), ["yes"]);
        assert_eq!(echoed("alias n { %x = 0 | inc %x -0 | echo [%x] }\n", "n"), ["[0]"]);
    }

    #[test]
    fn a_signed_zero_calc_result_is_written_as_zero() {
        assert_eq!(echoed("alias z { echo [$calc(0 * -1)] }\n", "z"), ["[0]"]);
    }

    #[test]
    fn false_is_false_in_a_condition() {
        assert!(echoed("alias f { if ($false) echo yes }\n", "f").is_empty());
        assert_eq!(echoed("alias g { if (!$false) echo yes }\n", "g"), ["yes"]);
        assert_eq!(echoed("alias h { echo [$false] }\n", "h"), ["[0]"]);
    }

    #[test]
    fn the_at_marker_needs_a_target_that_begins_with_at() {
        assert!(target_matches("@", "@#c"));
        assert!(!target_matches("@", "#c"));
    }

    #[test]
    fn a_direct_self_call_takes_the_built_in_branch() {
        // An alias named like a built-in command calls the command from its own body.
        assert_eq!(echoed("alias echo { echo a | echo b }", "echo"), ["a", "b"]);
    }

    #[test]
    fn iswm_is_a_wildcard_match_and_iswmcs_is_case_exact() {
        assert_eq!(echoed("alias a { if (abc.txt iswm *.txt) echo yes else echo no }", "a"), ["yes"]);
        assert_eq!(echoed("alias a { if (ABC iswm abc) echo yes else echo no }", "a"), ["yes"]);
        assert_eq!(echoed("alias a { if (ABC iswmcs abc) echo yes else echo no }", "a"), ["no"]);
    }

    #[test]
    fn set_computes_arithmetic_with_zero_for_division_by_zero() {
        assert_eq!(echoed("alias a { set %x 5/2 | echo %x }", "a"), ["2.5"]);
        assert_eq!(echoed("alias a { set %x 5/0 | echo %x }", "a"), ["0"]);
        assert_eq!(echoed("alias a { set %x 2+3*4 | echo %x }", "a"), ["14"]);
        // Plain numbers and other text stay as they were written.
        assert_eq!(echoed("alias a { set %x 007 | echo %x }", "a"), ["007"]);
        assert_eq!(echoed("alias a { set %x hello/world | echo %x }", "a"), ["hello/world"]);
    }

    #[test]
    fn an_interval_below_one_is_one_millisecond_and_the_first_run_waits_for_it() {
        let mut r = Runtime::new();
        r.load("t.mrc", "alias a { timer z 1 0 echo z }");
        r.run_alias("a", &[]);
        assert_eq!(r.timers.len(), 1);
        let due = r.timers.iter().next().map(|t| t.next_due).unwrap();
        // 1 ms is raised to the 100 ms floor; a second would be far later.
        assert!(due < Instant::now() + Duration::from_millis(500), "a 1 ms interval, not a second");
    }

    #[test]
    fn the_h_flag_and_negative_intervals_follow_the_unit_rules() {
        let mut r = Runtime::new();
        r.load("t.mrc", "alias a { timer -h z 1 2000 echo z }\nalias b { timer n 1 -5 echo n }");
        r.run_alias("a", &[]);
        let due = r.timers.iter().find(|t| t.name == "z").map(|t| t.next_due).unwrap();
        // -h: 2000 milliseconds, not 2000 seconds.
        assert!(due > Instant::now() + Duration::from_millis(1500));
        assert!(due < Instant::now() + Duration::from_millis(2500));
        r.run_alias("b", &[]);
        let due = r.timers.iter().find(|t| t.name == "n").map(|t| t.next_due).unwrap();
        assert!(due < Instant::now() + Duration::from_millis(500));
    }
}

