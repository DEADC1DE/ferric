//! Tests use small synthetic fixtures only.

use super::*;

fn alias(s: &Script, idx: usize) -> &Alias {
    match &s.items[idx] {
        Item::Alias(a) => a,
        other => panic!("expected alias, got {}", other.kind_name()),
    }
}

fn event(s: &Script, idx: usize) -> &Event {
    match &s.items[idx] {
        Item::Event(e) => e,
        other => panic!("expected event, got {}", other.kind_name()),
    }
}

fn names(stmts: &[Statement]) -> Vec<&str> {
    stmts.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn comments_blank_lines_and_line_endings() {
    let lf = "; header\n\nalias foo {\n  ; inside\n  echo -a hi\n}\n\n; tail\n";
    let crlf = lf.replace('\n', "\r\n");
    let a = parse_str(lf);
    let b = parse_str(&crlf);
    assert_eq!(a.items, b.items);
    assert_eq!(a.items.len(), 1);
    assert_eq!(a.stats.comment_lines, 3);
    assert_eq!(a.stats.blank_lines, 2);
    assert_eq!(a.stats.physical_lines, 8);
    let al = alias(&a, 0);
    assert_eq!((al.line, al.end_line), (3, 6));
    assert_eq!(names(&al.body), ["echo"]);
    assert!(a.unclassified.is_empty());
}

#[test]
fn block_comments_are_skipped() {
    let s = parse_str("/* one\ntwo\n*/\n/* single */\nalias a echo x\n");
    assert_eq!(s.items.len(), 1);
    assert_eq!(s.stats.comment_lines, 4);
    assert_eq!(alias(&s, 0).line, 5);
}

#[test]
fn inline_comment_after_brace() {
    let s = parse_str("alias a { ; note | echo not-run\n  echo x ; kept as text\n}\n");
    let a = alias(&s, 0);
    assert_eq!(names(&a.body), ["echo"]);
    assert_eq!(a.body[0].args, "x ; kept as text");
    assert_eq!(s.stats.inline_comments, 1);
}

#[test]
fn line_continuation() {
    let s = parse_str("alias a {\n  echo one $&\n    two $&\n  three\n  echo next\n}\n");
    let a = alias(&s, 0);
    assert_eq!(s.stats.continuations, 2);
    assert_eq!(a.body.len(), 2);
    assert_eq!(a.body[0].line, 2);
    assert_eq!(a.body[0].args, "one two three");
    assert_eq!(a.body[1].line, 5);
    assert_eq!(a.end_line, 6);
}

#[test]
fn comment_line_does_not_continue() {
    let s = parse_str("; note $&\nalias a echo x\n");
    assert_eq!(s.items.len(), 1);
    assert_eq!(s.stats.continuations, 0);
}

#[test]
fn alias_forms() {
    let src = "alias one echo a | echo b\n\
               alias -l two {\n  echo c\n}\n\
               alias three\n{\n  echo d\n}\n\
               alias four { echo e | echo f }\n\
               alias five\n\
               ALIAS Six echo g\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 6);
    let one = alias(&s, 0);
    assert_eq!((one.name.as_str(), one.local), ("one", false));
    assert_eq!(names(&one.body), ["echo", "echo"]);
    let two = alias(&s, 1);
    assert!(two.local);
    assert_eq!(names(&two.body), ["echo"]);
    assert_eq!(alias(&s, 2).name, "three");
    assert_eq!(names(&alias(&s, 2).body), ["echo"]);
    assert_eq!(names(&alias(&s, 3).body), ["echo", "echo"]);
    assert!(alias(&s, 4).body.is_empty());
    assert_eq!(alias(&s, 5).name, "six");
}

#[test]
fn statement_kinds_and_prefixes() {
    let src = "alias a {\n\
      /echo -a x\n\
      .msg # hi\n\
      //echo $me\n\
      var %x = 1, %y = 2\n\
      set %g 3\n\
      inc %g\n\
      dec %g 2\n\
      unset %g\n\
      goto end\n\
      :end\n\
      return $1-\n\
      $iif(1,echo a)\n\
    }\n";
    let s = parse_str(src);
    let a = alias(&s, 0);
    use StatementKind::*;
    let kinds: Vec<_> = a.body.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [Command, Command, Command, Var, Set, Inc, Dec, Unset, Goto, Label, Return, Eval]
    );
    assert_eq!(a.body[0].name, "echo");
    assert!(a.body[1].silent);
    assert_eq!(a.body[1].name, "msg");
    assert_eq!(a.body[2].name, "echo");
    assert_eq!(a.body[9].name, "end");
    assert_eq!(a.body[10].args, "$1-");
    assert_eq!(a.body[11].name, "$iif");
}

