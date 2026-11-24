//! Maps incoming IRC events to the script events the runtime dispatches.
//!
//! One IRC event can give several script events. A MODE line gives one on MODE
//! and one event for each operator, voice, halfop or ban change in it.

use crate::connection::IrcEvent;
use crate::text::is_channel_name;

use super::runtime::ScriptEvent;

/// The script events for one incoming IRC event; empty when no handler sees it.
///
/// `own_nick` is the local nick: its own messages fire no event. Server lines (sender
/// `*` or target `status`) fire no message event either, and a notice without a user's
/// address is a server notice.
pub fn script_events(event: &IrcEvent, own_nick: &str) -> Vec<ScriptEvent> {
    let me = own_nick.to_string();
    match event {
        IrcEvent::Message {
            network,
            target,
            sender,
            text,
            is_notice,
            is_action,
            sender_address,
            ..
        } => {
            let server_line = sender == "*" || target.eq_ignore_ascii_case("status");
            let own = sender.eq_ignore_ascii_case(own_nick);
            if server_line || own || sender.is_empty() {
                return Vec::new();
            }
            let address = sender_address.clone();
            if *is_action {
                vec![ScriptEvent::Action {
                    nick: sender.clone(),
                    target: target.clone(),
                    text: text.clone(),
                    address,
                    network: network.clone(),
                    me,
                }]
            } else if *is_notice {
                if sender_address.is_none() {
                    return Vec::new();
                }
                match ctcp_frame(text) {
                    Some(inner) => vec![ScriptEvent::CtcpReply {
                        nick: sender.clone(),
                        text: inner,
                        address,
                        network: network.clone(),
                        me,
                    }],
                    None => vec![ScriptEvent::Notice {
                        nick: sender.clone(),
                        target: target.clone(),
                        text: text.clone(),
                        address,
                        network: network.clone(),
                        me,
                    }],
                }
            } else {
                vec![ScriptEvent::Text {
                    nick: sender.clone(),
                    target: target.clone(),
                    text: text.clone(),
                    address,
                    network: network.clone(),
                    me,
                }]
            }
        }
        IrcEvent::Join {
            network,
            channel,
            nick,
            address,
            ..
        } => vec![ScriptEvent::Join {
            nick: nick.clone(),
            channel: channel.clone(),
            address: address.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Part {
            network,
            channel,
            nick,
            message,
            address,
            ..
        } => vec![ScriptEvent::Part {
            nick: nick.clone(),
            channel: channel.clone(),
            message: message.clone(),
            address: address.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Quit {
            network,
            nick,
            message,
            address,
            ..
        } => vec![ScriptEvent::Quit {
            nick: nick.clone(),
            message: message.clone(),
            address: address.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::NickChange {
            network,
            old_nick,
            new_nick,
            address,
            ..
        } => vec![ScriptEvent::Nick {
            nick: old_nick.clone(),
            new_nick: new_nick.clone(),
            address: address.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Connected { network, .. } => vec![ScriptEvent::Connect {
            network: network.clone(),
            me,
        }],
        IrcEvent::Disconnected { network, reason } => vec![ScriptEvent::Disconnect {
            reason: reason.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Error { network, message } => vec![ScriptEvent::Error {
            message: message.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Kick {
            network,
            channel,
            nick,
            by,
            reason,
        } => vec![ScriptEvent::Kick {
            nick: by.clone(),
            knick: nick.clone(),
            channel: channel.clone(),
            reason: reason.clone(),
            network: network.clone(),
            me,
        }],
        // The server's 332 reply has no setter; only a TOPIC line is a change.
        IrcEvent::Topic {
            network,
            channel,
            topic,
            set_by: Some(setter),
        } => vec![ScriptEvent::Topic {
            nick: setter.clone(),
            channel: channel.clone(),
            text: topic.clone(),
            network: network.clone(),
            me,
        }],
        IrcEvent::Mode {
            network,
            target,
            modes,
            params,
            by,
        } => {
            let setter = by.clone().unwrap_or_default();
            let text = if params.is_empty() {
                modes.clone()
            } else {
                format!("{} {}", modes, params.join(" "))
            };
            if is_channel_name(target) {
                let mut out = vec![ScriptEvent::Mode {
                    nick: setter.clone(),
                    channel: target.clone(),
                    modes: text,
                    network: network.clone(),
                    me: me.clone(),
                }];
                for (event, param) in mode_changes(modes, params) {
                    out.push(ScriptEvent::ModeChange {
                        event,
                        nick: setter.clone(),
                        channel: target.clone(),
                        param,
                        network: network.clone(),
                        me: me.clone(),
                    });
                }
                out
            } else if target.eq_ignore_ascii_case(own_nick) {
                vec![ScriptEvent::UserMode {
                    modes: text,
                    network: network.clone(),
                    me,
                }]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

/// The inside of a CTCP frame (`\x01VERSION\x01`), or `None` for text without one.
fn ctcp_frame(text: &str) -> Option<String> {
    let inner = text.strip_prefix('\x01')?.strip_suffix('\x01')?;
    Some(inner.to_string())
}

/// The changes of a MODE line that have their own events: operator, voice, halfop and ban
/// changes, as (event name, parameter). A mode letter takes a parameter when it is one of
/// the list modes or a key, and `l` only when it is set.
pub(crate) fn mode_changes(modes: &str, params: &[String]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut sign = '+';
    let mut next = params.iter();
    for c in modes.chars() {
        if c == '+' || c == '-' {
            sign = c;
            continue;
        }
        let takes = matches!(c, 'o' | 'v' | 'h' | 'b' | 'e' | 'I' | 'k' | 'q' | 'a')
            || (c == 'l' && sign == '+');
        let param = if takes { next.next().cloned() } else { None };
        let name = match (c, sign) {
            ('o', '+') => "op",
            ('o', _) => "deop",
            ('v', '+') => "voice",
            ('v', _) => "devoice",
            ('h', '+') => "help",
            ('h', _) => "dehelp",
            ('b', '+') => "ban",
            ('b', _) => "unban",
            _ => continue,
        };
        if let Some(param) = param {
            out.push((name, param));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mrc::interp::{EchoTarget, Effect, RunResult, Runtime, ScriptContext};

    fn message(sender: &str, target: &str, text: &str) -> IrcEvent {
        IrcEvent::Message {
            sender_address: None,
            network: "net".into(),
            target: target.into(),
            sender: sender.into(),
            text: text.into(),
            spans: Vec::new(),
            is_notice: false,
            is_action: false,
            is_encrypted: false,
            timestamp: 0,
            sender_host: None,
        }
    }

    fn with_flag(mut event: IrcEvent, notice: bool, action: bool, address: Option<&str>) -> IrcEvent {
        if let IrcEvent::Message { is_notice, is_action, sender_address, .. } = &mut event {
            *is_notice = notice;
            *is_action = action;
            *sender_address = address.map(str::to_string);
        }
        event
    }

    fn only(events: Vec<ScriptEvent>) -> ScriptEvent {
        assert_eq!(events.len(), 1, "{events:?}");
        events.into_iter().next().unwrap()
    }

    fn texts(rt: &mut Runtime, event: &ScriptEvent) -> Vec<String> {
        rt.dispatch_event(event, None)
            .effects
            .iter()
            .map(|e| e.text().to_string())
            .collect()
    }

    #[test]
    fn channel_message_maps_to_text() {
        assert_eq!(
            only(script_events(&message("alice", "#a", "hello"), "me")),
            ScriptEvent::Text {
                network: "net".into(),
                me: "me".into(),
                address: None,
                nick: "alice".into(),
                target: "#a".into(),
                text: "hello".into(),
            }
        );
    }

    #[test]
    fn query_message_maps_to_text_with_own_nick_as_target() {
        assert_eq!(
            only(script_events(&message("alice", "me", "hi"), "Me")),
            ScriptEvent::Text {
                network: "net".into(),
                me: "Me".into(),
                address: None,
                nick: "alice".into(),
                target: "me".into(),
                text: "hi".into(),
            }
        );
    }

    #[test]
    fn action_and_notice_and_ctcp_reply_map_to_their_own_events() {
        let action = with_flag(message("alice", "#a", "waves"), false, true, Some("a@h"));
        assert!(matches!(only(script_events(&action, "me")), ScriptEvent::Action { ref text, .. } if text == "waves"));

        let notice = with_flag(message("alice", "#a", "hi"), true, false, Some("a@h"));
        assert!(matches!(only(script_events(&notice, "me")), ScriptEvent::Notice { .. }));

        let reply = with_flag(message("alice", "me", "\u{1}VERSION Other 7\u{1}"), true, false, Some("a@h"));
        assert_eq!(
            only(script_events(&reply, "me")),
            ScriptEvent::CtcpReply {
                nick: "alice".into(),
                text: "VERSION Other 7".into(),
                address: Some("a@h".into()),
                network: "net".into(),
                me: "me".into(),
            }
        );
    }

    #[test]
    fn server_notice_and_own_lines_fire_no_message_event() {
        assert!(script_events(&message("ME", "#a", "hi"), "me").is_empty());
        assert!(script_events(&message("*", "#a", "hi"), "me").is_empty());
        assert!(script_events(&message("server.net", "status", "hi"), "me").is_empty());
        assert!(script_events(&message("", "#a", "hi"), "me").is_empty());
        // A notice without a user's address comes from the server.
        let server_notice = with_flag(message("irc.net", "me", "*** Looking up"), true, false, None);
        assert!(script_events(&server_notice, "me").is_empty());
        let own_action = with_flag(message("me", "#a", "waves"), false, true, None);
        assert!(script_events(&own_action, "me").is_empty());
    }

    #[test]
    fn join_part_quit_and_nick_map_one_to_one() {
        let join = IrcEvent::Join {
            address: None,
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            host: None,
        };
        assert_eq!(
            only(script_events(&join, "me")),
            ScriptEvent::Join { network: "net".into(), me: "me".into(), address: None, nick: "bob".into(), channel: "#a".into() }
        );

        let part = IrcEvent::Part {
            address: None,
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            message: Some("bye".into()),
        };
        assert_eq!(
            only(script_events(&part, "me")),
            ScriptEvent::Part {
                network: "net".into(),
                me: "me".into(),
                address: None,
                nick: "bob".into(),
                channel: "#a".into(),
                message: Some("bye".into()),
            }
        );

        let quit = IrcEvent::Quit { address: None, network: "net".into(), nick: "bob".into(), message: None };
        assert_eq!(
            only(script_events(&quit, "me")),
            ScriptEvent::Quit { network: "net".into(), me: "me".into(), address: None, nick: "bob".into(), message: None }
        );

        let nick = IrcEvent::NickChange {
            address: None,
            network: "net".into(),
            old_nick: "bob".into(),
            new_nick: "bobby".into(),
            host: None,
        };
        assert_eq!(
            only(script_events(&nick, "me")),
            ScriptEvent::Nick {
                network: "net".into(),
                me: "me".into(),
                address: None,
                nick: "bob".into(),
                new_nick: "bobby".into(),
            }
        );
    }

    #[test]
    fn address_reaches_the_script_event() {
        let join = IrcEvent::Join {
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            host: Some("h".into()),
            address: Some("bob@h".into()),
        };
        assert_eq!(
            only(script_events(&join, "me")),
            ScriptEvent::Join {
                network: "net".into(),
                me: "me".into(),
                nick: "bob".into(),
                channel: "#a".into(),
                address: Some("bob@h".into()),
            }
        );
    }

    #[test]
    fn connection_kick_topic_and_error_map() {
        let connected = IrcEvent::Connected { network: "net".into(), server: "irc".into() };
        assert_eq!(only(script_events(&connected, "me")), ScriptEvent::Connect { network: "net".into(), me: "me".into() });

        let kick = IrcEvent::Kick {
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            by: "op".into(),
            reason: Some("spam".into()),
        };
        assert_eq!(
            only(script_events(&kick, "me")),
            ScriptEvent::Kick {
                nick: "op".into(),
                knick: "bob".into(),
                channel: "#a".into(),
                reason: Some("spam".into()),
                network: "net".into(),
                me: "me".into(),
            }
        );

        // 332 has no setter and is a reply, not a change.
        let reply = IrcEvent::Topic { network: "net".into(), channel: "#a".into(), topic: "t".into(), set_by: None };
        assert!(script_events(&reply, "me").is_empty());
        let change = IrcEvent::Topic { network: "net".into(), channel: "#a".into(), topic: "new".into(), set_by: Some("op".into()) };
        assert!(matches!(only(script_events(&change, "me")), ScriptEvent::Topic { ref text, .. } if text == "new"));

        let error = IrcEvent::Error { network: "net".into(), message: "Closing Link".into() };
        assert!(matches!(only(script_events(&error, "me")), ScriptEvent::Error { ref message, .. } if message == "Closing Link"));
    }

    #[test]
    fn mode_line_gives_mode_and_one_event_per_change() {
        let mode = IrcEvent::Mode {
            network: "net".into(),
            target: "#a".into(),
            modes: "+ob-b".into(),
            params: vec!["alice".into(), "*!*@bad".into(), "*!*@worse".into()],
            by: Some("op".into()),
        };
        let events = script_events(&mode, "me");
        assert_eq!(events.len(), 4, "{events:?}");
        assert!(matches!(&events[0], ScriptEvent::Mode { modes, .. } if modes == "+ob-b alice *!*@bad *!*@worse"));
        assert!(matches!(&events[1], ScriptEvent::ModeChange { event: "op", param, .. } if param == "alice"));
        assert!(matches!(&events[2], ScriptEvent::ModeChange { event: "ban", param, .. } if param == "*!*@bad"));
        assert!(matches!(&events[3], ScriptEvent::ModeChange { event: "unban", param, .. } if param == "*!*@worse"));
    }

    #[test]
    fn own_user_modes_map_to_usermode() {
        let mode = IrcEvent::Mode {
            network: "net".into(),
            target: "me".into(),
            modes: "+i".into(),
            params: Vec::new(),
            by: None,
        };
        assert_eq!(
            only(script_events(&mode, "Me")),
            ScriptEvent::UserMode { modes: "+i".into(), network: "net".into(), me: "Me".into() }
        );
        let other = IrcEvent::Mode {
            network: "net".into(),
            target: "bob".into(),
            modes: "+i".into(),
            params: Vec::new(),
            by: None,
        };
        assert!(script_events(&other, "me").is_empty());
    }

    #[test]
    fn mode_changes_read_parameters_in_order() {
        let params: Vec<String> = ["a", "b", "c", "5"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            mode_changes("-o+lv", &params),
            vec![("deop", "a".to_string()), ("voice", "c".to_string())]
        );
        // `-l` takes no parameter, and `k` has no event: nothing comes out.
        assert_eq!(mode_changes("-l+k", &params), Vec::new());
        assert_eq!(mode_changes("+ol", &params), vec![("op", "a".to_string())]);
        assert_eq!(mode_changes("-b", &[]), Vec::<(&str, String)>::new());
    }

    #[test]
    fn other_events_are_not_mapped() {
        let names = IrcEvent::Names { network: "net".into(), channel: "#a".into(), names: Vec::new() };
        assert!(script_events(&names, "me").is_empty());
    }

    #[test]
    fn mapped_text_reaches_a_matching_handler_only() {
        let mut rt = Runtime::new();
        rt.load("s.mrc", "on *:TEXT:hello*:#:{ echo -a got it }");
        let hit = only(script_events(&message("alice", "#a", "hello world"), "me"));
        let res = rt.dispatch_event(&hit, None);
        assert_eq!(
            res.effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "got it".into(),
            }]
        );
        assert!(script_events(&message("me", "#a", "hello world"), "me").is_empty());
        let bye = only(script_events(&message("alice", "#a", "bye"), "me"));
        assert!(rt.dispatch_event(&bye, None).effects.is_empty());
    }

    /// `$` in the level makes the matchtext a regular expression: `m/body/switches`, PCRE switches.
    /// `S` keeps control codes in `$1-`.
    #[test]
    fn dollar_level_makes_the_matchtext_a_regex() {
        let mut rt = Runtime::new();
        rt.load("s.mrc", "on $*:TEXT:m/(.*SKIP.*)/iS:#demo:{ echo -a got $1- }");
        let hit = only(script_events(&message("DemoUser", "#demo", "please skip this"), "me"));
        assert_eq!(
            rt.dispatch_event(&hit, None).effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "got please skip this".into(),
            }]
        );
        let miss = only(script_events(&message("DemoUser", "#demo", "nothing here"), "me"));
        assert!(rt.dispatch_event(&miss, None).effects.is_empty());
    }

    /// Without `$` the same matchtext is a wildcard, so `m/.../` never matches a message.
    #[test]
    fn matchtext_regex_needs_the_dollar_level() {
        let mut rt = Runtime::new();
        rt.load("s.mrc", "on *:TEXT:m/(.*SKIP.*)/iS:#:{ echo -a got $1- }");
        let ev = only(script_events(&message("alice", "#a", "please skip this"), "me"));
        assert!(rt.dispatch_event(&ev, None).effects.is_empty());
    }

    /// With `S` the color code stays in `$1-`.
    #[test]
    fn s_switch_keeps_color_codes_in_dollar_one() {
        let mut rt = Runtime::new();
        rt.load("b.mrc", "on $*:TEXT:m/(.*)/iS:#:{ echo -a B: $1- }");
        let ev = only(script_events(&message("alice", "#a", "\u{3}04SKIP please"), "me"));
        assert_eq!(
            rt.dispatch_event(&ev, None).effects,
            vec![Effect::EchoTarget {
                target: EchoTarget::Active,
                text: "B: \u{3}04SKIP please".into(),
            }]
        );
    }

    /// The script from a user report, end to end: the message is relayed to `#demo`.
    #[test]
    fn skip_script_from_the_report_relays_the_message() {
        let mut rt = Runtime::new();
        rt.load(
            "skip.mrc",
            "on $*:TEXT:m/(.*SKIP.*)/iS:#demo:{\n  if ($nick == DemoUser) {\n    set %go 1\n    if (*@*ADDPRE* iswm $1-) { set %go 0 }\n    if (%go) {\n      /msg #demo 07Check SECTION $1-\n    }\n  }\n}\n",
        );
        let ev = only(script_events(&message("DemoUser", "#demo", "SKIP please check this"), "me"));
        assert_eq!(
            rt.dispatch_event(&ev, None).effects,
            vec![Effect::Irc(crate::mrc::interp::IrcEffect::Msg {
                target: "#demo".into(),
                text: "07Check SECTION SKIP please check this".into(),
            })]
        );
    }

    /// Scripts run in the order they were loaded, handlers in file order: the script loaded first runs first.
    #[test]
    fn handlers_run_in_load_order_then_file_order() {
        let mut rt = Runtime::new();
        rt.load("b.mrc", "on *:TEXT:*:#:{ echo b1 }\non *:TEXT:*:#:{ echo b2 }\n");
        rt.load("a.mrc", "on *:TEXT:*:#:{ echo a1 }\n");
        let ev = ScriptEvent::Text {
            network: "net".into(),
            me: "me".into(),
            address: None,
            nick: "alice".into(),
            target: "#a".into(),
            text: "hi".into(),
        };
        assert_eq!(texts(&mut rt, &ev), ["b1", "b2", "a1"]);
        assert_eq!(texts(&mut rt, &ev), ["b1", "b2", "a1"]);
    }

    #[test]
    fn load_and_unload_handlers_run_for_their_own_script_only() {
        let mut rt = Runtime::new();
        rt.load("l.mrc", "on *:LOAD:{ echo loaded }
on *:UNLOAD:{ echo bye }
on *:TEXT:*:#:{ echo no }");
        rt.load("other.mrc", "on *:LOAD:{ echo other }");
        let texts_of = |res: RunResult| -> Vec<String> { res.effects.iter().map(|e| e.text().to_string()).collect() };
        assert_eq!(texts_of(rt.dispatch_lifecycle("l.mrc", "load", &ScriptContext::default())), ["loaded"]);
        assert_eq!(texts_of(rt.dispatch_lifecycle("l.mrc", "unload", &ScriptContext::default())), ["bye"]);
        assert!(rt.dispatch_lifecycle("missing.mrc", "load", &ScriptContext::default()).effects.is_empty());
    }

    #[test]
    fn kick_and_op_handlers_see_their_nicks() {
        let mut rt = Runtime::new();
        rt.load(
            "k.mrc",
            "on *:KICK:#:{ echo $nick kicked $knick $1- | echo $chan }\non *:OP:#:{ echo $opnick got op by $nick }\non *:BAN:#:{ echo $bnick $banmask }\n",
        );
        let kick = IrcEvent::Kick {
            network: "net".into(),
            channel: "#a".into(),
            nick: "bob".into(),
            by: "op".into(),
            reason: Some("spam here".into()),
        };
        for ev in script_events(&kick, "me") {
            assert_eq!(texts(&mut rt, &ev), ["op kicked bob spam here", "#a"]);
        }
        let mode = IrcEvent::Mode {
            network: "net".into(),
            target: "#a".into(),
            modes: "+ob".into(),
            params: vec!["alice".into(), "bob!*@h".into()],
            by: Some("op".into()),
        };
        let events = script_events(&mode, "me");
        let op = events.iter().find(|e| matches!(e, ScriptEvent::ModeChange { event: "op", .. })).unwrap();
        assert_eq!(texts(&mut rt, op), ["alice got op by op"]);
        let ban = events.iter().find(|e| matches!(e, ScriptEvent::ModeChange { event: "ban", .. })).unwrap();
        assert_eq!(texts(&mut rt, ban), ["bob bob!*@h"]);
    }
}
