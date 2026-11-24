//! Stage 1 of .mrc support: a parser for .mrc script files.
//!
//! The parser reads script text into a [`Script`] structure and never
//! executes or interprets anything: arguments, conditions and strings are
//! kept as raw text. [`Script::inventory`] summarises which commands,
//! identifiers and events a script uses (counts and names only).
//!
//! # Rules and sources
//!
//! The syntax handled here follows the published mrc scripting reference: aliases (`aliases.html`), remote
//! scripts and events (`remote.html`, `events.html`), raw events
//! (`raw_events.html`), CTCP events (`ctcp_events.html`), popups/menus
//! (`popups.html`), dialogs (`dialogs.html`), if/elseif/else and goto
//! (`if_then_else.html`), variables (`variables.html`) and identifiers
//! (`identifiers.html`), plus the full help file for groups
//! (`#name on|off` ... `#name end`, no nesting), `/while` with `/break` and
//! `/continue`, `$&` line continuation and the LOAD/START/CONNECT forms.
//!
//! Rules that go beyond what the help documents, chosen to be lenient:
//! * a statement starting with `;` (for example `{ ; note`) comments out the
//!   rest of the line;
//! * `|` inside the parentheses of an identifier call or an
//!   `if`/`elseif`/`while` condition is not a separator and `||` is the OR
//!   operator; quotes do not protect a `|`;
//! * `{` and `}` count as block delimiters when they stand alone between
//!   whitespace or line ends outside such parentheses;
//! * `/* ... */` comment lines are skipped (mentioned in `aliases.html`).
//!
//! # Robustness
//!
//! No input panics. Block nesting is capped at 32 levels and inline
//! `if (..) if (..) ...` chains at 16; deeper input is flattened and reported
//! as [`UnclassifiedKind::NestingTooDeep`].

mod ast;
mod body;
mod cond;
mod expr;
pub mod interp;
mod inventory;
mod lexer;
mod parser;

#[cfg(test)]
mod tests;

use std::fs;
use std::io;
use std::path::Path;

pub use ast::*;
pub use interp::{Effect, Limits, Notice, RunResult, Runtime};
pub use inventory::{is_builtin_command, is_builtin_identifier, IdentStat, Inventory};

/// Parses script bytes (UTF-8, UTF-16 with BOM, otherwise Windows-1252).
pub fn parse_bytes(bytes: &[u8]) -> Script {
    let (text, encoding) = lexer::decode(bytes);
    parse_decoded(&text, encoding)
}

/// Parses script text.
pub fn parse_str(text: &str) -> Script {
    parse_decoded(text, Encoding::Utf8)
}

/// Reads and parses a script file. Only I/O errors are reported; content
/// problems end up in [`Script::unclassified`].
pub fn parse_file<P: AsRef<Path>>(path: P) -> io::Result<Script> {
    let bytes = fs::read(path)?;
    Ok(parse_bytes(&bytes))
}

/// Reads a script file and returns its inventory.
pub fn inventory_file<P: AsRef<Path>>(path: P) -> io::Result<Inventory> {
    Ok(parse_file(path)?.inventory())
}

fn parse_decoded(text: &str, encoding: Encoding) -> Script {
    let scan = lexer::scan_lines(text);
    let physical_lines = scan.physical;
    let blank_lines = scan.blank;
    let comment_lines = scan.comments;
    let continuations = scan.continuations;
    let code_lines = scan.lines.len();
    let parsed = parser::parse_lines(scan);
    Script {
        items: parsed.items,
        unclassified: parsed.unclassified,
        stats: ScriptStats {
            encoding,
            physical_lines,
            code_lines,
            blank_lines,
            comment_lines,
            inline_comments: parsed.inline_comments,
            continuations,
            unparenthesised_conditions: parsed.unparenthesised,
        },
    }
}