#[test]
fn control_flow_blocks() {
    let src = "alias a {\n\
      if ($1 == a) {\n\
        echo one\n\
      }\n\
      elseif ($1 == b) { echo two }\n\
      else {\n\
        echo three\n\
      }\n\
      while ($x < 3) {\n\
        inc %x\n\
        if (%x == 2) break\n\
        continue\n\
      }\n\
    }\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    let a = alias(&s, 0);
    use StatementKind::*;
    let kinds: Vec<_> = a.body.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [If, ElseIf, Else, While]);
    assert_eq!(a.body[0].args, "($1 == a)");
    assert_eq!(names(&a.body[0].body), ["echo"]);
    assert_eq!(names(&a.body[1].body), ["echo"]);
    assert_eq!(names(&a.body[2].body), ["echo"]);
    let w = &a.body[3];
    assert_eq!(w.body.len(), 3);
    assert_eq!(w.body[1].kind, If);
    assert_eq!(w.body[1].body[0].kind, Break);
    assert!(w.body[1].body[0].inline);
    assert_eq!(w.body[2].kind, Continue);
}

#[test]
fn brace_on_next_line_and_close_else_open() {
    let src = "alias a {\n if (x)\n {\n  echo 1\n } else {\n  echo 2\n }\n}\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    let a = alias(&s, 0);
    assert_eq!(a.body.len(), 2);
    assert_eq!(a.body[0].kind, StatementKind::If);
    assert_eq!(a.body[0].body.len(), 1);
    assert_eq!(a.body[1].kind, StatementKind::Else);
    assert_eq!(a.body[1].body.len(), 1);
}

#[test]
fn inline_if_else_and_else_if() {
    let s = parse_str(
        "alias a {\n if (x) echo a else echo b\n if (y) { echo c } else if (z) echo d\n}\n",
    );
    let a = alias(&s, 0);
    use StatementKind::*;
    let kinds: Vec<_> = a.body.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [If, Else, If, ElseIf]);
    assert_eq!(names(&a.body[0].body), ["echo"]);
    assert_eq!(names(&a.body[1].body), ["echo"]);
    assert_eq!(names(&a.body[3].body), ["echo"]);
    assert_eq!(s.stats.unparenthesised_conditions, 0);
}

#[test]
fn unparenthesised_condition_is_counted() {
    let s = parse_str("alias a {\n if %n isop # echo x\n}\n");
    assert_eq!(s.stats.unparenthesised_conditions, 1);
    assert_eq!(alias(&s, 0).body[0].kind, StatementKind::If);
}

#[test]
fn pipe_rules() {
    let s = parse_str(
        "alias a {\n echo 1 | echo 2\n if ($1 == a || $1 == b) echo 3\n echo $iif(x,a | b,c) | echo 4\n echo $+(a,b) | echo 5\n}\n",
    );
    let a = alias(&s, 0);
    let n: Vec<_> = a.body.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(n, ["echo", "echo", "if", "echo", "echo", "echo", "echo"]);
    assert_eq!(a.body[2].body[0].name, "echo");
    assert_eq!(a.body[3].args, "$iif(x,a | b,c)");
}

