//! Sends the IRC lines a script asked for, through the connection of the network the
//! script ran on.
//!
//! The interpreter only makes data (`Effect::Irc`). This module applies the per-network
//! send budget, sends each line the way the chat input does, and records the messages
//! we sent, so they show up in chat the same way as ones typed by hand.

use std::time::Instant;

use ferric_core::mrc::interp::{BroadcastKind, BroadcastScope, Effect, IrcEffect};
use ferric_core::{AppState, IrcClient};

use super::{record_own_message, AppSharedState, SentLine};

/// Lines a script may send per network: a burst of 10, then 4 a second (plan section 5).
const BURST: f64 = 10.0;
const PER_SECOND: f64 = 4.0;

/// Token bucket for the script sends of one network.
#[derive(Debug)]
pub(super) struct SendBudget {
    tokens: f64,
    last: Option<Instant>,
    /// True while lines are being dropped, so the notice is shown once per drop.
    refusing: bool,
}

/// What the budget decided for one line.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Admit {
    Sent,
    /// Dropped; `first` is true for the first drop of a run of drops.
    Dropped { first: bool },
}

impl SendBudget {
    pub(super) fn new() -> Self {
        SendBudget { tokens: BURST, last: None, refusing: false }
    }

    pub(super) fn admit(&mut self, now: Instant) -> Admit {
        let elapsed = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f64());
        self.last = Some(now);
        self.tokens = (self.tokens + elapsed * PER_SECOND).min(BURST);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.refusing = false;
            Admit::Sent
        } else {
            let first = !self.refusing;
            self.refusing = true;
            Admit::Dropped { first }
        }
    }
}

/// Sends the IRC effects of one trigger. `network` is the network the script ran on; a
/// broadcast reaches the windows its scope names. A failed or dropped line becomes one status
/// notice per network; the rest of the lines still go out.
pub(super) async fn send_effects(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    effects: &[Effect],
) {
    let work = expand(state, network, effects).await;
    let mut told_disconnected: Vec<String> = Vec::new();
    for (net, line) in work {
        if let IrcEffect::Query { nick, .. } = &line {
            let was_new = {
                let mut app_st = state.app_state.lock().await;
                super::open_query_buffer(&mut app_st, &net, nick)
            };
            if was_new == Some(true) {
                if let Some(app_handle) = app {
                    super::trigger_auto_dh1080(state, app_handle, &net, nick).await;
                }
            }
        }
        if matches!(line, IrcEffect::Query { text: None, .. }) {
            continue;
        }
        // A query window opens even when the network is not connected; only sends need the connection.
        let client = state.clients.lock().await.get(&net).cloned();
        let Some(client) = client else {
            if !told_disconnected.contains(&net) {
                status_notice(state, &net, "script: not connected; IRC lines dropped").await;
                told_disconnected.push(net);
            }
            continue;
        };
        let admit = {
            let mut budgets = state.script_budgets.lock().unwrap_or_else(|e| e.into_inner());
            budgets
                .entry(net.clone())
                .or_insert_with(SendBudget::new)
                .admit(Instant::now())
        };
        if let Admit::Dropped { first } = admit {
            if first {
                status_notice(
                    state,
                    &net,
                    "script sending too fast: further IRC lines dropped until it slows down",
                )
                .await;
            }
            continue;
        }
        if let Err(error) = send_one(state, app, &net, &client, &line).await {
            status_notice(state, &net, &format!("script send failed: {error}")).await;
        }
    }
}

/// The lines of one trigger, in order, each with the network it goes to. A broadcast becomes
/// one line per window, with the windows read from the state at this moment.
async fn expand(state: &AppSharedState, network: &str, effects: &[Effect]) -> Vec<(String, IrcEffect)> {
    let mut work = Vec::new();
    for effect in effects {
        match effect {
            Effect::Irc(IrcEffect::Broadcast { scope, kind, text }) => {
                let reached = {
                    let app_st = state.app_state.lock().await;
                    windows(&app_st, network, *scope)
                };
                for (net, window) in reached {
                    if let Some(line) = broadcast_line(&window, *kind, text.as_deref()) {
                        work.push((net, line));
                    }
                }
            }
            Effect::Irc(line) => work.push((network.to_string(), line.clone())),
            _ => {}
        }
    }
    work
}

/// The windows a broadcast reaches, as (network, window name), in a fixed order. Status
/// windows are never included.
fn windows(app_st: &AppState, network: &str, scope: BroadcastScope) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (net_id, net) in &app_st.networks {
        if scope != BroadcastScope::ChannelsEveryNetwork && net_id != network {
            continue;
        }
        for (key, buffer) in &net.buffers {
            if key == "status" {
                continue;
            }
            let wanted = match scope {
                BroadcastScope::Queries => !buffer.is_channel,
                _ => buffer.is_channel,
            };
            if wanted {
                out.push((net_id.clone(), buffer.name.clone()));
            }
        }
    }
    out.sort();
    out
}

/// The one line a broadcast sends to one window. `None` when the broadcast has no text to send.
fn broadcast_line(window: &str, kind: BroadcastKind, text: Option<&str>) -> Option<IrcEffect> {
    Some(match (kind, text) {
        (BroadcastKind::Msg, Some(text)) => IrcEffect::Msg { target: window.to_string(), text: text.to_string() },
        (BroadcastKind::Action, Some(text)) => IrcEffect::Action { target: window.to_string(), text: text.to_string() },
        (BroadcastKind::Part, reason) => IrcEffect::Part {
            channel: window.to_string(),
            reason: reason.map(str::to_string),
        },
        _ => return None,
    })
}

