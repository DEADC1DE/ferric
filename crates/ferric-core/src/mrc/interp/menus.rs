//! Popup menus (plan M5): the `menu <window> { ... }` definitions of the loaded scripts.
//!
//! `Runtime::menu_nodes` lists what a popup shows for one window kind. Labels are
//! evaluated each time the popup is listed; an entry whose label is empty, or a
//! submenu without visible entries, is left out. Every command entry carries an
//! address. `Runtime::run_menu_item` runs one with the popup's nicks as `$1`, `$2`,
//! ...; its `$?` prompts take their answers from the caller, in order. An address
//! made before the menus changed is refused: the entries may have moved.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::eval::Exec;
use super::runtime::{RunResult, Runtime, ScriptContext};
use crate::mrc::{Menu, MenuItem, MenuItemKind, Statement};

/// Window kinds a popup menu can be defined for. Custom windows (`@name`) are not supported.
pub const WINDOW_KINDS: &[&str] = &["status", "channel", "query", "nicklist", "menubar"];

/// A menu of a loaded script, with the script it came from.
pub(crate) struct RegisteredMenu {
    pub(crate) script: String,
    pub(crate) menu: Arc<Menu>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// An entry that runs commands.
    Command,
    Separator,
    /// An entry with entries under it.
    Submenu,
}

/// Where a command entry is: the menu version it was listed in, the menu's index and the path
/// of item indices down to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuAddress {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub menu: usize,
    #[serde(default)]
    pub path: Vec<usize>,
    #[serde(default)]
    pub python_id: Option<String>,
}