#[test]
fn events() {
    let src = "on *:TEXT:*hello*:#:{\n  msg $chan hi\n}\n\
               on 1,2:JOIN:#:echo joined\n\
               on !*:NICK:echo nick\n\
               on *:LOAD:echo load\n\
               on *:START:{ echo a | echo b }\n\
               on *:CONNECT:\n{\n  echo c\n}\n\
               on +5:KICK:#chan:echo kicked\n\
               on *:SOCKOPEN:name:{ echo s }\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 8);
    let t = event(&s, 0);
    assert_eq!(t.event, "text");
    assert_eq!(t.levels, ["*"]);
    assert_eq!(t.fields, ["*hello*", "#"]);
    assert_eq!((t.line, t.end_line), (1, 3));
    assert_eq!(names(&t.body), ["msg"]);
    let j = event(&s, 1);
    assert_eq!(j.levels, ["1", "2"]);
    assert_eq!(j.fields, ["#"]);
    assert_eq!(names(&j.body), ["echo"]);
    let n = event(&s, 2);
    assert_eq!(
        (n.event.as_str(), n.levels[0].as_str(), n.fields.len()),
        ("nick", "!*", 0)
    );
    assert_eq!(event(&s, 3).event, "load");
    assert_eq!(names(&event(&s, 4).body), ["echo", "echo"]);
    let c = event(&s, 5);
    assert_eq!(c.event, "connect");
    assert_eq!(names(&c.body), ["echo"]);
    assert_eq!(event(&s, 6).levels, ["+5"]);
    assert_eq!(event(&s, 7).fields, ["name"]);
}

#[test]
fn raw_and_ctcp() {
    let src = "raw 001:*:{\n  echo welcome\n}\n\
               raw *:*:echo any\n\
               raw 322:*x*:/halt\n\
               ctcp 1:VERSION:*:{ ctcpreply $nick VERSION v | halt }\n\
               ctcp +5:help:?:msg $nick no\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 5);
    match &s.items[0] {
        Item::Raw(r) => {
            assert_eq!(r.numeric, "001");
            assert_eq!(r.match_text, "*");
            assert_eq!(names(&r.body), ["echo"]);
        }
        _ => panic!(),
    }
    match &s.items[2] {
        Item::Raw(r) => assert_eq!(
            (r.numeric.as_str(), r.body[0].name.as_str()),
            ("322", "halt")
        ),
        _ => panic!(),
    }
    match &s.items[3] {
        Item::Ctcp(c) => {
            assert_eq!(c.levels, ["1"]);
            assert_eq!(c.match_text, "VERSION");
            assert_eq!(c.target, "*");
            assert_eq!(names(&c.body), ["ctcpreply", "halt"]);
        }
        _ => panic!(),
    }
    match &s.items[4] {
        Item::Ctcp(c) => assert_eq!((c.levels[0].as_str(), c.target.as_str()), ("+5", "?")),
        _ => panic!(),
    }
}

#[test]
fn raw_and_ctcp_commands_are_not_headers() {
    let s = parse_str("raw PRIVMSG x hello\nctcp nick VERSION\n");
    assert_eq!(s.items.len(), 2);
    for item in &s.items {
        match item {
            Item::Other(o) => assert_eq!(o.kind, OtherKind::TopLevelCommand),
            _ => panic!(),
        }
    }
    assert!(s.unclassified.is_empty());
}

