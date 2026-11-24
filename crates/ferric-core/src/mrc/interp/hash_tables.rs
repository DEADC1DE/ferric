//! Hash tables, in memory: the commands `/hmake`, `/hfree`, `/hadd`, `/hdel`,
//! `/hinc`, `/hdec` and the identifiers `$hget` and `$hfind`.
//!
//! Semantics follow the scripting reference `hash_tables`. Tables live in the
//! `Runtime` and outlive a single alias run. Item names are case-insensitive.
//!
//! Not implemented, and each stops the run with a notice: `/hload` and
//! `/hsave` (files), the `-u`, `-k` and `-z` timers and `-c` and `-b` binary
//! items of `/hadd`, the `&binvar` form of `$hget`, the `unset` property, the
//! window and command forms of `$hfind`, and regular-expression matching (`r`, `R`).
//!
//! Ferric choices:
//! * `-s` (display the result) is accepted and prints nothing.
//! * `/hmake` on an existing name replaces that table.
//! * `/hadd`, `/hinc` and `/hdec` stop when there is no table and no `-m`.
//!   `/hfree` and `/hdel` do nothing when the table or item is missing.
//! * Items are listed in name order. The client gives no order and the help says
//!   not to depend on one.
//! * `/hinc` and `/hdec` take whole numbers only.
//! * Limits: 1000 tables, 50000 items per table, and 64 MB of item names and data in all.

use std::collections::BTreeMap;

use super::runtime::wildcard_match;
use super::text_funcs::CallError;
use super::value::split_word;

const MAX_TABLES: usize = 1_000;
const MAX_ITEMS: usize = 50_000;

/// One table: its name as written, its slot count from `/hmake`, and its items
/// by lower-case name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Table {
    name: String,
    slots: u64,
    items: BTreeMap<String, (String, String)>,
}

/// Most bytes the items of all tables hold together: each item's name and its data.
const MAX_BYTES: usize = 64 * 1024 * 1024;

/// All tables by lower-case name, and the bytes their items hold. Reads go through `Deref`;
/// changes go through the methods below, which keep the byte count right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hashes {
    tables: BTreeMap<String, Table>,
    bytes: usize,
    budget: usize,
}

impl Default for Hashes {
    fn default() -> Self {
        Self {
            tables: BTreeMap::new(),
            bytes: 0,
            budget: MAX_BYTES,
        }
    }
}

impl std::ops::Deref for Hashes {
    type Target = BTreeMap<String, Table>;

    fn deref(&self) -> &Self::Target {
        &self.tables
    }
}

fn table_bytes(table: &Table) -> usize {
    table
        .items
        .iter()
        .map(|(name, (_, data))| name.len() + data.len())
        .sum()
}

