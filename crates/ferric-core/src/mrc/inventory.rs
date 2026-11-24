//! Inventory of the commands, identifiers and constructs a script uses.
//!
//! The inventory holds counts and names of commands, identifiers and events
//! only. Variable names, strings, arguments, hosts, nicks and channels are
//! never stored: variables are reduced to reference counts.

use std::collections::{BTreeMap, BTreeSet};

use super::ast::*;

/// Usage of one identifier name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentStat {
    /// All occurrences.
    pub total: usize,
    /// Occurrences written as a call, `$name(`.
    pub with_args: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    // Line level numbers.
    pub physical_lines: usize,
    pub code_lines: usize,
    pub blank_lines: usize,
    pub comment_lines: usize,
    pub inline_comments: usize,
    pub continuations: usize,
    pub unparenthesised_conditions: usize,

    // Structure.
    pub aliases: usize,
    pub local_aliases: usize,
    pub events: usize,
    pub raws: usize,
    pub ctcps: usize,
    pub menus: usize,
    pub menu_items: usize,
    pub dialogs: usize,
    pub groups: usize,
    /// Count of `Other` items by kind name.
    pub others: BTreeMap<String, usize>,
    pub statements: usize,
    pub max_block_depth: usize,

    // Commands and control flow (lowercased names).
    pub commands: BTreeMap<String, usize>,
    pub control: BTreeMap<String, usize>,
    /// Statements starting with `$`/`%`/`&`.
    pub eval_statements: usize,

    // Identifiers (lowercased names without `$`).
    pub identifiers: BTreeMap<String, IdentStat>,
    /// Positional forms (`$1`, `$2-`, `$1-3`, `$0`) grouped by form.
    pub positional: BTreeMap<String, usize>,

    // Variables: counts only, never names.
    pub global_var_refs: usize,
    pub local_var_refs: usize,
    pub distinct_global_vars: usize,
    pub distinct_local_vars: usize,
    pub binvar_refs: usize,
    pub distinct_binvars: usize,

    // Events.
    /// Uppercased event names.
    pub event_names: BTreeMap<String, usize>,
    /// Level specifiers; unusual ones are grouped as `<named>`.
    pub event_levels: BTreeMap<String, usize>,
    pub raw_numerics: BTreeMap<String, usize>,
    pub menu_windows: BTreeMap<String, usize>,

    /// Constructs the parser could not classify (line and kind only).
    pub unclassified: Vec<Unclassified>,
}

#[derive(Default)]
struct VarSets {
    global: BTreeSet<String>,
    local: BTreeSet<String>,
    bin: BTreeSet<String>,
}

impl Script {
    /// Computes the inventory of this script.
    pub fn inventory(&self) -> Inventory {
        let mut inv = Inventory {
            physical_lines: self.stats.physical_lines,
            code_lines: self.stats.code_lines,
            blank_lines: self.stats.blank_lines,
            comment_lines: self.stats.comment_lines,
            inline_comments: self.stats.inline_comments,
            continuations: self.stats.continuations,
            unparenthesised_conditions: self.stats.unparenthesised_conditions,
            unclassified: self.unclassified.clone(),
            ..Inventory::default()
        };
        let mut vars = VarSets::default();
        for item in &self.items {
            walk_item(item, &mut inv, &mut vars);
        }
        inv.distinct_global_vars = vars.global.len();
        inv.distinct_local_vars = vars.local.len();
        inv.distinct_binvars = vars.bin.len();
        inv
    }
}

fn bump(map: &mut BTreeMap<String, usize>, key: String) {
    *map.entry(key).or_insert(0) += 1;
}

/// Keeps only plain level shapes (`*`, `1`, `!*`, `+5`, ...).
fn level_shape(level: &str) -> String {
    let core = level.trim_start_matches(|c| matches!(c, '!' | '+' | '-' | '@' | '&' | '$' | '?'));
    let ok = core == "*"
        || (!core.is_empty() && core.len() <= 4 && core.bytes().all(|b| b.is_ascii_digit()));
    if ok && level.len() <= 8 {
        level.to_string()
    } else {
        "<named>".to_string()
    }
}

fn simple_name(s: &str, max: usize) -> String {
    if !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'*')
    {
        s.to_string()
    } else {
        "<other>".to_string()
    }
}