#[test]
fn menus_nested() {
    let src = "menu status,channel {\n\
      First:echo 1\n\
      Sub\n\
      .Child A:echo a\n\
      .Child B {\n\
        echo b1\n\
        echo b2\n\
      }\n\
      ..Deep:echo d\n\
      -\n\
      Last:{\n\
        echo l\n\
      }\n\
    }\n\
    menu nicklist\n{\n  Item:echo n\n}\n\
    menu @win { X:echo x }\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 3);
    let m = match &s.items[0] {
        Item::Menu(m) => m,
        _ => panic!(),
    };
    assert_eq!(m.windows, ["status", "channel"]);
    assert_eq!((m.line, m.end_line), (1, 14));
    let labels: Vec<_> = m.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["First", "Sub", "-", "Last"]);
    assert_eq!(m.items[0].kind, MenuItemKind::Action);
    let sub = &m.items[1];
    assert_eq!(sub.kind, MenuItemKind::Submenu);
    assert_eq!(sub.children.len(), 2);
    assert_eq!(sub.children[1].label, "Child B");
    assert_eq!(sub.children[1].body.len(), 2);
    assert_eq!(sub.children[1].children.len(), 1);
    assert_eq!(sub.children[1].children[0].label, "Deep");
    assert_eq!(m.items[2].kind, MenuItemKind::Separator);
    assert_eq!(names(&m.items[3].body), ["echo"]);
    match &s.items[1] {
        Item::Menu(m) => {
            assert_eq!(m.windows, ["nicklist"]);
            assert_eq!(m.items.len(), 1);
        }
        _ => panic!(),
    }
    match &s.items[2] {
        Item::Menu(m) => {
            assert_eq!(m.windows, ["@win"]);
            assert_eq!(m.items.len(), 1);
        }
        _ => panic!(),
    }
}

#[test]
fn dialogs_record_presence_only() {
    let src =
        "dialog box {\n  title \"x\"\n  size -1 -1 100 50\n  button \"ok\", 1, 1 1 10 10, ok\n}\n\
               dialog -l small {\n  title \"y\"\n}\n\
               dialog -m a b\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 3);
    match &s.items[0] {
        Item::Dialog(d) => assert_eq!(
            (d.name.as_str(), d.local, d.content_lines, d.end_line),
            ("box", false, 3, 5)
        ),
        _ => panic!(),
    }
    match &s.items[1] {
        Item::Dialog(d) => assert!(d.local && d.name == "small"),
        _ => panic!(),
    }
    // `dialog -m a b` is the command, not a definition.
    assert!(matches!(&s.items[2], Item::Other(o) if o.kind == OtherKind::TopLevelCommand));
}

#[test]
fn groups() {
    let src = "#grp1 on\nalias a echo 1\non *:TEXT:*:#:echo t\n#grp1 end\n#grp2 off\nalias b echo 2\n#grp2 end\nalias c echo 3\n";
    let s = parse_str(src);
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    assert_eq!(s.items.len(), 3);
    match &s.items[0] {
        Item::Group(g) => {
            assert_eq!(
                (g.name.as_str(), g.enabled, g.items.len()),
                ("#grp1", true, 2)
            );
            assert_eq!((g.line, g.end_line), (1, Some(4)));
        }
        _ => panic!(),
    }
    match &s.items[1] {
        Item::Group(g) => assert!(!g.enabled && g.items.len() == 1),
        _ => panic!(),
    }
    assert!(matches!(&s.items[2], Item::Alias(_)));
}

#[test]
fn group_errors_are_reported() {
    let s = parse_str("#a on\n#b on\nalias x echo 1\n#c end\n");
    let kinds: Vec<_> = s.unclassified.iter().map(|u| u.kind).collect();
    assert_eq!(
        kinds,
        [
            UnclassifiedKind::NestedGroup,
            UnclassifiedKind::UnmatchedGroupEnd,
            UnclassifiedKind::UnterminatedGroup
        ]
    );
    match &s.items[0] {
        Item::Group(g) => assert_eq!(g.items.len(), 1),
        _ => panic!(),
    }
}