/// One entry of a popup, as listed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MenuNode {
    pub kind: NodeKind,
    pub label: String,
    /// Set for command entries.
    pub address: Option<MenuAddress>,
    /// The questions of the entry's `$?` prompts, in order. A prompt without a question asks with the label.
    pub prompts: Vec<String>,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    fn separator() -> MenuNode {
        MenuNode {
            kind: NodeKind::Separator,
            label: String::new(),
            address: None,
            prompts: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// The entries of `items` that show, in order. Separators are kept only between entries.
fn nodes_of(
    exec: &mut Exec,
    items: &[MenuItem],
    version: u64,
    menu: usize,
    path: &mut Vec<usize>,
    args: &[String],
) -> Vec<MenuNode> {
    let mut out: Vec<MenuNode> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        path.push(index);
        let node = match item.kind {
            MenuItemKind::Separator => Some(MenuNode::separator()),
            MenuItemKind::Submenu => {
                let label = label_of(exec, &item.label, args);
                let children = nodes_of(exec, &item.children, version, menu, path, args);
                let has_entries = children.iter().any(|c| c.kind != NodeKind::Separator);
                (!label.is_empty() && has_entries).then(|| MenuNode {
                    kind: NodeKind::Submenu,
                    label,
                    address: None,
                    prompts: Vec::new(),
                    children,
                })
            }
            MenuItemKind::Action => {
                let label = label_of(exec, &item.label, args);
                (!label.is_empty()).then(|| {
                    let prompts = prompts_of(&item.body, &label);
                    MenuNode {
                        kind: NodeKind::Command,
                        address: Some(MenuAddress {
                            version,
                            menu,
                            path: path.clone(),
                            python_id: None,
                        }),
                        label,
                        prompts,
                        children: Vec::new(),
                    }
                })
            }
        };
        path.pop();
        if let Some(node) = node {
            out.push(node);
        }
    }
    tidy(out)
}

/// Drops separators at the start and the end and doubles between entries.
fn tidy(nodes: Vec<MenuNode>) -> Vec<MenuNode> {
    let mut out: Vec<MenuNode> = Vec::new();
    for node in nodes {
        let is_separator = node.kind == NodeKind::Separator;
        let previous_separator = out.last().map_or(true, |p| p.kind == NodeKind::Separator);
        if is_separator && previous_separator {
            continue;
        }
        out.push(node);
    }
    while out.last().is_some_and(|n| n.kind == NodeKind::Separator) {
        out.pop();
    }
    out
}

/// The evaluated label of an entry; empty when it cannot be evaluated, which hides the entry.
fn label_of(exec: &mut Exec, label: &str, args: &[String]) -> String {
    match exec.menu_text(label, args) {
        Ok(text) => text.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// The question of each `$?` in an entry's commands, in order; a prompt without a question
/// asks with the label.
fn prompts_of(body: &[Statement], label: &str) -> Vec<String> {
    let mut raw = String::new();
    collect_raw(body, &mut raw);
    crate::mrc::expr::prompts_in(&raw)
        .into_iter()
        .map(|question| question.unwrap_or_else(|| label.to_string()))
        .collect()
}

/// The raw text of statements, nested blocks included.
fn collect_raw(stmts: &[Statement], out: &mut String) {
    for st in stmts {
        out.push_str(&st.name);
        out.push(' ');
        out.push_str(&st.args);
        out.push('\n');
        collect_raw(&st.body, out);
    }
}

/// The item at a path of indices, down through the submenus.
fn item_at<'a>(items: &'a [MenuItem], path: &[usize]) -> Option<&'a MenuItem> {
    let (first, rest) = path.split_first()?;
    let item = items.get(*first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        item_at(&item.children, rest)
    }
}

impl Runtime {
    /// The entries of one popup, for a window kind (`status`, `channel`, `query`, `nicklist`
    /// or `menubar`), across all loaded scripts in load order. `args` are the popup's nicks,
    /// as `$1`, `$2`, ...
    pub fn menu_nodes(&mut self, window: &str, context: &ScriptContext, args: &[String]) -> Vec<MenuNode> {
        let version = self.menu_version;
        let menus: Vec<(usize, Arc<Menu>)> = self
            .menus
            .iter()
            .enumerate()
            // `menu *` applies to every window kind; it used to match no popup at all.
            .filter(|(_, reg)| reg.menu.windows.iter().any(|w| w == window || w == "*"))
            .map(|(index, reg)| (index, Arc::clone(&reg.menu)))
            .collect();
        let now = (self.clock)();
        let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
        let mut exec = Exec::new_with_context(scripts, globals, limits, rng, hashes, regex, timers, now, context.clone());
        let mut out = Vec::new();
        for (index, menu) in menus {
            let mut path = Vec::new();
            out.extend(nodes_of(&mut exec, &menu.items, version, index, &mut path, args));
        }
        out
    }

    /// Runs a command entry listed by `menu_nodes`. `answers` are the answers to its prompts, in
    /// order. An address from another menu version is refused.
    pub fn run_menu_item(
        &mut self,
        address: &MenuAddress,
        context: &ScriptContext,
        args: &[String],
        answers: Vec<String>,
    ) -> Result<RunResult, String> {
        if address.version != self.menu_version {
            return Err("the menu changed since it was opened; open it again".into());
        }
        let reg = self
            .menus
            .get(address.menu)
            .ok_or_else(|| "the menu is gone; open it again".to_string())?;
        let item = item_at(&reg.menu.items, &address.path)
            .filter(|item| item.kind == MenuItemKind::Action)
            .ok_or_else(|| "the menu entry is gone; open it again".to_string())?;
        let body = Arc::new(item.body.clone());
        let owner = reg.script.clone();
        let now = (self.clock)();
        let Runtime { scripts, globals, limits, rng, hashes, regex, timers, .. } = self;
        let mut exec = Exec::new_with_context(scripts, globals, limits, rng, hashes, regex, timers, now, context.clone());
        exec.owner = owner;
        exec.answers = answers;
        exec.run_top("menu", body, args.to_vec());
        let (effects, notices, halt_status) = exec.finish();
        Ok(RunResult {
            effects,
            notices,
            halt_status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mrc::interp::{Effect, IrcEffect};

    const NICKLIST: &str = "menu nicklist {\n\
        Who Is?:/whois $1\n\
        -\n\
        Modes\n\
        .Op:/mode # +o $1\n\
        .Deop:/mode # -o $1\n\
        Cookie { if ($1 == $me) echo self else echo other }\n\
        }\n";

    fn ctx() -> ScriptContext {
        ScriptContext::new().with_network("net").with_me("me").with_chan("#a")
    }

    fn labels(nodes: &[MenuNode]) -> Vec<String> {
        nodes.iter().map(|n| n.label.clone()).collect()
    }

    fn address_of(nodes: &[MenuNode], label: &str) -> MenuAddress {
        fn find<'a>(nodes: &'a [MenuNode], label: &str) -> Option<&'a MenuNode> {
            nodes.iter().find_map(|n| {
                if n.label == label {
                    Some(n)
                } else {
                    find(&n.children, label)
                }
            })
        }
        find(nodes, label).and_then(|n| n.address.clone()).expect("command entry")
    }

    fn texts(res: &RunResult) -> Vec<String> {
        res.effects.iter().map(|e| e.text().to_string()).collect()
    }

    #[test]
    fn the_nicklist_entries_list_with_their_submenu_and_separator() {
        let mut r = Runtime::new();
        assert!(r.load("m.mrc", NICKLIST).is_empty());
        let nodes = r.menu_nodes("nicklist", &ctx(), &["alice".into()]);
        assert_eq!(labels(&nodes), ["Who Is?", "", "Modes", "Cookie"]);
        assert_eq!(nodes[0].kind, NodeKind::Command);
        assert_eq!(nodes[1].kind, NodeKind::Separator);
        assert_eq!(nodes[2].kind, NodeKind::Submenu);
        assert_eq!(labels(&nodes[2].children), ["Op", "Deop"]);
    }

    #[test]
    fn op_runs_as_a_mode_line_on_the_channel_of_the_popup() {
        let mut r = Runtime::new();
        r.load("m.mrc", NICKLIST);
        let nodes = r.menu_nodes("nicklist", &ctx(), &["alice".into()]);
        let res = r
            .run_menu_item(&address_of(&nodes, "Op"), &ctx(), &["alice".into()], Vec::new())
            .unwrap();
        assert!(res.notices.is_empty(), "{:?}", res.notices);
        assert_eq!(
            res.effects,
            vec![Effect::Irc(IrcEffect::Mode { target: "#a".into(), modes: "+o alice".into() })]
        );
    }

    #[test]
    fn a_block_entry_takes_the_branch_of_its_nick() {
        let mut r = Runtime::new();
        r.load("m.mrc", NICKLIST);
        let nodes = r.menu_nodes("nicklist", &ctx(), &["me".into()]);
        let res = r.run_menu_item(&address_of(&nodes, "Cookie"), &ctx(), &["me".into()], Vec::new()).unwrap();
        assert_eq!(texts(&res), ["self"]);
        let res = r.run_menu_item(&address_of(&nodes, "Cookie"), &ctx(), &["alice".into()], Vec::new()).unwrap();
        assert_eq!(texts(&res), ["other"]);
    }

    #[test]
    fn windows_select_their_menus_and_empty_entries_stay_hidden() {
        let mut r = Runtime::new();
        r.load(
            "w.mrc",
            "menu channel { Chan item:/echo c }\nmenu status,channel { Both:/echo b }\nmenu status { Empty\n.Hidden\n}\nmenu status { Shown:/echo s }\n",
        );
        assert_eq!(labels(&r.menu_nodes("query", &ctx(), &[])), Vec::<String>::new());
        assert_eq!(labels(&r.menu_nodes("channel", &ctx(), &[])), ["Chan item", "Both"]);
        assert_eq!(labels(&r.menu_nodes("status", &ctx(), &[])), ["Both", "Shown"]);
    }

    #[test]
    fn a_menu_for_all_windows_shows_in_every_popup() {
        let mut r = Runtime::new();
        r.load("a.mrc", "menu * { .Addons\n..Stats:/echo s\n}\n");
        for window in ["status", "channel", "query", "nicklist"] {
            assert_eq!(labels(&r.menu_nodes(window, &ctx(), &[])), ["Addons"], "{window}");
        }
    }

    #[test]
    fn a_prompt_takes_the_answer_the_caller_gives() {
        let mut r = Runtime::new();
        r.load("p.mrc", "menu status {
Join?:/join $?=\"Channel to join:\"
Ask:/echo $?
}
");
        let nodes = r.menu_nodes("status", &ctx(), &[]);
        assert_eq!(nodes[0].prompts, ["Channel to join:"]);
        assert_eq!(nodes[1].prompts, ["Ask"]);
        let res = r
            .run_menu_item(&address_of(&nodes, "Join?"), &ctx(), &[], vec!["#x".into()])
            .unwrap();
        assert_eq!(res.effects, vec![Effect::Irc(IrcEffect::Join { channel: "#x".into(), key: None })]);
        // Without an answer the run stops with a notice.
        let res = r.run_menu_item(&address_of(&nodes, "Join?"), &ctx(), &[], Vec::new()).unwrap();
        assert!(res.effects.is_empty());
        assert_eq!(res.notices.len(), 1, "{:?}", res.notices);
    }

    #[test]
    fn an_address_from_before_a_reload_is_refused() {
        let mut r = Runtime::new();
        r.load("m.mrc", NICKLIST);
        let nodes = r.menu_nodes("nicklist", &ctx(), &["alice".into()]);
        let old = address_of(&nodes, "Who Is?");
        r.load("m.mrc", "menu nicklist { Other:/echo o }\n");
        let err = r.run_menu_item(&old, &ctx(), &["alice".into()], Vec::new()).unwrap_err();
        assert!(err.contains("open it again"), "{err}");
    }

    #[test]
    fn a_nick_with_separators_stays_data() {
        let mut r = Runtime::new();
        r.load("m.mrc", "menu nicklist { Msg:/msg $1 hi }\n");
        let nodes = r.menu_nodes("nicklist", &ctx(), &["x|quit".into()]);
        let res = r.run_menu_item(&address_of(&nodes, "Msg"), &ctx(), &["x|quit".into()], Vec::new()).unwrap();
        assert_eq!(
            res.effects,
            vec![Effect::Irc(IrcEffect::Msg { target: "x|quit".into(), text: "hi".into() })]
        );
    }
}