fn walk_item(item: &Item, inv: &mut Inventory, vars: &mut VarSets) {
    match item {
        Item::Alias(a) => {
            inv.aliases += 1;
            if a.local {
                inv.local_aliases += 1;
            }
            walk_statements(&a.body, inv, vars, 1);
        }
        Item::Event(e) => {
            inv.events += 1;
            bump(
                &mut inv.event_names,
                simple_name(&e.event, 24).to_uppercase(),
            );
            for l in &e.levels {
                bump(&mut inv.event_levels, level_shape(l));
            }
            for f in &e.fields {
                scan_text(f, inv, vars);
            }
            walk_statements(&e.body, inv, vars, 1);
        }
        Item::Raw(r) => {
            inv.raws += 1;
            bump(
                &mut inv.raw_numerics,
                simple_name(&r.numeric, 16).to_uppercase(),
            );
            scan_text(&r.match_text, inv, vars);
            walk_statements(&r.body, inv, vars, 1);
        }
        Item::Ctcp(c) => {
            inv.ctcps += 1;
            for l in &c.levels {
                bump(&mut inv.event_levels, level_shape(l));
            }
            scan_text(&c.match_text, inv, vars);
            walk_statements(&c.body, inv, vars, 1);
        }
        Item::Menu(m) => {
            inv.menus += 1;
            for w in &m.windows {
                let key = if w.starts_with('@') {
                    "@custom".to_string()
                } else {
                    simple_name(w, 16)
                };
                bump(&mut inv.menu_windows, key);
            }
            walk_menu(&m.items, inv, vars);
        }
        Item::Dialog(_) => inv.dialogs += 1,
        Item::Group(g) => {
            inv.groups += 1;
            for i in &g.items {
                walk_item(i, inv, vars);
            }
        }
        Item::Other(o) => {
            bump(&mut inv.others, o.kind.as_str().to_string());
            walk_statements(&o.statements, inv, vars, 1);
        }
    }
}

fn walk_menu(items: &[MenuItem], inv: &mut Inventory, vars: &mut VarSets) {
    for it in items {
        inv.menu_items += 1;
        scan_text(&it.label, inv, vars);
        walk_statements(&it.body, inv, vars, 1);
        // Menu depth is bounded by the parser (at most 32 levels).
        walk_menu(&it.children, inv, vars);
    }
}