impl Hashes {
    #[cfg(test)]
    fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }

    /// Adds a table, or replaces the one with the same name.
    fn put_table(&mut self, name: &str, table: Table) {
        if let Some(old) = self.tables.insert(key(name), table) {
            self.bytes = self.bytes.saturating_sub(table_bytes(&old));
        }
    }

    fn remove_table(&mut self, name: &str) {
        if let Some(old) = self.tables.remove(&key(name)) {
            self.bytes = self.bytes.saturating_sub(table_bytes(&old));
        }
    }

    fn retain_tables(&mut self, mut keep: impl FnMut(&Table) -> bool) {
        let mut freed = 0;
        self.tables.retain(|_, t| {
            let kept = keep(t);
            if !kept {
                freed += table_bytes(t);
            }
            kept
        });
        self.bytes = self.bytes.saturating_sub(freed);
    }

    /// Stores an item in a table that exists. Refused when the table has too many items or the
    /// byte budget would be exceeded; replacing an item gives back its old bytes first.
    fn set_item(&mut self, table: &str, item: &str, data: &str) -> Result<(), String> {
        let t = self
            .tables
            .get_mut(&key(table))
            .ok_or_else(|| format!("there is no hash table named '{table}'"))?;
        let name = key(item);
        let old = t.items.get(&name).map(|(_, old)| name.len() + old.len());
        if old.is_none() && t.items.len() >= MAX_ITEMS {
            return Err("too many items in the hash table".into());
        }
        let total = self.bytes + name.len() + data.len() - old.unwrap_or(0);
        if total > self.budget {
            return Err("the hash tables are full".into());
        }
        t.items.insert(name, (item.to_string(), data.to_string()));
        self.bytes = total;
        Ok(())
    }

    fn remove_item(&mut self, table: &str, item: &str) {
        let name = key(item);
        if let Some(t) = self.tables.get_mut(&key(table)) {
            if let Some((_, data)) = t.items.remove(&name) {
                self.bytes = self.bytes.saturating_sub(name.len() + data.len());
            }
        }
    }

    fn retain_items(&mut self, table: &str, mut keep: impl FnMut(&str) -> bool) {
        if let Some(t) = self.tables.get_mut(&key(table)) {
            let mut freed = 0;
            t.items.retain(|name, (shown, data)| {
                let kept = keep(shown.as_str());
                if !kept {
                    freed += name.len() + data.len();
                }
                kept
            });
            self.bytes = self.bytes.saturating_sub(freed);
        }
    }
}

/// Parses leading switches (`-s`, `-mN`, `-w`) of a command. Returns the switch
/// letters with their numbers, and the rest of the text.
fn switches(text: &str) -> (Vec<(char, Option<u64>)>, &str) {
    let mut found: Vec<(char, Option<u64>)> = Vec::new();
    let mut rest = text.trim_start();
    loop {
        let (word, after) = split_word(rest);
        let is_switch = word.len() >= 2
            && word.starts_with('-')
            && word[1..].chars().all(|c| c.is_ascii_alphanumeric());
        if !is_switch {
            return (found, rest);
        }
        for c in word[1..].chars() {
            if let Some(d) = c.to_digit(10) {
                if let Some(last) = found.last_mut() {
                    last.1 = Some(last.1.unwrap_or(0).saturating_mul(10).saturating_add(u64::from(d)));
                }
            } else {
                found.push((c, None));
            }
        }
        rest = after;
    }
}

fn check_switches(found: &[(char, Option<u64>)], allowed: &str) -> Result<(), String> {
    for (c, _) in found {
        if !allowed.contains(*c) {
            return Err(format!("switch -{c} is not supported by hash tables"));
        }
    }
    Ok(())
}

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// The first word and the text after it.
fn word(text: &str) -> (&str, &str) {
    split_word(text)
}

fn table_ref<'a>(hashes: &'a Hashes, name: &str) -> Result<&'a Table, String> {
    hashes
        .get(&key(name))
        .ok_or_else(|| format!("there is no hash table named '{name}'"))
}

