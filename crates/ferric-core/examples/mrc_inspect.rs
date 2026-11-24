//! Prints the structure summary and the inventory of a .mrc script file.
//!
//! Usage: cargo run -p ferric-core --example mrc_inspect -- <file>
//!
//! The output contains counts, kinds, line numbers and the names of
//! commands, identifiers and events only. It never prints strings,
//! arguments, hosts, nicks, channels, keys or alias names. Names that are not
//! built-in script commands or identifiers are folded into one counter.

use std::collections::BTreeMap;
use std::process::ExitCode;

use ferric_core::mrc::{
    self, is_builtin_command, is_builtin_identifier, Item, MenuItem, Script, Statement,
};

fn count_statements(stmts: &[Statement]) -> usize {
    stmts.iter().map(|s| 1 + count_statements(&s.body)).sum()
}

fn count_menu(items: &[MenuItem]) -> usize {
    items.iter().map(|i| 1 + count_menu(&i.children)).sum()
}

fn print_item(item: &Item, indent: usize) {
    let pad = " ".repeat(indent);
    match item {
        Item::Alias(a) => println!(
            "{pad}line {:>4}-{:<4} alias{} statements={}",
            a.line,
            a.end_line,
            if a.local { " (local)" } else { "" },
            count_statements(&a.body)
        ),
        Item::Event(e) => println!(
            "{pad}line {:>4}-{:<4} event {} levels={} fields={} statements={}",
            e.line,
            e.end_line,
            e.event.to_uppercase(),
            e.levels.len(),
            e.fields.len(),
            count_statements(&e.body)
        ),
        Item::Raw(r) => println!(
            "{pad}line {:>4}-{:<4} raw statements={}",
            r.line,
            r.end_line,
            count_statements(&r.body)
        ),
        Item::Ctcp(c) => println!(
            "{pad}line {:>4}-{:<4} ctcp statements={}",
            c.line,
            c.end_line,
            count_statements(&c.body)
        ),
        Item::Menu(m) => println!(
            "{pad}line {:>4}-{:<4} menu windows={} items={}",
            m.line,
            m.end_line,
            m.windows.len(),
            count_menu(&m.items)
        ),
        Item::Dialog(d) => println!(
            "{pad}line {:>4}-{:<4} dialog{} content_lines={}",
            d.line,
            d.end_line,
            if d.local { " (local)" } else { "" },
            d.content_lines
        ),
        Item::Group(g) => {
            println!(
                "{pad}line {:>4}-{:<4} group {} items={}",
                g.line,
                g.end_line.map_or("?".to_string(), |e| e.to_string()),
                if g.enabled { "on" } else { "off" },
                g.items.len()
            );
            for i in &g.items {
                print_item(i, indent + 2);
            }
        }
        Item::Other(o) => println!(
            "{pad}line {:>4}-{:<4} other kind={} statements={}",
            o.line,
            o.end_line,
            o.kind.as_str(),
            count_statements(&o.statements)
        ),
    }
}

/// Prints a name -> count table; names that are not built-ins are folded.
fn print_names(title: &str, map: &BTreeMap<String, usize>, known: fn(&str) -> bool) {
    let mut rows: Vec<(&str, usize)> = Vec::new();
    let mut other_kinds = 0usize;
    let mut other_total = 0usize;
    for (name, &n) in map {
        if known(name) {
            rows.push((name.as_str(), n));
        } else {
            other_kinds += 1;
            other_total += n;
        }
    }
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\n{title}");
    for (name, n) in rows {
        println!("  {name:<16} {n}");
    }
    if other_kinds > 0 {
        println!("  <non-builtin>    {other_total} uses of {other_kinds} distinct names");
    }
}

fn print_plain(title: &str, map: &BTreeMap<String, usize>) {
    println!("\n{title}");
    if map.is_empty() {
        println!("  (none)");
    }
    for (k, v) in map {
        println!("  {k:<16} {v}");
    }
}

fn report(script: &Script) {
    let st = &script.stats;
    println!("== Structure ==");
    println!(
        "encoding={} physical_lines={} code_lines={} blank={} comment_lines={} inline_comments={} continuations={}",
        st.encoding.as_str(),
        st.physical_lines,
        st.code_lines,
        st.blank_lines,
        st.comment_lines,
        st.inline_comments,
        st.continuations
    );
    for item in &script.items {
        print_item(item, 0);
    }

    let inv = script.inventory();
    println!("\n== Inventory ==");
    println!(
        "aliases={} (local {}) events={} raws={} ctcps={} menus={} (items {}) dialogs={} groups={}",
        inv.aliases,
        inv.local_aliases,
        inv.events,
        inv.raws,
        inv.ctcps,
        inv.menus,
        inv.menu_items,
        inv.dialogs,
        inv.groups
    );
    println!(
        "statements={} max_block_depth={} eval_statements={} unparenthesised_conditions={}",
        inv.statements, inv.max_block_depth, inv.eval_statements, inv.unparenthesised_conditions
    );
    print_plain("other items by kind", &inv.others);
    print_names("commands", &inv.commands, is_builtin_command);
    print_plain("control flow", &inv.control);

    let ids: BTreeMap<String, usize> = inv
        .identifiers
        .iter()
        .map(|(k, v)| (k.clone(), v.total))
        .collect();
    print_names("identifiers", &ids, is_builtin_identifier);
    print_plain("positional parameters", &inv.positional);
    println!(
        "\nvariables (counts only): global refs={} ({} distinct), local refs={} ({} distinct), binvar refs={} ({} distinct)",
        inv.global_var_refs,
        inv.distinct_global_vars,
        inv.local_var_refs,
        inv.distinct_local_vars,
        inv.binvar_refs,
        inv.distinct_binvars
    );
    print_plain("event names", &inv.event_names);
    print_plain("event levels", &inv.event_levels);
    print_plain("raw numerics", &inv.raw_numerics);
    print_plain("menu windows", &inv.menu_windows);

    println!("\n== Unclassified constructs ==");
    if inv.unclassified.is_empty() {
        println!("  (none)");
    }
    for u in &inv.unclassified {
        println!("  line {:>4}: {}", u.line, u.kind.as_str());
    }
}

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: mrc_inspect <file>");
        return ExitCode::from(2);
    };
    match mrc::parse_file(&path) {
        Ok(script) => {
            report(&script);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("cannot read file: {:?}", e.kind());
            ExitCode::from(1)
        }
    }
}