async fn send_one(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    client: &IrcClient,
    line: &IrcEffect,
) -> Result<(), String> {
    match line {
        IrcEffect::Msg { target, text } => {
            send_text(state, app, network, client, target, text).await?;
        }
        // A query with text sends it like a message; the window was opened by `send_effects`.
        IrcEffect::Query { nick, text: Some(text) } => {
            send_text(state, app, network, client, nick, text).await?;
        }
        IrcEffect::Query { text: None, .. } => {}
        // A notice leaves a local line `-> -target- text`.
        IrcEffect::Notice { target, text } => {
            client.send_notice(target, text).await?;
            if let Some(line) = notice_echo(target, text) {
                let window = target.trim_start_matches('@').to_ascii_lowercase();
                let mut app_st = state.app_state.lock().await;
                if !app_st.push_local_message(network, &window, line.clone()) {
                    app_st.push_local_message(network, "status", line);
                }
            }
        }
        IrcEffect::Action { target, text } => {
            let (_, is_encrypted) = client
                .send_privmsg(target, &format!("\x01ACTION {text}\x01"))
                .await?;
            let nick = client.current_nick();
            let sent = SentLine {
                network,
                target,
                nick: &nick,
                text: text.clone(),
                is_action: true,
                is_encrypted,
            };
            record_own_message(state, app, sent).await;
        }
        other => {
            let Some(raw) = other.line() else {
                return Err("internal: no line for this IRC effect".into());
            };
            client.send_raw(&raw)?;
        }
    }
    Ok(())
}

/// Sends one message and records it the way the input line does.
async fn send_text(
    state: &AppSharedState,
    app: Option<&tauri::AppHandle>,
    network: &str,
    client: &IrcClient,
    target: &str,
    text: &str,
) -> Result<(), String> {
    let (sent, is_encrypted) = client.send_privmsg(target, text).await?;
    let nick = client.current_nick();
    let sent = SentLine { network, target, nick: &nick, text: sent, is_action: false, is_encrypted };
    record_own_message(state, app, sent).await;
    Ok(())
}

/// The local line of a script notice, or `None` for a CTCP reply (`\x01...\x01`), which leaves none.
fn notice_echo(target: &str, text: &str) -> Option<String> {
    (!text.starts_with('\u{1}')).then(|| format!("-> -{target}- {text}"))
}

async fn status_notice(state: &AppSharedState, network: &str, text: &str) {
    let _ = state
        .app_state
        .lock()
        .await
        .push_local_message(network, "status", text.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_broadcast_reaches_the_windows_of_its_scope() {
        use ferric_core::IrcEvent;
        let message = |net: &str, target: &str| IrcEvent::Message {
            sender_address: None,
            network: net.into(),
            target: target.into(),
            sender: "alice".into(),
            text: "hi".into(),
            spans: Vec::new(),
            is_notice: false,
            is_action: false,
            is_encrypted: false,
            timestamp: 1,
            sender_host: None,
        };
        let mut app = AppState::new();
        app.handle_event(&message("a", "#one"));
        app.handle_event(&message("a", "#two"));
        app.handle_event(&message("a", "#chan-not-here"));
        app.handle_event(&message("b", "#three"));

        let names = |v: Vec<(String, String)>| -> Vec<String> {
            v.into_iter().map(|(n, w)| format!("{n}/{w}")).collect()
        };
        assert_eq!(names(windows(&app, "a", BroadcastScope::Channels)), ["a/#chan-not-here", "a/#one", "a/#two"]);
        assert_eq!(names(windows(&app, "a", BroadcastScope::ChannelsEveryNetwork)).len(), 4);
        assert!(windows(&app, "a", BroadcastScope::Queries).is_empty());
    }

    #[test]
    fn a_broadcast_line_is_one_message_or_part_per_window() {
        assert_eq!(
            broadcast_line("#c", BroadcastKind::Msg, Some("hi")),
            Some(IrcEffect::Msg { target: "#c".into(), text: "hi".into() })
        );
        assert_eq!(broadcast_line("#c", BroadcastKind::Msg, None), None);
        assert_eq!(
            broadcast_line("#c", BroadcastKind::Part, None),
            Some(IrcEffect::Part { channel: "#c".into(), reason: None })
        );
    }

    #[test]
    fn a_notice_leaves_a_local_line_but_a_ctcp_reply_does_not() {
        assert_eq!(notice_echo("alice", "hi"), Some("-> -alice- hi".to_string()));
        assert_eq!(notice_echo("alice", "\u{1}VERSION\u{1}"), None);
    }

    #[test]
    fn a_burst_of_ten_then_four_a_second() {
        let start = Instant::now();
        let mut budget = SendBudget::new();
        for _ in 0..10 {
            assert_eq!(budget.admit(start), Admit::Sent);
        }
        assert_eq!(budget.admit(start), Admit::Dropped { first: true });
        assert_eq!(budget.admit(start), Admit::Dropped { first: false });

        let later = start + Duration::from_secs(1);
        for _ in 0..4 {
            assert_eq!(budget.admit(later), Admit::Sent);
        }
        assert_eq!(budget.admit(later), Admit::Dropped { first: true });
    }

    #[test]
    fn the_budget_never_grows_past_the_burst() {
        let start = Instant::now();
        let mut budget = SendBudget::new();
        let much_later = start + Duration::from_secs(3600);
        for _ in 0..10 {
            assert_eq!(budget.admit(much_later), Admit::Sent);
        }
        assert_eq!(budget.admit(much_later), Admit::Dropped { first: true });
    }
}
