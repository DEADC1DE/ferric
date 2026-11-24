//! Data model produced by the .mrc parser.
//!
//! The parser never evaluates anything: every argument is kept as raw text so
//! a later interpreter stage can decide how to evaluate it.

/// Kind of a body statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatementKind {
    /// Any ordinary command (`echo`, `msg`, user alias call, ...).
    Command,
    If,
    ElseIf,
    Else,
    While,
    Return,
    Break,
    Continue,
    Goto,
    /// `:label`
    Label,
    Var,
    Set,
    Unset,
    Inc,
    Dec,
    /// A statement that starts with an identifier or variable (`$name`, `%var`).
    Eval,
    /// A bare `{ ... }` block that is not attached to a control statement.
    Block,
}

impl StatementKind {
    /// True for `if`, `elseif`, `else` and `while`.
    pub fn is_control(self) -> bool {
        matches!(
            self,
            StatementKind::If | StatementKind::ElseIf | StatementKind::Else | StatementKind::While
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StatementKind::Command => "command",
            StatementKind::If => "if",
            StatementKind::ElseIf => "elseif",
            StatementKind::Else => "else",
            StatementKind::While => "while",
            StatementKind::Return => "return",
            StatementKind::Break => "break",
            StatementKind::Continue => "continue",
            StatementKind::Goto => "goto",
            StatementKind::Label => "label",
            StatementKind::Var => "var",
            StatementKind::Set => "set",
            StatementKind::Unset => "unset",
            StatementKind::Inc => "inc",
            StatementKind::Dec => "dec",
            StatementKind::Eval => "eval",
            StatementKind::Block => "block",
        }
    }
}

/// One statement of a script body.
///
/// * `name` is the lowercased command or keyword (without `/` or `.` prefix).
///   For labels it is the label name, for `Eval` the leading `$`/`%` token.
/// * `args` is the raw argument text. For `if`/`elseif`/`while` it is the raw
///   condition (including its parentheses when present).
/// * `body` holds the statements of an attached `{ ... }` block or of an
///   inline form such as `if (x) command` (those children have `inline` set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub line: usize,
    pub kind: StatementKind,
    pub name: String,
    pub args: String,
    /// The command was written with a leading `.` (silent form).
    pub silent: bool,
    /// The statement was written on the same line as its parent control
    /// statement (`if (x) echo y`).
    pub inline: bool,
    pub body: Vec<Statement>,
}

impl Statement {
    pub(crate) fn new(line: usize, kind: StatementKind, name: String, args: String) -> Self {
        Statement {
            line,
            kind,
            name,
            args,
            silent: false,
            inline: false,
            body: Vec::new(),
        }
    }
}

/// `alias [-l] name ...`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    pub line: usize,
    pub end_line: usize,
    pub name: String,
    pub local: bool,
    pub body: Vec<Statement>,
}

/// `on <levels>:<EVENT>:<fields...>:<commands>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub line: usize,
    pub end_line: usize,
    /// Raw level specifiers, split on `,` (e.g. `*`, `1`, `!*`, `+5`).
    pub levels: Vec<String>,
    /// Lowercased event name.
    pub event: String,
    /// Header fields between the event name and the commands (match text,
    /// target, ...). Their number depends on the event.
    pub fields: Vec<String>,
    /// Shared, so running the event does not copy its statement tree.
    pub body: std::sync::Arc<Vec<Statement>>,
}

/// `raw <numeric>:<match>:<commands>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raw {
    pub line: usize,
    pub end_line: usize,
    pub numeric: String,
    pub match_text: String,
    pub body: Vec<Statement>,
}

/// `ctcp <levels>:<match>:<target>:<commands>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctcp {
    pub line: usize,
    pub end_line: usize,
    pub levels: Vec<String>,
    pub match_text: String,
    pub target: String,
    pub body: Vec<Statement>,
}

/// Kind of a menu entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemKind {
    /// A line consisting of `-`.
    Separator,
    /// An entry without commands whose children are the following deeper entries.
    Submenu,
    /// An entry with commands (inline or block).
    Action,
}

/// One menu entry. Nesting follows the leading `.` prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    pub line: usize,
    pub end_line: usize,
    /// Number of leading dots.
    pub level: usize,
    pub kind: MenuItemKind,
    pub label: String,
    pub body: Vec<Statement>,
    pub children: Vec<MenuItem>,
}

/// `menu <window[,window...]> { ... }`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub line: usize,
    pub end_line: usize,
    pub windows: Vec<String>,
    pub items: Vec<MenuItem>,
}

/// `dialog [-l] name { ... }`; only presence and name are recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub line: usize,
    pub end_line: usize,
    pub name: String,
    pub local: bool,
    /// Number of non-blank, non-comment content lines.
    pub content_lines: usize,
}

/// `#name on|off` ... `#name end`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub line: usize,
    /// `None` when the closing marker is missing.
    pub end_line: Option<usize>,
    pub name: String,
    pub enabled: bool,
    pub items: Vec<Item>,
}