/// Runs one hash command. The text is the evaluated argument text.
pub(crate) fn command(hashes: &mut Hashes, name: &str, text: &str) -> Result<(), String> {
    let (found, rest) = switches(text);
    match name {
        "hmake" => {
            check_switches(&found, "s")?;
            let (table, slots) = word(rest);
            let (n, _) = word(slots);
            let slots: u64 = n.parse().ok().filter(|n| *n >= 1).ok_or_else(|| {
                "/hmake needs a table name and a slot count of 1 or more".to_string()
            })?;
            if table.is_empty() {
                return Err("/hmake needs a table name".into());
            }
            if !hashes.contains_key(&key(table)) && hashes.len() >= MAX_TABLES {
                return Err("too many hash tables".into());
            }
            hashes.put_table(
                table,
                Table {
                    name: table.to_string(),
                    slots,
                    items: BTreeMap::new(),
                },
            );
            Ok(())
        }
        "hfree" => {
            check_switches(&found, "sw")?;
            let wild = found.iter().any(|(c, _)| *c == 'w');
            let (table, _) = word(rest);
            if wild {
                hashes.retain_tables(|t| !wildcard_match(table, &t.name));
            } else {
                hashes.remove_table(table);
            }
            Ok(())
        }
        "hadd" => {
            check_switches(&found, "sm")?;
            let (table, after) = word(rest);
            let (item, data) = word(after);
            if table.is_empty() || item.is_empty() {
                return Err("/hadd needs a table name and an item".into());
            }
            create_if_asked(hashes, &found, table)?;
            hashes.set_item(table, item, data)
        }
        "hdel" => {
            check_switches(&found, "sw")?;
            let wild = found.iter().any(|(c, _)| *c == 'w');
            let (table, after) = word(rest);
            let (item, _) = word(after);
            if wild {
                hashes.retain_items(table, |name| !wildcard_match(item, name));
            } else {
                hashes.remove_item(table, item);
            }
            Ok(())
        }
        "hinc" | "hdec" => {
            check_switches(&found, "sm")?;
            let (table, after) = word(rest);
            let (item, value) = word(after);
            if table.is_empty() || item.is_empty() {
                return Err(format!("/{name} needs a table name and an item"));
            }
            create_if_asked(hashes, &found, table)?;
            let by: i64 = if value.is_empty() {
                1
            } else {
                value
                    .trim()
                    .parse()
                    .map_err(|_| format!("'{value}' is not a whole number"))?
            };
            let current = match table_ref(hashes, table)?.items.get(&key(item)) {
                Some((_, data)) => data
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| format!("the item '{item}' is not a number"))?,
                None => 0,
            };
            let next = if name == "hinc" {
                current.checked_add(by)
            } else {
                current.checked_sub(by)
            }
            .ok_or_else(|| "the number overflows".to_string())?;
            hashes.set_item(table, item, &next.to_string())
        }
        other => Err(format!("unknown or unsupported command '{other}'")),
    }
}

/// Creates the table for `-mN` when it is missing, and fails when it is not there.
fn create_if_asked(
    hashes: &mut Hashes,
    found: &[(char, Option<u64>)],
    table: &str,
) -> Result<(), String> {
    if let Some((_, slots)) = found.iter().find(|(c, _)| *c == 'm') {
        if !hashes.contains_key(&key(table)) {
            let slots = slots
                .filter(|n| *n >= 1)
                .ok_or_else(|| "-m needs a slot count of 1 or more".to_string())?;
            if hashes.len() >= MAX_TABLES {
                return Err("too many hash tables".into());
            }
            hashes.put_table(
                table,
                Table {
                    name: table.to_string(),
                    slots,
                    items: BTreeMap::new(),
                },
            );
        }
    }
    Ok(())
}

fn nth_item(t: &Table, n: usize) -> Option<&(String, String)> {
    t.items.values().nth(n - 1)
}

/// Answers `$hget` and `$hfind`. `args` are the evaluated parameters and `prop`
/// the lower-case property after the call, if any.
pub(crate) fn ident(
    hashes: &Hashes,
    name: &str,
    args: &[String],
    prop: Option<&str>,
) -> Result<String, CallError> {
    let unsupported =
        |what: &str| CallError::Arguments(format!("{what} is not supported by Ferric"));
    match (name, prop) {
        (_, Some("unset")) => return Err(unsupported("the unset property")),
        (_, Some(p))
            if !(name == "hget" && matches!(p, "size" | "item" | "data"))
                && !(name == "hfind" && p == "data") =>
        {
            return Err(CallError::Arguments(format!(
                "the .{p} property of ${name} is not supported"
            )));
        }
        _ => {}
    }
    match name {
        "hget" => hget(hashes, args, prop),
        "hfind" => hfind(hashes, args, prop),
        other => Err(CallError::Arguments(format!(
            "${other} is not a hash identifier"
        ))),
    }
}

