//! Interpreter for .mrc scripts, milestone M1: aliases, `echo`, variables,
//! `if`/`elseif`/`else`, `while` and `goto`.
//!
//! A pure library: no file, network or process access. [`Runtime::run_alias`]
//! returns its output as [`Effect`] values and notice text; the caller applies
//! them. IRC commands (`/msg`, `/join`, `/raw`, ...) return [`IrcEffect`] values
//! that the app sends through the connection. All limits live in [`Limits`].
//! Commands outside the list, `run` and `dll` included, are refused with one
//! notice and no effect.

mod eval;
mod events;
mod float_calc;
mod hash_tables;
mod irc_effects;
mod menus;
mod limits;
mod runtime;
mod string_funcs;
mod math_funcs;
mod regex_funcs;
mod text_funcs;
mod timers;
mod token_funcs;
mod time_funcs;
mod value;

pub use events::script_events;
pub use irc_effects::{
    channel_ok, name_ok, nick_ok, query_ok, raw_refusal, targets_ok, BroadcastKind, BroadcastScope,
    IrcEffect, MAX_LINE_BYTES,
};
pub use menus::{MenuAddress, MenuNode, NodeKind, WINDOW_KINDS};
pub use runtime::TimerRun;
pub use limits::Limits;
pub use runtime::{
    EchoTarget, Effect, HaltStatus, Notice, RunResult, Runtime, ScriptContext, ScriptEvent,
};