fn walk_statements(stmts: &[Statement], inv: &mut Inventory, vars: &mut VarSets, depth: usize) {
    inv.max_block_depth = inv.max_block_depth.max(depth);
    for s in stmts {
        inv.statements += 1;
        match s.kind {
            StatementKind::Command
            | StatementKind::Var
            | StatementKind::Set
            | StatementKind::Unset
            | StatementKind::Inc
            | StatementKind::Dec => bump(&mut inv.commands, s.name.clone()),
            StatementKind::Eval => {
                inv.eval_statements += 1;
                scan_text(&s.name, inv, vars);
            }
            k => bump(&mut inv.control, k.as_str().to_string()),
        }
        if s.kind != StatementKind::Label {
            scan_text(&s.args, inv, vars);
        }
        // Nesting is bounded by the parser (block and inline depth limits).
        walk_statements(&s.body, inv, vars, depth + 1);
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Scans raw text for identifiers, positional parameters and variables.
fn scan_text(s: &str, inv: &mut Inventory, vars: &mut VarSets) {
    let b = s.as_bytes();
    let n = b.len();
    let mut i = 0;
    while i < n {
        match b[i] {
            b'$' => {
                let mut j = i + 1;
                while j < n && (b[j] == b'$' || b[j] == b'!') {
                    j += 1;
                }
                if j >= n {
                    break;
                }
                let c = b[j];
                if c.is_ascii_digit() {
                    let mut k = j;
                    while k < n && b[k].is_ascii_digit() {
                        k += 1;
                    }
                    let mut form = format!("${}", &s[j..k]);
                    if k < n && b[k] == b'-' {
                        form.push('-');
                        k += 1;
                        let d = k;
                        while k < n && b[k].is_ascii_digit() {
                            k += 1;
                        }
                        form.push_str(&s[d..k]);
                    }
                    bump(&mut inv.positional, form);
                    i = k;
                } else if c == b'+' || c == b'(' {
                    let name = (c as char).to_string();
                    let st = inv.identifiers.entry(name).or_default();
                    st.total += 1;
                    if c == b'(' || (j + 1 < n && b[j + 1] == b'(') {
                        st.with_args += 1;
                    }
                    i = j + 1;
                } else if c.is_ascii_alphabetic() || c == b'_' {
                    let mut k = j;
                    while k < n && is_name_byte(b[k]) {
                        k += 1;
                    }
                    let name = s[j..k].to_ascii_lowercase();
                    let st = inv.identifiers.entry(name).or_default();
                    st.total += 1;
                    if k < n && b[k] == b'(' {
                        st.with_args += 1;
                    }
                    i = k;
                } else {
                    i = j;
                }
            }
            b'%' => {
                let prev_alnum = i > 0 && b[i - 1].is_ascii_alphanumeric();
                let local = i + 1 < n && b[i + 1] == b'%';
                let start = if local { i + 2 } else { i + 1 };
                let mut k = start;
                while k < n && (is_name_byte(b[k]) || b[k] == b'.') {
                    k += 1;
                }
                if !prev_alnum
                    && k > start
                    && (b[start].is_ascii_alphabetic()
                        || b[start] == b'_'
                        || b[start].is_ascii_digit())
                {
                    let name = s[start..k].to_ascii_lowercase();
                    if local {
                        inv.local_var_refs += 1;
                        vars.local.insert(name);
                    } else {
                        inv.global_var_refs += 1;
                        vars.global.insert(name);
                    }
                    i = k;
                } else {
                    i += 1;
                }
            }
            b'&' => {
                let prev_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'&');
                let mut k = i + 1;
                while k < n && is_name_byte(b[k]) {
                    k += 1;
                }
                if prev_ok && k > i + 1 && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
                    inv.binvar_refs += 1;
                    vars.bin.insert(s[i + 1..k].to_ascii_lowercase());
                    i = k;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

const BUILTIN_COMMANDS: &[&str] = &[
    "abook",
    "ajinvite",
    "alias",
    "aline",
    "ame",
    "amsg",
    "anick",
    "aop",
    "auser",
    "autojoin",
    "avoice",
    "away",
    "background",
    "ban",
    "beep",
    "bindip",
    "bcopy",
    "bread",
    "bset",
    "bunset",
    "bwrite",
    "channel",
    "clear",
    "clearall",
    "cline",
    "clipboard",
    "close",
    "closemsg",
    "color",
    "comclose",
    "comopen",
    "comreg",
    "copy",
    "creq",
    "ctcp",
    "ctcpreply",
    "ctcps",
    "dcc",
    "dccserver",
    "dde",
    "ddeserver",
    "debug",
    "dec",
    "describe",
    "dialog",
    "did",
    "didtok",
    "disable",
    "disconnect",
    "dlevel",
    "dline",
    "dll",
    "dns",
    "dqwindow",
    "drawcopy",
    "drawdot",
    "drawfill",
    "drawline",
    "drawpic",
    "drawrect",
    "drawreplace",
    "drawsave",
    "drawscroll",
    "drawtext",
    "ebeeps",
    "echo",
    "editbox",
    "emailaddr",
    "enable",
    "events",
    "exit",
    "fclose",
    "filter",
    "findtext",
    "finger",
    "flash",
    "flist",
    "flood",
    "flush",
    "flushini",
    "font",
    "fopen",
    "fseek",
    "fsend",
    "fserve",
    "fullname",
    "fupdate",
    "fwrite",
    "game",
    "gdi",
    "get",
    "getfile",
    "gload",
    "gmove",
    "gopts",
    "gplay",
    "gpoint",
    "gqreq",
    "groups",
    "gsave",
    "gunload",
    "hadd",
    "halt",
    "haltdef",
    "hdec",
    "hdel",
    "hfree",
    "hinc",
    "hload",
    "hmake",
    "hop",
    "hsave",
    "iaddress",
    "ial",
    "ialclear",
    "ialmark",
    "identd",
    "ignore",
    "iline",
    "inc",
    "invite",
    "join",
    "kick",
    "leave",
    "linesep",
    "list",
    "load",
    "loadbuf",
    "localinfo",
    "log",
    "logview",
    "mdi",
    "me",
    "mkdir",
    "mnick",
    "mode",
    "msg",
    "names",
    "nick",
    "noop",
    "notice",
    "notify",
    "omsg",
    "onotice",
    "part",
    "partall",
    "pdcc",
    "perform",
    "ping",
    "play",
    "pop",
    "protect",
    "pvoice",
    "qme",
    "qmsg",
    "query",
    "queryrn",
    "quit",
    "raw",
    "rawmode",
    "reload",
    "remini",
    "remote",
    "remove",
    "rename",
    "renwin",
    "reseterror",
    "resetidle",
    "rline",
    "rlevel",
    "rmdir",
    "run",
    "ruser",
    "save",
    "savebuf",
    "saveini",
    "say",
    "scid",
    "scon",
    "server",
    "set",
    "signal",
    "silence",
    "sline",
    "sockaccept",
    "sockclose",
    "socklisten",
    "sockmark",
    "sockopen",
    "sockpause",
    "sockread",
    "sockrename",
    "sockudp",
    "sockwrite",
    "sound",
    "speak",
    "splay",
    "sreq",
    "strip",
    "tabkey",
    "time",
    "timer",
    "timers",
    "timestamp",
    "titlebar",
    "tnick",
    "tokenize",
    "toolbar",
    "topic",
    "tray",
    "treebar",
    "uline",
    "ulist",
    "unload",
    "unset",
    "unsetall",
    "updatenl",
    "url",
    "uwho",
    "var",
    "vol",
    "wavplay",
    "whois",
    "window",
    "winhelp",
    "write",
    "writeini",
    "xdcc",
];

const BUILTIN_IDENTIFIERS: &[&str] = &[
    "abs",
    "active",
    "address",
    "addtok",
    "anick",
    "asc",
    "asctime",
    "away",
    "awaymsg",
    "awaytime",
    "base",
    "bfind",
    "bytes",
    "calc",
    "chan",
    "chat",
    "chr",
    "cid",
    "cmd",
    "comchan",
    "cos",
    "count",
    "crc",
    "ctime",
    "date",
    "decode",
    "deltok",
    "dialog",
    "did",
    "dir",
    "dll",
    "duration",
    "emailaddr",
    "encode",
    "event",
    "eval",
    "evalnext",
    "exists",
    "false",
    "file",
    "filename",
    "findfile",
    "findtok",
    "fopen",
    "fgetc",
    "fread",
    "fullname",
    "fulladdress",
    "gettok",
    "gmt",
    "group",
    "hash",
    "height",
    "hfind",
    "hget",
    "hmatch",
    "host",
    "iaddress",
    "ial",
    "ialchan",
    "idle",
    "ifmatch",
    "iif",
    "iifs",
    "ignore",
    "inpaste",
    "input",
    "int",
    "ip",
    "isadmin",
    "isalias",
    "isban",
    "isbit",
    "ischan",
    "isdir",
    "isfile",
    "ishop",
    "isid",
    "isin",
    "isincs",
    "islower",
    "isnum",
    "isop",
    "isreg",
    "istok",
    "isupper",
    "isutf",
    "isvoice",
    "iswm",
    "iswmcs",
    "key",
    "lf",
    "left",
    "len",
    "level",
    "line",
    "lines",
    "lock",
    "lower",
    "maddress",
    "mask",
    "matchtok",
    "max",
    "md5",
    "me",
    "mid",
    "min",
    "mode",
    "modefirst",
    "mouse",
    "network",
    "nick",
    "nofile",
    "nopath",
    "notify",
    "null",
    "numtok",
    "numeric",
    "nvalid",
    "online",
    "opnick",
    "pos",
    "port",
    "prop",
    "puttok",
    "rand",
    "read",
    "readini",
    "regex",
    "regml",
    "regsub",
    "regsubex",
    "remove",
    "remtok",
    "replace",
    "replacex",
    "reptok",
    "right",
    "round",
    "scid",
    "scon",
    "server",
    "sha1",
    "sha256",
    "sin",
    "snotice",
    "sock",
    "sockbr",
    "sockerr",
    "sockname",
    "sockstate",
    "sorttok",
    "sqrt",
    "starttime",
    "status",
    "str",
    "strip",
    "stripped",
    "style",
    "submenu",
    "target",
    "tan",
    "time",
    "timer",
    "token",
    "topic",
    "true",
    "ticks",
    "uptime",
    "upper",
    "usermode",
    "version",
    "wid",
    "width",
    "window",
    "wildsite",
    "wildtok",
    "dname",
    "cr",
    "crlf",
    "cb",
    "comchan",
    "mnick",
    "nopath",
    "ctrlenter",
    "hotlink",
    "menu",
    "menutype",
    "menucontext",
    "hnick",
    "vnick",
    "rnick",
    "ulevel",
    "readn",
    "1",
];

/// True for command names documented in the script command set. Anything else
/// is a user-defined alias call or unknown.
pub fn is_builtin_command(name: &str) -> bool {
    BUILTIN_COMMANDS.contains(&name)
}

/// True for common built-in script identifier names (lowercase, without `$`).
pub fn is_builtin_identifier(name: &str) -> bool {
    matches!(name, "+" | "(") || BUILTIN_IDENTIFIERS.contains(&name)
}