fn hget(hashes: &Hashes, args: &[String], prop: Option<&str>) -> Result<String, CallError> {
    match args.len() {
        1 => {
            let arg = args[0].trim();
            if let Ok(n) = arg.parse::<usize>() {
                // $hget(N): the Nth table name, or the number of tables for 0.
                if prop.is_some() {
                    return Err(CallError::Arguments("$hget(N) takes no property".into()));
                }
                return Ok(if n == 0 {
                    hashes.len().to_string()
                } else {
                    hashes
                        .values()
                        .nth(n - 1)
                        .map_or(String::new(), |t| t.name.clone())
                });
            }
            Ok(match hashes.get(&key(arg)) {
                None => String::new(),
                Some(t) if prop == Some("size") => t.slots.to_string(),
                Some(t) if prop.is_none() => t.name.clone(),
                Some(_) => {
                    return Err(CallError::Arguments(
                        "$hget(name).item and .data need a position".into(),
                    ))
                }
            })
        }
        2 => {
            let Some(t) = hashes.get(&key(&args[0])) else {
                return Ok(String::new());
            };
            match prop {
                None => Ok(t
                    .items
                    .get(&key(&args[1]))
                    .map_or(String::new(), |(_, d)| d.clone())),
                Some(p) => {
                    let n: usize = args[1].trim().parse().map_err(|_| {
                        CallError::Arguments("the position of $hget must be a number".into())
                    })?;
                    if n == 0 {
                        return Ok(t.items.len().to_string());
                    }
                    Ok(match (nth_item(t, n), p) {
                        (Some((item, _)), "item") => item.clone(),
                        (Some((_, data)), "data") => data.clone(),
                        _ => String::new(),
                    })
                }
            }
        }
        _ => Err(CallError::Arguments(
            "$hget takes a table and an item, or a table and a position".into(),
        )),
    }
}