/// Kind of an item that is not one of the structured item types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OtherKind {
    /// A command line at file level (as used by plain command files).
    TopLevelCommand,
    /// `[section]` header of an old-style ini script.
    IniSection,
    /// `n12=...` entry of an old-style ini script.
    IniEntry,
    /// A recognised item keyword with an unusable header.
    MalformedHeader,
    /// A `{ ... }` block at file level.
    StrayBlock,
}

impl OtherKind {
    pub fn as_str(self) -> &'static str {
        match self {
            OtherKind::TopLevelCommand => "top-level-command",
            OtherKind::IniSection => "ini-section",
            OtherKind::IniEntry => "ini-entry",
            OtherKind::MalformedHeader => "malformed-header",
            OtherKind::StrayBlock => "stray-block",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Other {
    pub line: usize,
    pub end_line: usize,
    pub kind: OtherKind,
    pub statements: Vec<Statement>,
}

/// A top-level item of a script file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Alias(Alias),
    Event(Event),
    Raw(Raw),
    Ctcp(Ctcp),
    Menu(Menu),
    Dialog(Dialog),
    Group(Group),
    Other(Other),
}

impl Item {
    /// First source line (1-based) of the item.
    pub fn line(&self) -> usize {
        match self {
            Item::Alias(i) => i.line,
            Item::Event(i) => i.line,
            Item::Raw(i) => i.line,
            Item::Ctcp(i) => i.line,
            Item::Menu(i) => i.line,
            Item::Dialog(i) => i.line,
            Item::Group(i) => i.line,
            Item::Other(i) => i.line,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Item::Alias(_) => "alias",
            Item::Event(_) => "event",
            Item::Raw(_) => "raw",
            Item::Ctcp(_) => "ctcp",
            Item::Menu(_) => "menu",
            Item::Dialog(_) => "dialog",
            Item::Group(_) => "group",
            Item::Other(_) => "other",
        }
    }
}

/// Why a construct could not be classified. Never carries source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UnclassifiedKind {
    UnknownTopLevel,
    UnbalancedClose,
    StrayOpenBrace,
    UnterminatedBlock,
    NestingTooDeep,
    MalformedAliasHeader,
    MalformedEventHeader,
    MalformedRawHeader,
    MalformedCtcpHeader,
    MalformedGroupMarker,
    NestedGroup,
    UnterminatedGroup,
    UnmatchedGroupEnd,
    UnterminatedMenu,
    UnterminatedDialog,
}

impl UnclassifiedKind {
    pub fn as_str(self) -> &'static str {
        match self {
            UnclassifiedKind::UnknownTopLevel => "unknown-top-level",
            UnclassifiedKind::UnbalancedClose => "unbalanced-close-brace",
            UnclassifiedKind::StrayOpenBrace => "stray-open-brace",
            UnclassifiedKind::UnterminatedBlock => "unterminated-block",
            UnclassifiedKind::NestingTooDeep => "nesting-too-deep",
            UnclassifiedKind::MalformedAliasHeader => "malformed-alias-header",
            UnclassifiedKind::MalformedEventHeader => "malformed-event-header",
            UnclassifiedKind::MalformedRawHeader => "malformed-raw-header",
            UnclassifiedKind::MalformedCtcpHeader => "malformed-ctcp-header",
            UnclassifiedKind::MalformedGroupMarker => "malformed-group-marker",
            UnclassifiedKind::NestedGroup => "nested-group",
            UnclassifiedKind::UnterminatedGroup => "unterminated-group",
            UnclassifiedKind::UnmatchedGroupEnd => "unmatched-group-end",
            UnclassifiedKind::UnterminatedMenu => "unterminated-menu",
            UnclassifiedKind::UnterminatedDialog => "unterminated-dialog",
        }
    }
}

/// A construct the parser could not classify: line number and kind only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unclassified {
    pub line: usize,
    pub kind: UnclassifiedKind,
}

/// Text encoding detected while decoding the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// Used when the bytes are not valid UTF-8.
    Windows1252,
    Utf16,
}

impl Encoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf-8",
            Encoding::Windows1252 => "windows-1252",
            Encoding::Utf16 => "utf-16",
        }
    }
}

/// Line level statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptStats {
    pub encoding: Encoding,
    pub physical_lines: usize,
    /// Logical lines holding code (after joining `$&` continuations and
    /// dropping blanks and comments).
    pub code_lines: usize,
    pub blank_lines: usize,
    /// Full-line `;` comments and `/* */` comment lines.
    pub comment_lines: usize,
    /// Statement-initial `;` comments that follow code (`{ ; note`).
    pub inline_comments: usize,
    /// Number of `$&` continuation joins.
    pub continuations: usize,
    /// `if`/`while` conditions written without parentheses (not split).
    pub unparenthesised_conditions: usize,
}

/// Parsed script file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub items: Vec<Item>,
    pub unclassified: Vec<Unclassified>,
    pub stats: ScriptStats,
}