#[test]
fn top_level_commands_and_ini_lines() {
    let s =
        parse_str("server -m example.invalid:+6697\n/join #x\n[script]\nn0=alias foo echo hi\n");
    assert!(s.unclassified.is_empty(), "{:?}", s.unclassified);
    let kinds: Vec<_> = s
        .items
        .iter()
        .map(|i| match i {
            Item::Other(o) => o.kind,
            _ => panic!(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            OtherKind::TopLevelCommand,
            OtherKind::TopLevelCommand,
            OtherKind::IniSection,
            OtherKind::IniEntry
        ]
    );
    match &s.items[0] {
        Item::Other(o) => assert_eq!(
            (o.statements[0].name.as_str(), o.statements[0].args.as_str()),
            ("server", "-m example.invalid:+6697")
        ),
        _ => panic!(),
    }
}

#[test]
fn unclassified_constructs() {
    let s = parse_str("}\nalias a {\n echo 1\n");
    let kinds: Vec<_> = s.unclassified.iter().map(|u| (u.line, u.kind)).collect();
    assert!(kinds.contains(&(1, UnclassifiedKind::UnbalancedClose)));
    assert!(kinds.contains(&(2, UnclassifiedKind::UnterminatedBlock)));
    let s2 = parse_str("?? weird\nalias b echo }\n");
    let kinds2: Vec<_> = s2.unclassified.iter().map(|u| u.kind).collect();
    assert_eq!(
        kinds2,
        [
            UnclassifiedKind::UnknownTopLevel,
            UnclassifiedKind::UnbalancedClose
        ]
    );
    let s3 = parse_str("on *:TEXT\nraw 1:x\nctcp 1:x\nalias\nmenu status {\n a:echo 1\n");
    assert!(s3
        .unclassified
        .iter()
        .any(|u| u.kind == UnclassifiedKind::MalformedAliasHeader));
    assert!(s3
        .unclassified
        .iter()
        .any(|u| u.kind == UnclassifiedKind::MalformedRawHeader));
    assert!(s3
        .unclassified
        .iter()
        .any(|u| u.kind == UnclassifiedKind::MalformedCtcpHeader));
    assert!(s3
        .unclassified
        .iter()
        .any(|u| u.kind == UnclassifiedKind::UnterminatedMenu));
}

#[test]
fn encodings() {
    // Windows-1252: 0xE4 (a-umlaut) and 0x80 (euro) are invalid as UTF-8.
    let bytes: &[u8] = b"alias a echo \xE4\x80\n";
    let s = parse_bytes(bytes);
    assert_eq!(s.stats.encoding, Encoding::Windows1252);
    assert_eq!(alias(&s, 0).body[0].args, "\u{e4}\u{20ac}");
    // UTF-8 with BOM.
    let mut b = vec![0xEF, 0xBB, 0xBF];
    b.extend_from_slice("alias a echo \u{e4}\n".as_bytes());
    let s = parse_bytes(&b);
    assert_eq!(s.stats.encoding, Encoding::Utf8);
    assert_eq!(alias(&s, 0).name, "a");
    // UTF-16 LE with BOM.
    let mut u = vec![0xFF, 0xFE];
    for c in "alias a echo x\r\n".encode_utf16() {
        u.extend_from_slice(&c.to_le_bytes());
    }
    let s = parse_bytes(&u);
    assert_eq!(s.stats.encoding, Encoding::Utf16);
    assert_eq!(alias(&s, 0).name, "a");
}

#[test]
fn inventory_counts() {
    let src = "alias -l a {\n\
      echo -a $nick $1 $2- %v %%l &bin\n\
      msg # $iif($1,a,b) $+(x,y)\n\
      var %%l = $calc(1+1)\n\
      if ($1 == x) halt\n\
    }\n\
    on 1:TEXT:*x*:#:msg $chan $1-3\n\
    on *:TEXT:*y*:?:msg $nick $0\n\
    raw 001:*:echo hi\n\
    menu status { A:echo 1 }\n\
    ctcp 1:PING:*:halt\n";
    let s = parse_str(src);
    let inv = s.inventory();
    assert_eq!(inv.aliases, 1);
    assert_eq!(inv.local_aliases, 1);
    assert_eq!(inv.events, 2);
    assert_eq!(inv.raws, 1);
    assert_eq!(inv.ctcps, 1);
    assert_eq!(inv.menus, 1);
    assert_eq!(inv.menu_items, 1);
    assert_eq!(inv.commands["echo"], 3);
    assert_eq!(inv.commands["msg"], 3);
    assert_eq!(inv.commands["var"], 1);
    assert_eq!(inv.commands["halt"], 2);
    assert_eq!(inv.control["if"], 1);
    assert_eq!(inv.event_names["TEXT"], 2);
    assert_eq!(inv.event_levels["1"], 2);
    assert_eq!(inv.event_levels["*"], 1);
    assert_eq!(inv.raw_numerics["001"], 1);
    assert_eq!(inv.menu_windows["status"], 1);
    assert_eq!(inv.identifiers["nick"].total, 2);
    assert_eq!(inv.identifiers["iif"].with_args, 1);
    assert_eq!(inv.identifiers["+"].with_args, 1);
    assert_eq!(inv.identifiers["calc"].with_args, 1);
    assert_eq!(inv.positional["$1"], 3);
    assert_eq!(inv.positional["$2-"], 1);
    assert_eq!(inv.positional["$1-3"], 1);
    assert_eq!(inv.positional["$0"], 1);
    assert_eq!(inv.global_var_refs, 1);
    assert_eq!(inv.local_var_refs, 2);
    assert_eq!(inv.distinct_local_vars, 1);
    assert_eq!(inv.binvar_refs, 1);
    assert!(inv.unclassified.is_empty());
}

#[test]
fn inventory_level_names_are_not_leaked() {
    let s = parse_str("on boss:TEXT:*:#:echo x\n");
    let inv = s.inventory();
    assert_eq!(inv.event_levels.len(), 1);
    assert!(inv.event_levels.contains_key("<named>"));
}

#[test]
fn deep_nesting_is_bounded() {
    let mut src = String::from("alias a {\n");
    for _ in 0..5000 {
        src.push_str("if (x) {\n");
    }
    src.push_str("echo deep\n");
    for _ in 0..5000 {
        src.push_str("}\n");
    }
    src.push_str("}\nalias b echo ok\n");
    let s = parse_str(&src);
    assert!(s
        .unclassified
        .iter()
        .any(|u| u.kind == UnclassifiedKind::NestingTooDeep));
    assert_eq!(s.items.len(), 2);
    assert!(s.inventory().max_block_depth <= 64);

    let chain = format!("alias a {}echo x\n", "if (x) ".repeat(50_000));
    let s = parse_str(&chain);
    assert_eq!(s.items.len(), 1);
    let _ = s.inventory();

    let braces = format!("alias a {}\n", "{ ".repeat(100_000));
    let s = parse_str(&braces);
    let _ = s.inventory();

    let menu = format!("menu status {{\n{}x:echo a\n}}\n", ".".repeat(10_000));
    let s = parse_str(&menu);
    let _ = s.inventory();
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..400 {
        let len = (rng.next() % 2000) as usize;
        let bytes: Vec<u8> = (0..len)
            .map(|_| {
                if round % 2 == 0 {
                    rng.next() as u8
                } else {
                    b" \n{}|;$&()%:#.-/\r"[(rng.next() % 17) as usize]
                }
            })
            .collect();
        let s = parse_bytes(&bytes);
        let _ = s.inventory();
    }
}

#[test]
fn random_fragments_never_panic() {
    const FRAGS: &[&str] = &[
        "alias ",
        "alias -l ",
        "on *:TEXT:*:#:",
        "on 1:JOIN:#:",
        "raw 001:*:",
        "ctcp 1:V:*:",
        "menu status ",
        "dialog d ",
        "#g on\n",
        "#g off\n",
        "#g end\n",
        "{",
        "}",
        " { ",
        " } ",
        "\n",
        "\r\n",
        " | ",
        "||",
        "if (",
        ") ",
        "elseif (",
        "else ",
        "while (",
        "$iif(",
        "$+(",
        "$1-",
        "%v",
        "%%l",
        "&b",
        "$&\n",
        "; c\n",
        "/* ",
        " */\n",
        "echo ",
        "var ",
        "set ",
        ":lbl",
        "goto ",
        "return ",
        "\u{e9}",
        "'",
        "\"",
        ".",
        "-",
        ":",
        "x",
        " ",
    ];
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..600 {
        let n = (rng.next() % 120) as usize;
        let mut src = String::new();
        for _ in 0..n {
            src.push_str(FRAGS[(rng.next() % FRAGS.len() as u64) as usize]);
        }
        let s = parse_str(&src);
        let _ = s.inventory();
    }
}