fn hfind(hashes: &Hashes, args: &[String], prop: Option<&str>) -> Result<String, CallError> {
    if !(3..=4).contains(&args.len()) {
        return Err(CallError::Arguments(
            "$hfind takes a table, a text, N and an optional match type".into(),
        ));
    }
    let nth: usize = args[2]
        .trim()
        .parse()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or_else(|| CallError::Arguments("$hfind needs an N of 1 or more".into()))?;
    let mode = args.get(3).map_or("n", String::as_str);
    if !matches!(mode, "n" | "w" | "W") {
        return Err(CallError::Arguments(format!(
            "$hfind match type '{mode}' is not supported; r and R (regular expressions) are not"
        )));
    }
    let Some(t) = hashes.get(&key(&args[0])) else {
        return Ok(String::new());
    };
    let text = args[1].as_str();
    let on_data = prop == Some("data");
    let hit = |item: &str, data: &str| {
        let subject = if on_data { data } else { item };
        match mode {
            "n" => subject.eq_ignore_ascii_case(text),
            "w" => wildcard_match(text, subject),
            _ => wildcard_match(subject, text),
        }
    };
    let found = t
        .items
        .values()
        .filter(|(item, data)| hit(item, data))
        .nth(nth - 1);
    Ok(found.map_or(String::new(), |(item, _)| item.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// Runs one command line: the first word is the command, the rest its text.
    fn line(h: &mut Hashes, text: &str) -> Result<(), String> {
        let (name, rest) = split_word(text);
        command(h, name, rest)
    }

    #[test]
    fn make_add_get_and_free() {
        let mut h = Hashes::default();
        line(&mut h, "hmake -s moo 100").unwrap();
        assert_eq!(
            ident(&h, "hget", &v(&["moo"]), Some("size")).unwrap(),
            "100"
        );
        line(&mut h, "hadd moo hello big world").unwrap();
        assert_eq!(
            ident(&h, "hget", &v(&["moo", "hello"]), None).unwrap(),
            "big world"
        );
        assert_eq!(
            ident(&h, "hget", &v(&["MOO", "HELLO"]), None).unwrap(),
            "big world"
        );
        assert_eq!(ident(&h, "hget", &v(&["moo", "nope"]), None).unwrap(), "");
        assert_eq!(ident(&h, "hget", &v(&["1"]), None).unwrap(), "moo");
        assert_eq!(ident(&h, "hget", &v(&["0"]), None).unwrap(), "1");
        line(&mut h, "hfree moo").unwrap();
        assert_eq!(ident(&h, "hget", &v(&["moo"]), None).unwrap(), "");
    }

    #[test]
    fn the_byte_budget_refuses_an_item_past_it() {
        let mut h = Hashes::with_budget(20);
        line(&mut h, "hmake t 5").unwrap();
        line(&mut h, "hadd t a 0123456789").unwrap();
        assert!(line(&mut h, "hadd t b 0123456789").is_err());
        // Replacing an item gives back its old bytes first.
        line(&mut h, "hadd t a 01234").unwrap();
        line(&mut h, "hadd t b 0123456789").unwrap();
        line(&mut h, "hdel t b").unwrap();
        assert_eq!(h.bytes, 6);
        line(&mut h, "hfree t").unwrap();
        assert_eq!(h.bytes, 0);
    }

    #[test]
    fn counting_and_deleting() {
        let mut h = Hashes::default();
        line(&mut h, "hadd -m10 t a 1").unwrap();
        line(&mut h, "hinc t a 4").unwrap();
        line(&mut h, "hinc t b").unwrap();
        line(&mut h, "hdec t b 3").unwrap();
        assert_eq!(ident(&h, "hget", &v(&["t", "a"]), None).unwrap(), "5");
        assert_eq!(ident(&h, "hget", &v(&["t", "b"]), None).unwrap(), "-2");
        assert_eq!(
            ident(&h, "hget", &v(&["t", "0"]), Some("item")).unwrap(),
            "2"
        );
        line(&mut h, "hdel t a").unwrap();
        assert_eq!(ident(&h, "hget", &v(&["t", "a"]), None).unwrap(), "");
    }

    #[test]
    fn positions_and_find() {
        let mut h = Hashes::default();
        line(&mut h, "hadd -m5 t apple red").unwrap();
        line(&mut h, "hadd t apricot orange").unwrap();
        line(&mut h, "hadd t banana yellow").unwrap();
        assert_eq!(
            ident(&h, "hget", &v(&["t", "1"]), Some("item")).unwrap(),
            "apple"
        );
        assert_eq!(
            ident(&h, "hget", &v(&["t", "2"]), Some("data")).unwrap(),
            "orange"
        );
        assert_eq!(
            ident(&h, "hfind", &v(&["t", "a*", "2", "w"]), None).unwrap(),
            "apricot"
        );
        assert_eq!(
            ident(&h, "hfind", &v(&["t", "orange", "1"]), Some("data")).unwrap(),
            "apricot"
        );
        assert_eq!(ident(&h, "hfind", &v(&["t", "zz", "1"]), None).unwrap(), "");
        assert_eq!(
            ident(&h, "hfind", &v(&["t", "apple", "1", "W"]), None).unwrap(),
            "apple"
        );
        assert_eq!(
            ident(&h, "hfind", &v(&["t", "x", "1", "W"]), None).unwrap(),
            ""
        );
    }

    #[test]
    fn missing_tables_and_unsupported_forms_stop() {
        let mut h = Hashes::default();
        assert!(line(&mut h, "hadd nope a b",).is_err());
        assert!(line(&mut h, "hinc nope a",).is_err());
        assert!(line(&mut h, "hadd -u5 t a b").is_err());
        assert!(matches!(
            ident(&h, "hget", &v(&["t", "a"]), Some("unset")),
            Err(CallError::Arguments(_))
        ));
        assert!(matches!(
            ident(&h, "hfind", &v(&["t", "a", "1", "r"]), None),
            Err(CallError::Arguments(_))
        ));
    }
}
