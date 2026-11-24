use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use crate::connection::event::IrcEvent;
use crate::protocol::{IrcMessage, StyledSpan};
use crate::text::{is_channel_name_with, strip_status_prefix};

/// Most entries kept per ban, exception or invite list of a channel.
const MAX_CHANNEL_LIST: usize = 1000;

/// Most channel buffers one network keeps. A server can name channels the user never joined,
/// by JOIN or PRIVMSG; past this many, such lines are not kept.
const MAX_CHANNEL_BUFFERS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing)]
    pub spans: Vec<StyledSpan>,
    #[serde(default)]
    pub is_notice: bool,
    #[serde(default)]
    pub is_action: bool,
    #[serde(default)]
    pub is_encrypted: bool,
    #[serde(default)]
    pub is_highlight: bool,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct ChannelUser {
    pub nick: String,
    pub prefix: Option<char>, // '~', '&', '@', '%', '+'
    #[serde(default)]
    pub prefix_modes: Vec<char>,
    #[serde(default)]
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BufferState {
    #[serde(default)]
    pub name: String, // channel name or nick or "status"
    #[serde(default)]
    pub is_channel: bool,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub users: BTreeMap<String, ChannelUser>,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub unread_count: u32,
    #[serde(default)]
    pub highlight_count: u32,
    /// Who set the current topic, when the server said.
    #[serde(default)]
    pub topic_set_by: Option<String>,
    /// Topic changes, oldest first. A change to the same text is not added twice.
    #[serde(default)]
    pub topic_history: Vec<TopicChange>,
    /// Mode letters without a parameter that are set on the channel, e.g. "nt".
    #[serde(default)]
    pub channel_flags: String,
    /// Channel key (+k) and user limit (+l), when set.
    #[serde(default)]
    pub channel_key: Option<String>,
    #[serde(default)]
    pub channel_limit: Option<String>,
    /// Ban list (+b) as the server reports it, plus changes seen in MODE lines.
    #[serde(default)]
    pub bans: Vec<BanEntry>,
    /// Except list (+e) and invite list (+I), kept the same way as the bans.
    #[serde(default)]
    pub excepts: Vec<BanEntry>,
    #[serde(default)]
    pub invites: Vec<BanEntry>,
    /// Only in UI snapshots: how `messages` relates to what the UI already holds. "full" replaces
    /// the UI's list, "append" adds to it, "same" keeps it. Empty elsewhere.
    #[serde(default)]
    pub messages_mode: String,
    /// Only in UI snapshots: "full" when the lists (users, topic history, bans, excepts and
    /// invites) are included; "same" when the UI keeps the ones it has. Empty elsewhere.
    #[serde(default)]
    pub lists_mode: String,
    /// Hash of the current lists (users, bans, etc.) sent to UI for O(1) change detection.
    #[serde(default)]
    pub lists_hash: u64,
    /// Cached hash on the backend to avoid rehashing unchanged user tables on every tick.
    #[serde(skip)]
    pub cached_lists_hash: std::cell::Cell<Option<u64>>,
}

/// One ban mask. `by` is empty when the server did not say who set it.
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct BanEntry {
    pub mask: String,
    pub by: String,
    pub at: u64,
}

/// One topic as it was set: who set it, when (Unix time in seconds), and the text.
#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct TopicChange {
    pub by: String,
    pub at: u64,
    pub topic: String,
}

/// A channel keeps at most this many topic changes; the oldest are dropped first.
pub const MAX_TOPIC_HISTORY: usize = 50;

/// A buffer keeps at most this many messages; the oldest are dropped first.
pub const MAX_BUFFER_MESSAGES: usize = 2000;

/// A network keeps at most this many query buffers.
pub const MAX_QUERY_BUFFERS: usize = 200;

/// The UI renders at most this many rows per buffer, so a state snapshot carries no more.
pub const MAX_UI_MESSAGES: usize = 1000;

/// Newest messages per buffer that a sync sends to the UI. Older history comes with
/// `buffer_history` when the buffer is opened, so buffers nobody is looking at stay small.
pub const UI_TAIL_MESSAGES: usize = 200;

impl BufferState {
    /// Copy for the UI: same fields, only the newest `MAX_UI_MESSAGES` messages, lists included.
    fn ui_copy(&self) -> BufferState {
        let skip = self.messages.len().saturating_sub(MAX_UI_MESSAGES);
        self.ui_copy_with(self.messages[skip..].to_vec(), "full", true)
    }

    /// Hash of the lists the UI copies. A sync sends the lists only when this changed.
    pub fn lists_hash(&self) -> u64 {
        if let Some(h) = self.cached_lists_hash.get() {
            return h;
        }
        let mut hasher = DefaultHasher::new();
        self.users.hash(&mut hasher);
        self.topic_history.hash(&mut hasher);
        self.bans.hash(&mut hasher);
        self.excepts.hash(&mut hasher);
        self.invites.hash(&mut hasher);
        let h = hasher.finish();
        self.cached_lists_hash.set(Some(h));
        h
    }

    /// Clears the cached lists hash when users, bans, excepts, invites or topic history change.
    pub fn invalidate_lists_hash(&self) {
        self.cached_lists_hash.set(None);
    }

    /// The buffer for the UI with the given messages. The lists are copied only when `with_lists`;
    /// otherwise they stay empty and `lists_mode` tells the UI to keep the ones it has.
    fn ui_copy_with(&self, messages: Vec<ChatMessage>, messages_mode: &str, with_lists: bool) -> BufferState {
        let mut copy = BufferState {
            name: self.name.clone(),
            is_channel: self.is_channel,
            topic: self.topic.clone(),
            users: BTreeMap::new(),
            messages,
            messages_mode: messages_mode.to_string(),
            lists_mode: String::from("same"),
            unread_count: self.unread_count,
            highlight_count: self.highlight_count,
            topic_set_by: self.topic_set_by.clone(),
            topic_history: Vec::new(),
            channel_flags: self.channel_flags.clone(),
            channel_key: self.channel_key.clone(),
            channel_limit: self.channel_limit.clone(),
            bans: Vec::new(),
            excepts: Vec::new(),
            invites: Vec::new(),
            lists_hash: self.lists_hash(),
            cached_lists_hash: std::cell::Cell::new(self.cached_lists_hash.get()),
        };
        if with_lists {
            copy.users = self.users.clone();
            copy.topic_history = self.topic_history.clone();
            copy.bans = self.bans.clone();
            copy.excepts = self.excepts.clone();
            copy.invites = self.invites.clone();
            copy.lists_mode = String::from("full");
        }
        copy
    }

    /// Appends a message and drops the oldest ones beyond `MAX_BUFFER_MESSAGES`.
    pub fn push_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        let excess = self.messages.len().saturating_sub(MAX_BUFFER_MESSAGES);
        if excess > 0 {
            self.messages.drain(..excess);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkState {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub nick: String,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub active_buffer: String,
    #[serde(default)]
    pub buffers: BTreeMap<String, BufferState>,
    /// The name shown for the network, when the server order gives one. Empty shows `id`.
    #[serde(default)]
    pub label: String,
    /// The place in the server order, which is also the startup order. None for networks the list does not name.
    #[serde(default)]
    pub order: Option<usize>,
    /// Whether the server is enabled in the server manager. A disabled server is not shown in the sidebar.
    #[serde(default = "network_enabled_default")]
    pub enabled: bool,
}

fn network_enabled_default() -> bool {
    true
}

impl NetworkState {
    /// Whether a channel buffer `key` may exist: it does already, or the network has room.
    fn has_room_for_channel(&self, key: &str) -> bool {
        self.buffers.contains_key(key)
            || self.buffers.values().filter(|b| b.is_channel).count() < MAX_CHANNEL_BUFFERS
    }

    /// Heals a stale `active_buffer` (e.g. after loading persisted state or
    /// evicting buffers): falls back to `status` when the named buffer is gone.
    pub fn heal_active_buffer(&mut self) {
        if self.active_buffer.is_empty() || !self.buffers.contains_key(&self.active_buffer) {
            self.active_buffer = "status".to_string();
        }
    }

    /// Call before creating the query buffer `new_key`: when the network already holds
    /// `MAX_QUERY_BUFFERS` queries, drops the one with the oldest last activity that is not the
    /// active buffer. Does nothing if `new_key` already exists.
    pub fn make_room_for_query(&mut self, new_key: &str) {
        if self.buffers.contains_key(new_key) {
            return;
        }
        let is_query = |key: &String, buf: &BufferState| !buf.is_channel && key.as_str() != "status";
        let query_count = self.buffers.iter().filter(|(k, b)| is_query(k, b)).count();
        let mut query_count = query_count;
        while query_count >= MAX_QUERY_BUFFERS {
            // Message ids only grow, so the last message id orders buffers by recent activity.
            let oldest = self
                .buffers
                .iter()
                .filter(|(k, b)| is_query(k, b) && **k != self.active_buffer)
                .min_by_key(|(_, b)| b.messages.last().map_or(0, |m| m.id))
                .map(|(k, _)| k.clone());
            match oldest {
                Some(key) => {
                    self.buffers.remove(&key);
                    query_count = query_count.saturating_sub(1);
                }
                None => break,
            }
        }
        self.heal_active_buffer();
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    pub networks: BTreeMap<String, NetworkState>,
    pub active_network: Option<String>,
    /// Per-network PREFIX pairs (mode letter, symbol), highest rank first, for the UI mode menu.
    /// The fallback pairs are used until the server sends a valid PREFIX; networks without any
    /// event yet have no entry. `default` keeps older serialized states loadable.
    #[serde(default)]
    pub network_prefix: BTreeMap<String, Vec<(char, char)>>,
    #[serde(default)]
    next_msg_id: u64,
    /// Per-network RPL_ISUPPORT data (PREFIX, CHANMODES); not sent to the UI.
    #[serde(skip)]
    isupport: BTreeMap<String, Isupport>,
    /// Masks received since the last end of each list, to drop the entries a listing no
    /// longer has. Not serialised.
    #[serde(skip)]
    list_seen: BTreeMap<(String, String, char), std::collections::BTreeSet<String>>,
    /// Per (network, buffer): the id of the newest message the UI was sent. Not serialised.
    #[serde(skip)]
    sent_to_ui: BTreeMap<(String, String), u64>,
    /// Per (network, buffer): the hash of the lists last sent to the UI. Not serialised.
    #[serde(skip)]
    sent_lists: BTreeMap<(String, String), u64>,
    /// Counts the states the UI has been offered: it moves when `ui_snapshot_delta` finds the state
    /// changed since its last call. The UI skips a sync whose revision it already applied.
    pub revision: u64,
    /// Digest of the state `revision` was last moved for. Not serialised.
    #[serde(skip)]
    digest: u64,
    /// Monotonic counter of mutations applied to state (events, UI commands).
    #[serde(skip)]
    pub events_counter: u64,
    /// Value of `events_counter` when `digest` was last refreshed.
    #[serde(skip)]
    pub last_digest_counter: u64,
}

/// Values longer than this (in characters) are ignored.
const MAX_ISUPPORT_VALUE: usize = 32;

/// Fallback used until the server sends a valid PREFIX token: (mode, symbol), highest rank first.
const DEFAULT_PREFIX: [(char, char); 5] = [('q', '~'), ('a', '&'), ('o', '@'), ('h', '%'), ('v', '+')];

/// Parsed RPL_ISUPPORT tokens for one network. `None` means "not sent yet, use the fallback".
#[derive(Default, Debug, Clone)]
struct Isupport {
    /// (mode letter, symbol) pairs from PREFIX=(modes)symbols, highest rank first.
    prefix: Option<Vec<(char, char)>>,
    /// CHANMODES groups A (list), B (always param), C (param when set), D (no param).
    chanmodes: Option<[String; 4]>,
    /// CHANTYPES prefixes. `None` means "not sent yet, use the default prefixes".
    chantypes: Option<String>,
}

impl Isupport {
    fn pairs(&self) -> &[(char, char)] {
        self.prefix.as_deref().unwrap_or(&DEFAULT_PREFIX)
    }

    fn mode_for_symbol(&self, symbol: char) -> Option<char> {
        self.pairs().iter().find(|(_, s)| *s == symbol).map(|(m, _)| *m)
    }

    fn symbol_for_mode(&self, mode: char) -> Option<char> {
        self.pairs().iter().find(|(m, _)| *m == mode).map(|(_, s)| *s)
    }

    /// Lower is higher rank.
    fn rank(&self, mode: char) -> Option<usize> {
        self.pairs().iter().position(|(m, _)| *m == mode)
    }

    fn mode_takes_param(&self, mode: char, adding: bool) -> bool {
        if self.symbol_for_mode(mode).is_some() {
            return true;
        }
        match &self.chanmodes {
            Some([a, b, c, _]) => a.contains(mode) || b.contains(mode) || (adding && c.contains(mode)),
            None => matches!(mode, 'b' | 'e' | 'I' | 'k') || (mode == 'l' && adding),
        }
    }

    /// Apply the tokens of one 005 line. Malformed or oversized values are ignored.
    fn apply_tokens<'a>(&mut self, tokens: impl Iterator<Item = &'a String>) {
        for token in tokens {
            let Some((key, value)) = token.split_once('=') else { continue };
            if value.chars().count() > MAX_ISUPPORT_VALUE {
                continue;
            }
            if key.eq_ignore_ascii_case("PREFIX") {
                if let Some(pairs) = parse_prefix_token(value) {
                    self.prefix = Some(pairs);
                }
            } else if key.eq_ignore_ascii_case("CHANMODES") {
                if let Some(groups) = parse_chanmodes_token(value) {
                    self.chanmodes = Some(groups);
                }
            } else if key.eq_ignore_ascii_case("CHANTYPES") {
                if !value.is_empty() && value.chars().all(|c| c.is_ascii_punctuation() && c != ',') {
                    self.chantypes = Some(value.to_string());
                }
            }
        }
    }
}

/// "(ov)@+" -> [('o','@'), ('v','+')]. Modes must be ASCII letters, symbols ASCII punctuation.
fn parse_prefix_token(value: &str) -> Option<Vec<(char, char)>> {
    let (modes, symbols) = value.strip_prefix('(')?.split_once(')')?;
    let modes: Vec<char> = modes.chars().collect();
    let symbols: Vec<char> = symbols.chars().collect();
    if modes.is_empty() || modes.len() != symbols.len() {
        return None;
    }
    let valid = modes.iter().all(char::is_ascii_alphabetic)
        && symbols.iter().all(|c| c.is_ascii_punctuation() && !matches!(c, ',' | ':' | '(' | ')' | '#'));
    let unique = |items: &[char]| items.iter().enumerate().all(|(i, c)| !items[..i].contains(c));
    if !valid || !unique(&modes) || !unique(&symbols) {
        return None;
    }
    Some(modes.into_iter().zip(symbols).collect())
}

/// "beI,k,l,imnpst" -> four groups of ASCII letters.
fn parse_chanmodes_token(value: &str) -> Option<[String; 4]> {
    let groups: Vec<&str> = value.split(',').collect();
    if groups.len() != 4 || !groups.iter().all(|g| g.chars().all(|c| c.is_ascii_alphabetic())) {
        return None;
    }
    Some([groups[0].to_string(), groups[1].to_string(), groups[2].to_string(), groups[3].to_string()])
}

impl AppState {
    pub fn mark_dirty(&mut self) {
        self.events_counter = self.events_counter.wrapping_add(1);
    }

    /// Append local output to an existing buffer without creating IRC events.
    pub fn push_local_message(&mut self, network: &str, buffer: &str, text: String) -> bool {
        // Buffers are keyed by their lowercase name, as in buffer_history.
        let key = buffer.to_lowercase();
        if !self.networks.get(network).is_some_and(|net| net.buffers.contains_key(&key)) {
            return false;
        }
        self.mark_dirty();
        let Some(buf) = self.networks.get_mut(network).and_then(|net| net.buffers.get_mut(&key)) else {
            return false;
        };
        self.next_msg_id += 1;
        buf.push_message(ChatMessage {
            id: self.next_msg_id,
            sender: "*".to_string(),
            spans: Vec::new(),
            text,
            is_notice: true,
            is_action: false,
            is_encrypted: false,
            is_highlight: false,
            timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        });
        true
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// State for the UI sync: like `clone`, but each buffer keeps only its newest
    /// `MAX_UI_MESSAGES` messages, so older ones are never copied or serialised.
    pub fn ui_snapshot(&self) -> AppState {
        let networks = self
            .networks
            .iter()
            .map(|(key, net)| {
                let buffers = net.buffers.iter().map(|(k, b)| (k.clone(), b.ui_copy())).collect();
                let copy = NetworkState {
                    id: net.id.clone(),
                    server: net.server.clone(),
                    nick: net.nick.clone(),
                    connected: net.connected,
                    active_buffer: net.active_buffer.clone(),
                    buffers,
                    label: net.label.clone(),
                    order: net.order,
                    enabled: net.enabled,
                };
                (key.clone(), copy)
            })
            .collect();
        AppState {
            networks,
            active_network: self.active_network.clone(),
            network_prefix: self.network_prefix.clone(),
            next_msg_id: self.next_msg_id,
            isupport: BTreeMap::new(), // not serialised
            list_seen: BTreeMap::new(),
            sent_to_ui: BTreeMap::new(),
            sent_lists: BTreeMap::new(),
            revision: self.revision,
            digest: 0,
            events_counter: self.events_counter,
            last_digest_counter: self.last_digest_counter,
        }
    }

    /// Like `ui_snapshot`, but each buffer carries only the messages the UI has not been sent yet,
    /// marked by `messages_mode`. `full` (or a buffer the UI has never seen) sends the newest
    /// `UI_TAIL_MESSAGES`. Only the one UI may call this: it records what was sent.
    pub fn ui_snapshot_delta(&mut self, full: bool) -> AppState {
        self.refresh_revision();
        self.build_snapshot(full)
    }

    /// Moves `revision` when the UI-visible state changed. Done once per sync.
    fn refresh_revision(&mut self) {
        let digest = self.ui_digest();
        self.last_digest_counter = self.events_counter;
        if digest != self.digest {
            self.digest = digest;
            self.revision += 1;
        }
    }

    /// The snapshot itself, without looking at the revision again.
    fn build_snapshot(&mut self, full: bool) -> AppState {
        self.prune_sent_to_ui();
        let mut updates: Vec<((String, String), Option<u64>, Option<u64>)> = Vec::new();
        let mut networks = BTreeMap::new();
        for (key, net) in &self.networks {
            let mut buffers = BTreeMap::new();
            for (name, buffer) in &net.buffers {
                let slot = (key.clone(), name.clone());
                let newest = buffer.messages.last().map(|m| m.id);
                let seen = if full { None } else { self.sent_to_ui.get(&slot).copied() };
                let (mode, messages) = match (seen, newest) {
                    (_, None) => ("full", Vec::new()),
                    (Some(seen_id), Some(newest_id)) if seen_id == newest_id => ("same", Vec::new()),
                    (Some(seen_id), Some(_)) => match buffer.messages.iter().position(|m| m.id == seen_id) {
                        Some(pos) => {
                            let fresh = &buffer.messages[pos + 1..];
                            let skip = fresh.len().saturating_sub(UI_TAIL_MESSAGES);
                            ("append", fresh[skip..].to_vec())
                        }
                        None => ("full", buffer.messages[buffer.messages.len().saturating_sub(UI_TAIL_MESSAGES)..].to_vec()),
                    },
                    (None, Some(_)) => ("full", buffer.messages[buffer.messages.len().saturating_sub(UI_TAIL_MESSAGES)..].to_vec()),
                };
                // A window's lists go to the UI whenever they change, shown or not, so a window the user
                // opens never shows stale ones. Each sync compares against the hash last sent.
                let lists = buffer.lists_hash();
                let with_lists = full || self.sent_lists.get(&slot) != Some(&lists);
                updates.push((slot, newest, if with_lists { Some(lists) } else { None }));
                buffers.insert(name.clone(), buffer.ui_copy_with(messages, mode, with_lists));
            }
            networks.insert(
                key.clone(),
                NetworkState {
                    id: net.id.clone(),
                    server: net.server.clone(),
                    nick: net.nick.clone(),
                    connected: net.connected,
                    active_buffer: net.active_buffer.clone(),
                    buffers,
                    label: net.label.clone(),
                    order: net.order,
                    enabled: net.enabled,
                },
            );
        }
        for (slot, newest, lists) in updates {
            // Recorded only when sent, so a window that was not shown still differs when it is shown.
            if let Some(lists) = lists {
                self.sent_lists.insert(slot.clone(), lists);
            }
            match newest {
                Some(id) => {
                    self.sent_to_ui.insert(slot, id);
                }
                None => {
                    self.sent_to_ui.remove(&slot);
                }
            }
        }
        AppState {
            networks,
            active_network: self.active_network.clone(),
            network_prefix: self.network_prefix.clone(),
            next_msg_id: self.next_msg_id,
            isupport: BTreeMap::new(),
            list_seen: BTreeMap::new(),
            sent_to_ui: BTreeMap::new(),
            sent_lists: BTreeMap::new(),
            revision: self.revision,
            digest: 0,
            events_counter: self.events_counter,
            last_digest_counter: self.last_digest_counter,
        }
    }

    /// The newest `limit` messages of one buffer (at most `MAX_BUFFER_MESSAGES`), oldest first.
    /// Used when a buffer is opened, so the UI can show its full scrollback.
    pub fn buffer_history(&self, network: &str, buffer: &str, limit: usize) -> Vec<ChatMessage> {
        let Some(net) = self.networks.get(network) else { return Vec::new() };
        let Some(buf) = net.buffers.get(&buffer.to_lowercase()) else { return Vec::new() };
        let keep = limit.min(MAX_BUFFER_MESSAGES);
        buf.messages[buf.messages.len().saturating_sub(keep)..].to_vec()
    }

    /// `/clear`: drops one buffer's scrollback, so `buffer_history` and the UI sync no longer hold
    /// those lines. Returns false when the buffer does not exist.
    pub fn clear_buffer_messages(&mut self, network: &str, buffer: &str) -> bool {
        let key = buffer.to_lowercase();
        if !self.networks.get(network).is_some_and(|net| net.buffers.contains_key(&key)) {
            return false;
        }
        self.mark_dirty();
        let Some(buf) = self.networks.get_mut(network).and_then(|net| net.buffers.get_mut(&key)) else {
            return false;
        };
        buf.messages.clear();
        true
    }

    /// The snapshot for the UI, or `None` when nothing has changed since the revision the UI last
    /// applied (`seen`). A `full` request is always answered. An unchanged state builds and
    /// serialises no copy, which is where the cost of a sync was.
    pub fn ui_snapshot_if_changed(&mut self, full: bool, seen: Option<u64>) -> Option<AppState> {
        if !full && seen == Some(self.revision) && self.events_counter == self.last_digest_counter {
            return None;
        }
        self.refresh_revision();
        if !full && seen == Some(self.revision) {
            return None;
        }
        Some(self.build_snapshot(full))
    }

    /// Hash of everything a UI snapshot carries: network and buffer fields, message ids and the list
    /// hashes. The same digest means the UI has nothing new, so it moves no revision.
    fn ui_digest(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.active_network.hash(&mut h);
        self.network_prefix.hash(&mut h);
        for (key, net) in &self.networks {
            key.hash(&mut h);
            net.id.hash(&mut h);
            net.server.hash(&mut h);
            net.nick.hash(&mut h);
            net.connected.hash(&mut h);
            net.active_buffer.hash(&mut h);
            for (name, buffer) in &net.buffers {
                name.hash(&mut h);
                buffer.is_channel.hash(&mut h);
                buffer.topic.hash(&mut h);
                buffer.topic_set_by.hash(&mut h);
                buffer.channel_flags.hash(&mut h);
                buffer.channel_key.hash(&mut h);
                buffer.channel_limit.hash(&mut h);
                buffer.unread_count.hash(&mut h);
                buffer.highlight_count.hash(&mut h);
                buffer.messages.len().hash(&mut h);
                buffer.messages.last().map(|m| m.id).hash(&mut h);
                // Every window's lists reach the UI when they change, so they move the revision too.
                buffer.lists_hash().hash(&mut h);
            }
        }
        h.finish()
    }

    /// Forgets what was sent to the UI for buffers that no longer exist. Closing a query or a
    /// buffer drops it from `networks`, and this keeps `sent_to_ui` and `sent_lists` in step.
    fn prune_sent_to_ui(&mut self) {
        let networks = &self.networks;
        let live = |slot: &(String, String)| {
            networks.get(&slot.0).is_some_and(|n| n.buffers.contains_key(&slot.1))
        };
        self.sent_to_ui.retain(|slot, _| live(slot));
        self.sent_lists.retain(|slot, _| live(slot));
    }

    /// Forget a network and everything kept for it. If it was the active one, another network
    /// (or none) becomes active. Returns whether the network existed.
    pub fn remove_network(&mut self, network: &str) -> bool {
        let existed = self.networks.contains_key(network);
        if !existed {
            return false;
        }
        self.mark_dirty();
        self.isupport.remove(network);
        self.network_prefix.remove(network);
        self.sent_to_ui.retain(|(net, _), _| net != network);
        self.sent_lists.retain(|(net, _), _| net != network);
        self.list_seen.retain(|(net, _, _), _| net != network);
        self.networks.remove(network);
        if self.active_network.as_deref() == Some(network) {
            self.active_network = self.networks.keys().next().cloned();
        }
        true
    }

    fn isupport_for(&self, network: &str) -> Isupport {
        self.isupport.get(network).cloned().unwrap_or_default()
    }

    /// Copy the effective PREFIX pairs (server value or fallback) into the serialized field.
    fn sync_prefix(&mut self, network: &str) {
        let pairs = self.isupport_for(network).pairs().to_vec();
        self.network_prefix.insert(network.to_string(), pairs);
    }

    fn ensure_network<'a>(&'a mut self, network: &str) -> &'a mut NetworkState {
        let net = self.networks.entry(network.to_string()).or_insert_with(|| {
            let mut buffers = BTreeMap::new();
            buffers.insert("status".to_string(), BufferState {
                name: "status".to_string(),
                is_channel: false,
                topic: Some("Network status".to_string()),
                ..Default::default()
            });
            NetworkState {
                id: network.to_string(),
                server: String::new(),
                nick: "FerricUser".to_string(),
                connected: false,
                active_buffer: "status".to_string(),
                buffers,
                label: String::new(),
                order: None,
                enabled: true,
            }
        });
        net.heal_active_buffer();
        net
    }

    /// Repairs stale `active_buffer` values in loaded state: every network whose
    /// active buffer no longer exists falls back to `status`.
    pub fn heal_active_buffers(&mut self) {
        for net in self.networks.values_mut() {
            net.heal_active_buffer();
        }
    }

    pub fn handle_event(&mut self, event: &IrcEvent) {
        self.mark_dirty();
        match event {
            IrcEvent::Raw { network, raw } => {
                // RPL_WELCOME: the first parameter is the nick the server gave us.
                if raw.contains(" 001 ") {
                    if let Some(msg) = IrcMessage::parse(raw) {
                        if msg.command == "001" && matches!(&msg.prefix, Some(p) if p.user.is_none()) {
                            if let Some(nick) = msg.params.first().filter(|n| !n.is_empty() && *n != "*") {
                                self.ensure_network(network).nick = nick.clone();
                            }
                        }
                    }
                }
                // RPL_ISUPPORT: only from a server source (no user part), never from a user.
                if raw.contains(" 005 ") {
                    if let Some(msg) = IrcMessage::parse(raw) {
                        if msg.command == "005" && matches!(&msg.prefix, Some(p) if p.user.is_none()) {
                            let isupport = self.isupport.entry(network.clone()).or_default();
                            isupport.apply_tokens(msg.params.iter().skip(1).filter(|t| !t.contains(' ')));
                            self.sync_prefix(network);
                        }
                    }
                }
            }
            IrcEvent::Connected { network, server } => {
                // New registration: forget what the previous connection announced.
                self.isupport.remove(network);
                self.sync_prefix(network);
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                net.connected = true;
                net.server = server.clone();

                let status_buf = net.buffers.entry("status".to_string()).or_insert_with(|| BufferState {
                    name: "status".to_string(),
                    is_channel: false,
                    topic: Some(format!("Connected to {}", server)),
                    ..Default::default()
                });
                status_buf.push_message(ChatMessage {
                    id: next_id,
                    sender: "*".to_string(),
                    text: format!("*** Connected to {} ({})", network, server),
                    spans: Vec::new(),
                    is_notice: true,
                    is_action: false,
                    is_encrypted: false,
                    is_highlight: false,
                    timestamp: now_millis(),
                });
            }
            IrcEvent::Disconnected { network, reason } => {
                self.list_seen.retain(|(net, _, _), _| net != network);
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                net.connected = false;
                // The user lists are stale now; NAMES after the next connect rebuilds them.
                // Without this, nicks that left while offline would stay for good.
                for buf in net.buffers.values_mut() {
                    buf.users.clear();
                    buf.invalidate_lists_hash();
                }

                if let Some(status_buf) = net.buffers.get_mut("status") {
                    status_buf.push_message(ChatMessage {
                        id: next_id,
                        sender: "*".to_string(),
                        text: format!("*** Disconnected: {}", reason),
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        is_highlight: false,
                        timestamp: now_millis(),
                    });
                }
            }
            IrcEvent::Error { network, message } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                let status_buf = net.buffers.entry("status".to_string()).or_insert_with(|| BufferState {
                    name: "status".to_string(),
                    is_channel: false,
                    ..Default::default()
                });
                status_buf.push_message(ChatMessage {
                    id: next_id,
                    sender: "*".to_string(),
                    text: format!("*** Error: {}", message),
                    spans: Vec::new(),
                    is_notice: true,
                    is_action: false,
                    is_encrypted: false,
                    is_highlight: false,
                    timestamp: now_millis(),
                });
            }
            IrcEvent::Message {
                network,
                target,
                sender,
                text,
                spans: _,
                is_notice,
                is_action,
                is_encrypted,
                timestamp,
                sender_host,
                ..
            } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let chantypes = self.isupport.get(network).and_then(|i| i.chantypes.clone());
                // A message to a status prefix (`@#chan`) belongs to the channel, not to a query named `@#chan`.
                let stripped = strip_status_prefix(target, chantypes.as_deref()).to_string();
                let target = &stripped;
                // Ensure the network exists so the default nick is present for routing.
                if !self.networks.contains_key(network) {
                    self.ensure_network(network);
                }
                let nick_snapshot = self.networks.get(network).map(|n| n.nick.clone());
                let nick_for_key = nick_snapshot.as_deref().unwrap_or("");

                let is_channel = is_channel_name_with(target, chantypes.as_deref());
                let (buf_key, display_name) = if is_channel {
                    (target.to_lowercase(), target.clone())
                } else if target == "*" || target.eq_ignore_ascii_case("status") || target.eq_ignore_ascii_case("auth") || sender.is_empty() {
                    ("status".to_string(), "status".to_string())
                } else if *is_notice && target.eq_ignore_ascii_case(nick_for_key) {
                    // A private notice (service or server) belongs to status, not to a query.
                    ("status".to_string(), "status".to_string())
                } else if !nick_for_key.is_empty() && target.eq_ignore_ascii_case(nick_for_key) {
                    // Incoming private message from sender to us
                    (sender.to_lowercase(), sender.clone())
                } else {
                    // Outgoing message to target
                    (target.to_lowercase(), target.clone())
                };

                // A server can name channels the user never joined; past the cap they are not kept.
                if is_channel {
                    let capped = self
                        .networks
                        .get(network)
                        .map(|n| !n.has_room_for_channel(&buf_key))
                        .unwrap_or(false);
                    if capped {
                        self.push_local_message(
                            network,
                            "status",
                            format!("Not tracking {target}: this network already has {MAX_CHANNEL_BUFFERS} channels open"),
                        );
                        return;
                    }
                }
                let net = self.ensure_network(network);
                // Our own lines never highlight, and a nick only highlights as a whole word.
                let is_highlight = !sender.eq_ignore_ascii_case(&net.nick) && mentions_nick(text, &net.nick);
                if !is_channel && buf_key != "status" {
                    net.make_room_for_query(&buf_key);
                }

                let buffer = net.buffers.entry(buf_key.clone()).or_insert_with(|| BufferState {
                    name: display_name.clone(),
                    is_channel,
                    topic: if is_channel { None } else { Some(format!("Private conversation with {}", display_name)) },
                    ..Default::default()
                });

                buffer.push_message(ChatMessage {
                    id: next_id,
                    sender: sender.clone(),
                    text: text.clone(),
                    spans: Vec::new(),
                    is_notice: *is_notice,
                    is_action: *is_action,
                    is_encrypted: *is_encrypted,
                    is_highlight,
                    timestamp: *timestamp,
                });

                if net.active_buffer != buf_key {
                    buffer.unread_count += 1;
                    if is_highlight {
                        buffer.highlight_count += 1;
                    }
                }

                if let Some(host) = sender_host {
                    if !sender.is_empty() && sender != "*" {
                        let key = sender.to_lowercase();
                        for buf in net.buffers.values_mut() {
                            if let Some(user) = buf.users.get_mut(&key) {
                                if user.host.as_deref() != Some(host.as_str()) {
                                    user.host = Some(host.clone());
                                    buf.invalidate_lists_hash();
                                }
                            }
                        }
                    }
                }
            }
            IrcEvent::Join { network, channel, nick, host, .. } => {
                let chan_key = channel.to_lowercase();
                if !self.networks.get(network).map_or(true, |n| n.has_room_for_channel(&chan_key)) {
                    let text = format!(
                        "Not tracking {channel}: this network already has {MAX_CHANNEL_BUFFERS} channels open"
                    );
                    self.push_local_message(network, "status", text);
                    return;
                }
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                let buffer = net.buffers.entry(chan_key.clone()).or_insert_with(|| BufferState {
                    name: channel.clone(),
                    is_channel: true,
                    ..Default::default()
                });

                let (prefix, pure_nick) = parse_user_prefix(&isupport, nick);
                let user_key = pure_nick.to_lowercase();
                let host = host.clone().or_else(|| buffer.users.get(&user_key).and_then(|u| u.host.clone()));
                buffer.users.insert(
                    user_key,
                    ChannelUser {
                        nick: pure_nick.clone(),
                        prefix,
                        prefix_modes: prefix.and_then(|s| mode_for_prefix(&isupport, s)).into_iter().collect(),
                        host,
                    },
                );
                buffer.invalidate_lists_hash();

                // If user is self and currently on status, switch active buffer to channel
                if pure_nick.eq_ignore_ascii_case(&net.nick) && net.active_buffer == "status" {
                    net.active_buffer = chan_key;
                }

                buffer.push_message(ChatMessage {
                    id: next_id,
                    sender: "*".to_string(),
                    text: format!("{} has joined {}", pure_nick, channel),
                    spans: Vec::new(),
                    is_notice: true,
                    is_action: false,
                    is_encrypted: false,
                    is_highlight: false,
                    timestamp: now_millis(),
                });
            }
            IrcEvent::Part { network, channel, nick, message, .. } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                let chan_key = channel.to_lowercase();
                if let Some(buffer) = net.buffers.get_mut(&chan_key) {
                    let (_, pure_nick) = parse_user_prefix(&isupport, nick);
                    if pure_nick.eq_ignore_ascii_case(&net.nick) {
                        // We left: nobody in the channel is known any more.
                        buffer.users.clear();
                    } else {
                        buffer.users.remove(&pure_nick.to_lowercase());
                    }
                    buffer.invalidate_lists_hash();

                    let reason = message.as_ref().map(|m| format!(" ({})", m)).unwrap_or_default();
                    buffer.push_message(ChatMessage {
                        id: next_id,
                        sender: "*".to_string(),
                        text: format!("{} has left {}{}", pure_nick, channel, reason),
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        is_highlight: false,
                        timestamp: now_millis(),
                    });
                }
            }
            IrcEvent::Kick { network, channel, nick, by, reason } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                let reason = reason.as_ref().map(|r| format!(" ({})", r)).unwrap_or_default();
                if let Some(buffer) = net.buffers.get_mut(&channel.to_lowercase()) {
                    let text = if nick.eq_ignore_ascii_case(&net.nick) {
                        // We were removed, so nobody in the channel is known any more.
                        buffer.users.clear();
                        format!("You were kicked from {} by {}{}", channel, by, reason)
                    } else {
                        buffer.users.remove(&nick.to_lowercase());
                        format!("{} was kicked from {} by {}{}", nick, channel, by, reason)
                    };
                    buffer.invalidate_lists_hash();
                    buffer.push_message(ChatMessage {
                        id: next_id,
                        sender: "*".to_string(),
                        text,
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        is_highlight: false,
                        timestamp: now_millis(),
                    });
                }
            }
            IrcEvent::Quit { network, nick, message, .. } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                let (_, pure_nick) = parse_user_prefix(&isupport, nick);
                let key = pure_nick.to_lowercase();
                let reason = message.as_ref().map(|m| format!(" ({})", m)).unwrap_or_default();

                for buf in net.buffers.values_mut() {
                    if buf.users.remove(&key).is_some() {
                        buf.invalidate_lists_hash();
                        buf.push_message(ChatMessage {
                            id: next_id,
                            sender: "*".to_string(),
                            text: format!("{} has quit{}", pure_nick, reason),
                            spans: Vec::new(),
                            is_notice: true,
                            is_action: false,
                            is_encrypted: false,
                            is_highlight: false,
                            timestamp: now_millis(),
                        });
                    }
                }
            }
            IrcEvent::NickChange { network, old_nick, new_nick, host, .. } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                if old_nick.eq_ignore_ascii_case(&net.nick) {
                    net.nick = new_nick.clone();
                }

                let old_key = old_nick.to_lowercase();
                for buf in net.buffers.values_mut() {
                    if let Some(user) = buf.users.remove(&old_key) {
                        buf.users.insert(new_nick.to_lowercase(), ChannelUser {
                            nick: new_nick.clone(),
                            prefix: user.prefix,
                            prefix_modes: user.prefix_modes,
                            host: host.clone().or(user.host),
                        });
                        buf.invalidate_lists_hash();
                        buf.push_message(ChatMessage {
                            id: next_id,
                            sender: "*".to_string(),
                            text: format!("{} is now known as {}", old_nick, new_nick),
                            spans: Vec::new(),
                            is_notice: true,
                            is_action: false,
                            is_encrypted: false,
                            is_highlight: false,
                            timestamp: now_millis(),
                        });
                    }
                }
            }
            IrcEvent::Topic { network, channel, topic, set_by } => {
                let next_id = {
                    self.next_msg_id += 1;
                    self.next_msg_id
                };
                let net = self.ensure_network(network);
                let chan_key = channel.to_lowercase();
                if let Some(buffer) = net.buffers.get_mut(&chan_key) {
                    buffer.topic = Some(topic.clone());
                    buffer.topic_set_by = set_by.clone();
                    if buffer.topic_history.last().map(|c| c.topic.as_str()) != Some(topic.as_str()) {
                        buffer.topic_history.push(TopicChange {
                            by: set_by.clone().unwrap_or_default(),
                            at: now_timestamp(),
                            topic: topic.clone(),
                        });
                        let excess = buffer.topic_history.len().saturating_sub(MAX_TOPIC_HISTORY);
                        buffer.topic_history.drain(..excess);
                        buffer.invalidate_lists_hash();
                    }
                    let by_str = set_by.as_ref().map(|s| format!(" by {}", s)).unwrap_or_default();
                    let text = format!("Topic set{}: {}", by_str, topic);
                    buffer.push_message(ChatMessage {
                        id: next_id,
                        sender: "*".to_string(),
                        text: text.clone(),
                        spans: Vec::new(),
                        is_notice: true,
                        is_action: false,
                        is_encrypted: false,
                        is_highlight: false,
                        timestamp: now_millis(),
                    });
                }
            }
            IrcEvent::Names { network, channel, names } => {
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                let chan_key = channel.to_lowercase();
                if let Some(buffer) = net.buffers.get_mut(&chan_key) {
                    buffer.invalidate_lists_hash();
                    for name in names {
                        let (prefix_modes, raw_nick) = parse_name_prefixes(&isupport, name);
                        // userhost-in-names: `nick!user@host`. The host is kept, the nick is the part before `!`.
                        let (pure_nick, name_host) = split_name_userhost(&raw_nick);
                        let prefix = displayed_prefix(&isupport, &prefix_modes);
                        let user_key = pure_nick.to_lowercase();
                        let host = name_host.or_else(|| buffer.users.get(&user_key).and_then(|u| u.host.clone()));
                        buffer.users.insert(
                            user_key,
                            ChannelUser {
                                nick: pure_nick,
                                prefix,
                                prefix_modes,
                                host,
                            },
                        );
                    }
                }
            }
            IrcEvent::ChannelModes { network, channel, modes, params } => {
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                if let Some(buffer) = net.buffers.get_mut(&channel.to_lowercase()) {
                    // 324 is the whole flag set, so the flags, key and limit start from empty.
                    buffer.channel_flags.clear();
                    buffer.channel_key = None;
                    buffer.channel_limit = None;
                    apply_channel_modes(buffer, &isupport, modes, params);
                }
            }
            IrcEvent::ChannelList { network, channel, list, mask, set_by, set_at } => {
                // Recorded up to the cap, like the entries, so a listing without an end line
                // cannot grow this set without limit.
                let seen = self
                    .list_seen
                    .entry((network.clone(), channel.to_lowercase(), *list))
                    .or_default();
                let mask_key = mask.to_lowercase();
                if seen.len() < MAX_CHANNEL_LIST || seen.contains(&mask_key) {
                    seen.insert(mask_key);
                }
                let net = self.ensure_network(network);
                if let Some(buffer) = net.buffers.get_mut(&channel.to_lowercase()) {
                    buffer.invalidate_lists_hash();
                    let entries = list_entries_mut(buffer, *list);
                    let full = entries.len() >= MAX_CHANNEL_LIST;
                    match entries.iter_mut().find(|b| b.mask.eq_ignore_ascii_case(mask)) {
                        Some(existing) => {
                            existing.by = set_by.clone();
                            existing.at = *set_at;
                        }
                        None if !full => entries.push(BanEntry {
                            mask: mask.clone(),
                            by: set_by.clone(),
                            at: *set_at,
                        }),
                        None => {}
                    }
                }
            }
            IrcEvent::ChannelListEnd { network, channel, list } => {
                let seen = self
                    .list_seen
                    .remove(&(network.clone(), channel.to_lowercase(), *list))
                    .unwrap_or_default();
                if let Some(buffer) = self
                    .networks
                    .get_mut(network)
                    .and_then(|net| net.buffers.get_mut(&channel.to_lowercase()))
                {
                    buffer.invalidate_lists_hash();
                    list_entries_mut(buffer, *list).retain(|b| seen.contains(&b.mask.to_lowercase()));
                }
            }
            IrcEvent::Mode { network, target, modes, params, .. } => {
                let isupport = self.isupport_for(network);
                let net = self.ensure_network(network);
                if let Some(buffer) = net.buffers.get_mut(&target.to_lowercase()) {
                    if !buffer.is_channel {
                        return;
                    }
                    buffer.invalidate_lists_hash();

                    apply_channel_modes(buffer, &isupport, modes, params);
                    let mut adding = true;
                    let mut param_index = 0;
                    for mode in modes.chars() {
                        match mode {
                            '+' => adding = true,
                            '-' => adding = false,
                            _ => {
                                let takes_param = isupport.mode_takes_param(mode, adding);
                                let param = if takes_param {
                                    let param = params.get(param_index);
                                    param_index += 1;
                                    param
                                } else {
                                    None
                                };

                                // `mode` is a mode letter here (o, v, ...), not a symbol.
                                if let (Some(_), Some(nick)) = (isupport.symbol_for_mode(mode), param) {
                                    let prefix_mode = mode;
                                    if let Some(user) = buffer.users.get_mut(&nick.to_lowercase()) {
                                        if adding {
                                            if !user.prefix_modes.contains(&prefix_mode) {
                                                user.prefix_modes.push(prefix_mode);
                                            }
                                        } else {
                                            user.prefix_modes.retain(|existing| *existing != prefix_mode);
                                        }
                                        user.prefix = displayed_prefix(&isupport, &user.prefix_modes);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The list behind a list mode: 'b' bans, 'e' excepts, 'I' invites.
fn list_entries_mut(buffer: &mut BufferState, list: char) -> &mut Vec<BanEntry> {
    match list {
        'e' => &mut buffer.excepts,
        'I' => &mut buffer.invites,
        _ => &mut buffer.bans,
    }
}

/// Applies a mode string to a channel: flags, key, limit and the ban list. Prefix modes (o, v, ...)
/// belong to the nick list and are skipped here, but their parameters are still counted so the
/// parameter order stays right for the modes after them.
fn apply_channel_modes(buffer: &mut BufferState, isupport: &Isupport, modes: &str, params: &[String]) {
    buffer.invalidate_lists_hash();
    let mut adding = true;
    let mut param_index = 0;
    for mode in modes.chars() {
        match mode {
            '+' => adding = true,
            '-' => adding = false,
            _ => {
                let takes_param = isupport.mode_takes_param(mode, adding);
                let param = if takes_param {
                    let value = params.get(param_index).cloned();
                    param_index += 1;
                    value
                } else {
                    None
                };
                if isupport.symbol_for_mode(mode).is_some() {
                    continue;
                }
                match mode {
                    'b' | 'e' | 'I' => {
                        if let Some(mask) = param {
                            let entries = list_entries_mut(buffer, mode);
                            if adding {
                                if entries.len() < MAX_CHANNEL_LIST
                                    && !entries.iter().any(|b| b.mask.eq_ignore_ascii_case(&mask))
                                {
                                    entries.push(BanEntry { mask, by: String::new(), at: now_timestamp() });
                                }
                            } else {
                                entries.retain(|b| !b.mask.eq_ignore_ascii_case(&mask));
                            }
                        }
                    }
                    'k' => buffer.channel_key = if adding { param } else { None },
                    'l' => buffer.channel_limit = if adding { param } else { None },
                    _ => {
                        // Flags are letters; anything else from a server is ignored.
                        if !mode.is_ascii_alphabetic() {
                            continue;
                        }
                        if adding {
                            if !buffer.channel_flags.contains(mode) {
                                buffer.channel_flags.push(mode);
                            }
                        } else {
                            buffer.channel_flags.retain(|c| c != mode);
                        }
                    }
                }
            }
        }
    }
}

fn now_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Unix time in milliseconds, for chat messages (the chat shows milliseconds on request).
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn parse_user_prefix(isupport: &Isupport, raw: &str) -> (Option<char>, String) {
    if let Some(first) = raw.chars().next() {
        if mode_for_prefix(isupport, first).is_some() {
            return (Some(first), raw[first.len_utf8()..].to_string());
        }
    }
    (None, raw.to_string())
}

fn parse_name_prefixes(isupport: &Isupport, raw: &str) -> (Vec<char>, String) {
    let mut modes = Vec::new();
    let mut nick_start = 0;
    for (index, symbol) in raw.char_indices() {
        let Some(mode) = mode_for_prefix(isupport, symbol) else { break };
        modes.push(mode);
        nick_start = index + symbol.len_utf8();
    }
    (modes, raw[nick_start..].to_string())
}

/// Splits `nick!user@host` (userhost-in-names) into the nick and the host. A name without a host
/// part is returned unchanged, with no host.
fn split_name_userhost(raw: &str) -> (String, Option<String>) {
    match raw.split_once('!') {
        Some((nick, rest)) => {
            let host = rest.split_once('@').map(|(_, host)| host.to_string()).filter(|h| !h.is_empty());
            (nick.to_string(), host)
        }
        None => (raw.to_string(), None),
    }
}

/// Mode letter for a prefix symbol, e.g. '@' -> 'o'.
fn mode_for_prefix(isupport: &Isupport, symbol: char) -> Option<char> {
    isupport.mode_for_symbol(symbol)
}

/// Symbol of the highest-ranked mode in `modes`.
fn displayed_prefix(isupport: &Isupport, modes: &[char]) -> Option<char> {
    modes
        .iter()
        .filter_map(|mode| isupport.rank(*mode).map(|rank| (rank, *mode)))
        .min_by_key(|(rank, _)| *rank)
        .and_then(|(_, mode)| isupport.symbol_for_mode(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_output_survives_backend_events_and_stays_in_the_selected_buffer() {
        let mut state = AppState::new();
        state.handle_event(&IrcEvent::Connected { network: "net".into(), server: "server".into() });
        assert!(!state.push_local_message("net", "missing", "wrong".into()));
        assert!(state.push_local_message("net", "status", "script output".into()));
        state.handle_event(&IrcEvent::Disconnected { network: "net".into(), reason: "done".into() });
        let messages = &state.networks["net"].buffers["status"].messages;
        assert!(messages.iter().any(|message| message.text == "script output"));
        assert!(messages.windows(2).all(|pair| pair[0].id < pair[1].id));
    }

    #[test]
    fn push_local_message_and_clear_buffer_produce_snapshot_after_previous_sync() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let snap1 = state.ui_snapshot_if_changed(false, None);
        assert!(snap1.is_some());
        let rev1 = snap1.unwrap().revision;

        assert!(state.ui_snapshot_if_changed(false, Some(rev1)).is_none());

        assert!(state.push_local_message("net", "#chan", "local text".to_string()));
        let snap2 = state.ui_snapshot_if_changed(false, Some(rev1));
        assert!(snap2.is_some(), "push_local_message should trigger a snapshot");
        let rev2 = snap2.unwrap().revision;
        assert_ne!(rev1, rev2);

        assert!(state.ui_snapshot_if_changed(false, Some(rev2)).is_none());

        assert!(state.clear_buffer_messages("net", "#chan"));
        let snap3 = state.ui_snapshot_if_changed(false, Some(rev2));
        assert!(snap3.is_some(), "clear_buffer_messages should trigger a snapshot");
        let rev3 = snap3.unwrap().revision;
        assert_ne!(rev2, rev3);
    }

    fn join(state: &mut AppState, nick: &str, host: Option<&str>) {
        state.handle_event(&IrcEvent::Join { address: None,
            network: "net".into(),
            channel: "#chan".into(),
            nick: nick.into(),
            host: host.map(str::to_string),
        });
    }

    fn privmsg(state: &mut AppState, sender: &str, host: Option<&str>) {
        state.handle_event(&IrcEvent::Message { sender_address: None,
            network: "net".into(),
            target: "#chan".into(),
            sender: sender.into(),
            text: "hi".into(),
            spans: Vec::new(),
            is_notice: false,
            is_action: false,
            is_encrypted: false,
            timestamp: 0,
            sender_host: host.map(str::to_string),
        });
    }

    fn host_of(state: &AppState, nick: &str) -> Option<String> {
        state.networks["net"].buffers["#chan"].users.get(&nick.to_lowercase())?.host.clone()
    }

    #[test]
    fn join_stores_host() {
        let mut state = AppState::new();
        join(&mut state, "Alice", Some("a.example.com"));
        assert_eq!(host_of(&state, "alice").as_deref(), Some("a.example.com"));
    }

    #[test]
    fn privmsg_updates_host() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        assert_eq!(host_of(&state, "alice"), None);
        privmsg(&mut state, "Alice", Some("b.example.com"));
        assert_eq!(host_of(&state, "alice").as_deref(), Some("b.example.com"));
    }

    #[test]
    fn nick_change_keeps_host() {
        let mut state = AppState::new();
        join(&mut state, "Alice", Some("a.example.com"));
        state.handle_event(&IrcEvent::NickChange { address: None,
            network: "net".into(),
            old_nick: "Alice".into(),
            new_nick: "Alicia".into(),
            host: None,
        });
        assert_eq!(host_of(&state, "alice"), None);
        assert_eq!(host_of(&state, "alicia").as_deref(), Some("a.example.com"));
    }

    fn isupport_line(state: &mut AppState, tokens: &str) {
        state.handle_event(&IrcEvent::Raw {
            network: "net".into(),
            raw: format!(":irc.example.net 005 me {} :are supported by this server", tokens),
        });
    }

    /// Join #chan, deliver NAMES, return (prefix_modes, prefix) per pure nick.
    fn names(state: &mut AppState, names: &[&str]) -> Vec<(String, Vec<char>, Option<char>)> {
        join(state, "me", None);
        state.handle_event(&IrcEvent::Names {
            network: "net".into(),
            channel: "#chan".into(),
            names: names.iter().map(|n| n.to_string()).collect(),
        });
        let users = &state.networks["net"].buffers["#chan"].users;
        let mut out: Vec<_> = users
            .values()
            .filter(|u| u.nick != "me")
            .map(|u| (u.nick.clone(), u.prefix_modes.clone(), u.prefix))
            .collect();
        out.sort();
        out
    }

    const DEFAULT_NAMES: [&str; 6] = ["~Q", "&A", "@O", "%H", "+V", "@+OV"];

    #[test]
    fn default_prefix_005_matches_fallback() {
        let mut with_005 = AppState::new();
        isupport_line(&mut with_005, "CHANTYPES=# PREFIX=(qaohv)~&@%+ NETWORK=Test");
        let mut without = AppState::new();
        assert_eq!(names(&mut with_005, &DEFAULT_NAMES), names(&mut without, &DEFAULT_NAMES));
        let got = names(&mut with_005, &DEFAULT_NAMES);
        assert!(got.contains(&("Q".to_string(), vec!['q'], Some('~'))));
        assert!(got.contains(&("OV".to_string(), vec!['o', 'v'], Some('@'))));
    }

    #[test]
    fn custom_prefix_005_changes_mapping() {
        let mut state = AppState::new();
        isupport_line(&mut state, "PREFIX=(yov)!@+");
        let got = names(&mut state, &["!Alice", "@Bob", "~Carol", "@!Dave"]);
        assert!(got.contains(&("Alice".to_string(), vec!['y'], Some('!'))));
        assert!(got.contains(&("Bob".to_string(), vec!['o'], Some('@'))));
        // '~' is not a prefix on this network, so it stays part of the nick.
        assert!(got.contains(&("~Carol".to_string(), vec![], None)));
        // Highest rank in PREFIX order wins: y outranks o.
        assert!(got.contains(&("Dave".to_string(), vec!['o', 'y'], Some('!'))));
    }

    #[test]
    fn missing_005_uses_fallback() {
        let mut state = AppState::new();
        let got = names(&mut state, &["~Q", "+V"]);
        assert!(got.contains(&("Q".to_string(), vec!['q'], Some('~'))));
        assert!(got.contains(&("V".to_string(), vec!['v'], Some('+'))));
    }

    #[test]
    fn prefix_pairs_are_exposed_to_the_ui() {
        let mut state = AppState::new();
        isupport_line(&mut state, "PREFIX=(yov)!@+");
        assert_eq!(state.network_prefix["net"], vec![('y', '!'), ('o', '@'), ('v', '+')]);

        // A 005 line without PREFIX, or a fresh connection, gives the fallback.
        let fallback = DEFAULT_PREFIX.to_vec();
        let mut no_prefix = AppState::new();
        isupport_line(&mut no_prefix, "CHANTYPES=# NETWORK=Test");
        assert_eq!(no_prefix.network_prefix["net"], fallback);
        state.handle_event(&IrcEvent::Connected { network: "net".into(), server: "s".into() });
        assert_eq!(state.network_prefix["net"], fallback);
    }

    #[test]
    fn malformed_prefix_is_ignored() {
        let long = format!("PREFIX=({}){}", "a".repeat(20), "!".repeat(20));
        for bad in [
            "PREFIX=(ov)@",      // length mismatch
            "PREFIX=(o1)@+",     // mode is not a letter
            "PREFIX=(ov)a+",     // symbol is a letter
            "PREFIX=(ov@+",      // no closing paren
            "PREFIX=ov@+",       // no parens
            "PREFIX=(oo)@+",     // duplicate mode
            "PREFIX=",           // empty
            long.as_str(),       // longer than 32 characters
        ] {
            let mut state = AppState::new();
            isupport_line(&mut state, bad);
            let got = names(&mut state, &["@Bob"]);
            assert_eq!(got, vec![("Bob".to_string(), vec!['o'], Some('@'))], "token {bad}");
            assert!(state.isupport_for("net").prefix.is_none(), "token {bad}");
        }

        // A malformed token after a valid one keeps the valid mapping.
        let mut state = AppState::new();
        isupport_line(&mut state, "PREFIX=(yov)!@+");
        isupport_line(&mut state, "PREFIX=(ov)@");
        assert_eq!(state.isupport_for("net").prefix, Some(vec![('y', '!'), ('o', '@'), ('v', '+')]));
    }

    #[test]
    fn prefix_005_from_user_is_ignored_and_reconnect_resets() {
        let mut state = AppState::new();
        state.handle_event(&IrcEvent::Raw {
            network: "net".into(),
            raw: ":evil!u@h 005 me PREFIX=(yov)!@+ :x".into(),
        });
        assert!(state.isupport_for("net").prefix.is_none());
        isupport_line(&mut state, "PREFIX=(yov)!@+");
        assert!(state.isupport_for("net").prefix.is_some());
        state.handle_event(&IrcEvent::Connected { network: "net".into(), server: "s".into() });
        assert!(state.isupport_for("net").prefix.is_none());
    }

    #[test]
    fn names_keep_multi_prefixes_and_the_host() {
        let mut state = AppState::new();
        names(&mut state, &["@+Alice!a@alice.example", "Bob"]);
        let users = &state.networks["net"].buffers["#chan"].users;
        let alice = &users["alice"];
        assert_eq!(alice.nick, "Alice");
        assert_eq!(alice.prefix, Some('@'));
        assert_eq!(alice.host.as_deref(), Some("alice.example"));
        assert_eq!(users["bob"].host, None);
    }

    #[test]
    fn mode_event_updates_prefix_modes() {
        let mut state = AppState::new();
        names(&mut state, &["Alice"]);
        state.handle_event(&IrcEvent::Mode {
            network: "net".into(),
            target: "#chan".into(),
            modes: "+o".into(),
            params: vec!["Alice".into()],
            by: None,
        });
        let user = &state.networks["net"].buffers["#chan"].users["alice"];
        assert_eq!((user.prefix_modes.clone(), user.prefix), (vec!['o'], Some('@')));
    }

    #[test]
    fn chanmodes_005_decides_which_modes_take_params() {
        let mode = |state: &mut AppState| {
            state.handle_event(&IrcEvent::Mode {
                network: "net".into(),
                target: "#chan".into(),
                modes: "+Zo".into(),
                params: vec!["x!*@*".into(), "Alice".into()],
                by: None,
            });
            state.networks["net"].buffers["#chan"].users["alice"].prefix
        };

        // Z is a type-A (list) mode on this network: it consumes the first param.
        let mut with_005 = AppState::new();
        isupport_line(&mut with_005, "CHANMODES=beIZ,k,l,imnpst");
        names(&mut with_005, &["Alice"]);
        assert_eq!(mode(&mut with_005), Some('@'));

        // Fallback table does not know Z, so "x!*@*" is taken as the op target and Alice stays plain.
        let mut without = AppState::new();
        names(&mut without, &["Alice"]);
        assert_eq!(mode(&mut without), None);
    }

    #[test]
    fn remove_network_drops_buffers_and_moves_active() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.handle_event(&IrcEvent::Connected { network: "other".into(), server: "s".into() });
        state.active_network = Some("net".into());

        assert!(state.remove_network("net"));
        assert!(!state.networks.contains_key("net"));
        assert!(!state.network_prefix.contains_key("net"));
        assert_eq!(state.active_network.as_deref(), Some("other"));

        assert!(!state.remove_network("net"));
        assert!(state.remove_network("other"));
        assert_eq!(state.active_network, None);
    }

    #[test]
    fn ui_snapshot_caps_messages_per_buffer() {
        let mut state = AppState::new();
        for i in 0..MAX_BUFFER_MESSAGES {
            state.handle_event(&IrcEvent::Message { sender_address: None,
                network: "net".into(),
                target: "#chan".into(),
                sender: "Alice".into(),
                text: format!("line {i}"),
                spans: Vec::new(),
                is_notice: false,
                is_action: false,
                is_encrypted: false,
                timestamp: 0,
                sender_host: None,
            });
        }
        let snap = state.ui_snapshot();
        let sent = &snap.networks["net"].buffers["#chan"].messages;
        assert_eq!(sent.len(), MAX_UI_MESSAGES);
        assert_eq!(sent.first().unwrap().text, format!("line {}", MAX_BUFFER_MESSAGES - MAX_UI_MESSAGES));
        assert_eq!(sent.last().unwrap().text, format!("line {}", MAX_BUFFER_MESSAGES - 1));
        assert_eq!(state.networks["net"].buffers["#chan"].messages.len(), MAX_BUFFER_MESSAGES);
        assert_eq!(snap.networks["net"].buffers["#chan"].name, "#chan");
    }

    #[test]
    fn buffer_keeps_only_the_newest_messages() {
        let mut state = AppState::new();
        let total = MAX_BUFFER_MESSAGES + 25;
        for i in 0..total {
            state.handle_event(&IrcEvent::Message { sender_address: None,
                network: "net".into(),
                target: "#chan".into(),
                sender: "Alice".into(),
                text: format!("line {i}"),
                spans: Vec::new(),
                is_notice: false,
                is_action: false,
                is_encrypted: false,
                timestamp: 0,
                sender_host: None,
            });
        }
        let messages = &state.networks["net"].buffers["#chan"].messages;
        assert_eq!(messages.len(), MAX_BUFFER_MESSAGES);
        assert_eq!(messages.first().unwrap().text, "line 25");
        assert_eq!(messages.last().unwrap().text, format!("line {}", total - 1));
        assert!(messages.windows(2).all(|pair| pair[0].id < pair[1].id));
    }

    #[test]
    fn status_buffer_is_capped_by_system_lines() {
        let mut state = AppState::new();
        for _ in 0..MAX_BUFFER_MESSAGES + 5 {
            state.handle_event(&IrcEvent::Error { network: "net".into(), message: "x".into() });
        }
        assert_eq!(state.networks["net"].buffers["status"].messages.len(), MAX_BUFFER_MESSAGES);
    }

    #[test]
    fn topic_history_records_changes_once_and_caps() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let topic = |text: &str, by: Option<&str>| IrcEvent::Topic {
            network: "net".into(),
            channel: "#chan".into(),
            topic: text.into(),
            set_by: by.map(str::to_string),
        };
        state.handle_event(&topic("first", Some("Alice")));
        state.handle_event(&topic("first", Some("Alice")));
        state.handle_event(&topic("second", Some("Bob")));
        let buffer = &state.networks["net"].buffers["#chan"];
        assert_eq!(buffer.topic_history.len(), 2);
        assert_eq!(buffer.topic_history[1].topic, "second");
        assert_eq!(buffer.topic_history[1].by, "Bob");
        assert_eq!(buffer.topic_set_by.as_deref(), Some("Bob"));
        for i in 0..MAX_TOPIC_HISTORY + 10 {
            state.handle_event(&topic(&format!("t{i}"), Some("Carol")));
        }
        let buffer = &state.networks["net"].buffers["#chan"];
        assert_eq!(buffer.topic_history.len(), MAX_TOPIC_HISTORY);
        assert_eq!(buffer.topic_history.last().unwrap().topic, format!("t{}", MAX_TOPIC_HISTORY + 9));
    }

    #[test]
    fn a_kicked_user_leaves_the_user_list() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        join(&mut state, "Bob", None);
        state.handle_event(&IrcEvent::Kick {
            network: "net".into(),
            channel: "#chan".into(),
            nick: "Bob".into(),
            by: "Alice".into(),
            reason: Some("spam".into()),
        });
        let buffer = &state.networks["net"].buffers["#chan"];
        assert!(!buffer.users.contains_key("bob"));
        assert!(buffer.messages.iter().any(|m| m.text == "Bob was kicked from #chan by Alice (spam)"));
    }

    #[test]
    fn a_private_notice_goes_to_status_not_to_a_new_query() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.networks.get_mut("net").unwrap().nick = "Alice".into();
        state.handle_event(&IrcEvent::Message {
            sender_address: None,
            network: "net".into(),
            target: "Alice".into(),
            sender: "Oper".into(),
            text: "hello".into(),
            spans: Vec::new(),
            is_notice: true,
            is_action: false,
            is_encrypted: false,
            timestamp: 0,
            sender_host: None,
        });
        assert!(!state.networks["net"].buffers.contains_key("oper"));
        assert!(state.networks["net"].buffers["status"].messages.iter().any(|m| m.text == "hello"));
    }

    #[test]
    fn a_notice_to_channel_operators_stays_in_the_channel() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.networks.get_mut("net").unwrap().nick = "Alice".into();
        state.handle_event(&IrcEvent::Message {
            sender_address: None,
            network: "net".into(),
            target: "@#chan".into(),
            sender: "server.example".into(),
            text: "CZ invited badkid into the channel.".into(),
            spans: Vec::new(),
            is_notice: true,
            is_action: false,
            is_encrypted: false,
            timestamp: 0,
            sender_host: None,
        });
        assert!(!state.networks["net"].buffers.contains_key("@#chan"));
        assert!(state.networks["net"].buffers["#chan"].messages.iter().any(|m| m.text.contains("invited badkid")));
    }

    #[test]
    fn channels_past_the_cap_are_not_tracked() {
        let mut state = AppState::new();
        for i in 0..=MAX_CHANNEL_BUFFERS {
            state.handle_event(&IrcEvent::Join {
                address: None,
                network: "net".into(),
                channel: format!("#c{i}"),
                nick: "Alice".into(),
                host: None,
            });
        }
        let net = &state.networks["net"];
        assert_eq!(net.buffers.values().filter(|b| b.is_channel).count(), MAX_CHANNEL_BUFFERS);
        assert!(net.buffers["status"].messages.iter().any(|m| m.text.starts_with("Not tracking #c500")));
    }

    #[test]
    fn mode_lists_stop_at_the_cap() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let masks: Vec<String> = (0..MAX_CHANNEL_LIST + 10).map(|i| format!("*!*@host{i}")).collect();
        state.handle_event(&IrcEvent::Mode {
            network: "net".into(),
            target: "#chan".into(),
            modes: format!("+{}", "b".repeat(masks.len())),
            params: masks,
            by: None,
        });
        assert_eq!(state.networks["net"].buffers["#chan"].bans.len(), MAX_CHANNEL_LIST);
    }

    #[test]
    fn an_unchanged_state_builds_no_copy_for_the_revision_the_ui_has() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let first = state.ui_snapshot_if_changed(false, None).expect("first copy");
        assert!(state.ui_snapshot_if_changed(false, Some(first.revision)).is_none());
        join(&mut state, "Bob", None);
        let second = state.ui_snapshot_if_changed(false, Some(first.revision)).expect("changed");
        assert!(second.revision > first.revision);
        assert!(state.ui_snapshot_if_changed(true, Some(second.revision)).is_some());
    }

    #[test]
    fn a_background_buffers_lists_are_sent_when_they_change() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        join(&mut state, "Bob", None);
        state.active_network = Some("net".into());
        state.networks.get_mut("net").unwrap().active_buffer = "status".into();
        let away = state.ui_snapshot_delta(false);
        let buffer = &away.networks["net"].buffers["#chan"];
        assert_eq!(buffer.lists_mode, "full");
        assert_eq!(buffer.users.len(), 2);

        // Nothing changed: the background window's lists are left out of the copy.
        let unchanged = state.ui_snapshot_delta(false);
        assert_eq!(unchanged.networks["net"].buffers["#chan"].lists_mode, "same");

        join(&mut state, "Carol", None);
        let changed = state.ui_snapshot_delta(false);
        let buffer = &changed.networks["net"].buffers["#chan"];
        assert_eq!(buffer.lists_mode, "full");
        assert_eq!(buffer.users.len(), 3);
    }

    #[test]
    fn a_background_window_list_change_moves_the_digest() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        join(&mut state, "Bob", None);
        state.active_network = Some("net".into());
        state.networks.get_mut("net").unwrap().active_buffer = "status".into();
        let before = state.ui_digest();
        let chan = state.networks.get_mut("net").unwrap().buffers.get_mut("#chan").unwrap();
        chan.users.get_mut("bob").unwrap().prefix = Some('@');
        chan.invalidate_lists_hash();
        assert_ne!(before, state.ui_digest());
    }

    #[test]
    fn the_revision_moves_only_when_the_ui_would_see_a_change() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let first = state.ui_snapshot_delta(false).revision;
        assert_eq!(state.ui_snapshot_delta(false).revision, first);
        join(&mut state, "Bob", None);
        let second = state.ui_snapshot_delta(false).revision;
        assert!(second > first);
        assert_eq!(state.ui_snapshot_delta(false).revision, second);
    }

    #[test]
    fn lists_are_sent_again_only_when_they_change() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        join(&mut state, "Bob", None);
        // The lists go to the window the user looks at.
        state.active_network = Some("net".into());
        state.networks.get_mut("net").unwrap().active_buffer = "#chan".into();
        let first = state.ui_snapshot_delta(false);
        let buffer = &first.networks["net"].buffers["#chan"];
        assert_eq!(buffer.lists_mode, "full");
        assert_eq!(buffer.users.len(), 2);

        // Nothing changed: the lists are left out of the copy.
        let unchanged = state.ui_snapshot_delta(false);
        let buffer = &unchanged.networks["net"].buffers["#chan"];
        assert_eq!(buffer.lists_mode, "same");
        assert!(buffer.users.is_empty());

        join(&mut state, "Carol", None);
        let changed = state.ui_snapshot_delta(false);
        let buffer = &changed.networks["net"].buffers["#chan"];
        assert_eq!(buffer.lists_mode, "full");
        assert_eq!(buffer.users.len(), 3);

        // A full snapshot always carries the lists.
        let forced = state.ui_snapshot_delta(true);
        assert_eq!(forced.networks["net"].buffers["#chan"].lists_mode, "full");
    }

    #[test]
    fn a_closed_query_is_forgotten_by_the_ui_sync() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.networks.get_mut("net").unwrap().nick = "Alice".into();
        state.handle_event(&IrcEvent::Message {
            sender_address: None,
            network: "net".into(),
            target: "Alice".into(),
            sender: "Bob".into(),
            text: "hi".into(),
            spans: Vec::new(),
            is_notice: false,
            is_action: false,
            is_encrypted: false,
            timestamp: 0,
            sender_host: None,
        });
        let key = ("net".to_string(), "bob".to_string());
        state.active_network = Some("net".into());
        state.networks.get_mut("net").unwrap().active_buffer = "bob".into();
        state.ui_snapshot_delta(true);
        assert!(state.sent_to_ui.contains_key(&key));
        assert!(state.sent_lists.contains_key(&key));
        state.networks.get_mut("net").unwrap().buffers.remove("bob");
        state.ui_snapshot_delta(false);
        assert!(!state.sent_to_ui.contains_key(&key));
        assert!(!state.sent_lists.contains_key(&key));
    }

    #[test]
    fn a_ban_the_server_no_longer_lists_is_dropped() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let entry = |mask: &str| IrcEvent::ChannelList {
            network: "net".into(),
            channel: "#chan".into(),
            list: 'b',
            mask: mask.into(),
            set_by: "Bob".into(),
            set_at: 1,
        };
        let end = IrcEvent::ChannelListEnd {
            network: "net".into(),
            channel: "#chan".into(),
            list: 'b',
        };
        state.handle_event(&entry("*!*@old.example"));
        state.handle_event(&entry("*!*@kept.example"));
        state.handle_event(&end);
        assert_eq!(state.networks["net"].buffers["#chan"].bans.len(), 2);
        // The next listing has only one of them: the other was lifted on the server.
        state.handle_event(&entry("*!*@kept.example"));
        state.handle_event(&end);
        let bans = &state.networks["net"].buffers["#chan"].bans;
        assert_eq!(bans.len(), 1);
        assert_eq!(bans[0].mask, "*!*@kept.example");
    }

    #[test]
    fn channel_modes_and_ban_list_follow_the_server() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.handle_event(&IrcEvent::ChannelModes {
            network: "net".into(),
            channel: "#chan".into(),
            modes: "+ntk".into(),
            params: vec!["secret".into()],
        });
        let buffer = &state.networks["net"].buffers["#chan"];
        assert_eq!(buffer.channel_flags, "nt");
        assert_eq!(buffer.channel_key.as_deref(), Some("secret"));
        state.handle_event(&IrcEvent::ChannelList {
            network: "net".into(),
            channel: "#chan".into(),
            list: 'b',
            mask: "*!*@bad.example".into(),
            set_by: "Bob".into(),
            set_at: 42,
        });
        state.handle_event(&IrcEvent::Mode {
            network: "net".into(),
            target: "#chan".into(),
            modes: "+b-n".into(),
            params: vec!["*!*@evil.example".into()],
            by: None,
        });
        state.handle_event(&IrcEvent::Mode {
            network: "net".into(),
            target: "#chan".into(),
            modes: "-b-k".into(),
            params: vec!["*!*@bad.example".into(), "secret".into()],
            by: None,
        });
        let buffer = &state.networks["net"].buffers["#chan"];
        assert_eq!(buffer.bans.len(), 1);
        assert_eq!(buffer.bans[0].mask, "*!*@evil.example");
        assert_eq!(buffer.channel_flags, "t");
        assert_eq!(buffer.channel_key, None);
    }

    #[test]
    fn ui_snapshot_sends_only_new_messages() {
        let mut state = AppState::new();
        let error = |state: &mut AppState, text: &str| {
            state.handle_event(&IrcEvent::Error { network: "net".into(), message: text.into() });
        };
        error(&mut state, "one");
        let first = state.ui_snapshot_delta(false);
        let status = &first.networks["net"].buffers["status"];
        assert_eq!(status.messages_mode, "full");
        assert_eq!(status.messages.len(), 1);

        let unchanged = state.ui_snapshot_delta(false);
        let status = &unchanged.networks["net"].buffers["status"];
        assert_eq!(status.messages_mode, "same");
        assert!(status.messages.is_empty());

        error(&mut state, "two");
        error(&mut state, "three");
        let appended = state.ui_snapshot_delta(false);
        let status = &appended.networks["net"].buffers["status"];
        assert_eq!(status.messages_mode, "append");
        let texts: Vec<&str> = status.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["*** Error: two", "*** Error: three"]);

        let forced = state.ui_snapshot_delta(true);
        let status = &forced.networks["net"].buffers["status"];
        assert_eq!(status.messages_mode, "full");
        assert_eq!(status.messages.len(), 3);
    }

    #[test]
    fn buffer_history_is_the_full_scrollback_and_the_ui_gets_a_tail() {
        let mut state = AppState::new();
        for i in 0..300 {
            state.handle_event(&IrcEvent::Error { network: "net".into(), message: format!("m{i}") });
        }
        assert_eq!(state.buffer_history("net", "status", 2000).len(), 300);
        let last = state.buffer_history("net", "STATUS", 5);
        assert_eq!(last.len(), 5);
        assert!(last[4].text.ends_with("m299"));
        assert!(state.buffer_history("net", "missing", 5).is_empty());
        assert!(state.push_local_message("net", "STATUS", "local".to_string()));
        let snapshot = state.ui_snapshot_delta(true);
        assert_eq!(snapshot.networks["net"].buffers["status"].messages.len(), UI_TAIL_MESSAGES);
    }

    #[test]
    fn clear_buffer_drops_the_scrollback_for_good() {
        let mut state = AppState::new();
        for i in 0..3 {
            state.handle_event(&IrcEvent::Error { network: "net".into(), message: format!("m{i}") });
        }
        assert_eq!(state.ui_snapshot_delta(false).networks["net"].buffers["status"].messages.len(), 3);
        assert!(state.clear_buffer_messages("net", "STATUS"));
        assert!(state.buffer_history("net", "status", 2000).is_empty());
        assert!(state.ui_snapshot_delta(false).networks["net"].buffers["status"].messages.is_empty());
        assert!(state.push_local_message("net", "status", "after".to_string()));
        let fresh = state.ui_snapshot_delta(false);
        let messages = &fresh.networks["net"].buffers["status"].messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "after");
        assert!(!state.clear_buffer_messages("net", "missing"));
    }

    #[test]
    fn excepts_and_invites_follow_the_server() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.handle_event(&IrcEvent::ChannelList {
            network: "net".into(),
            channel: "#chan".into(),
            list: 'e',
            mask: "*!*@friend.example".into(),
            set_by: "Bob".into(),
            set_at: 7,
        });
        state.handle_event(&IrcEvent::ChannelList {
            network: "net".into(),
            channel: "#chan".into(),
            list: 'I',
            mask: "guest!*@*".into(),
            set_by: String::new(),
            set_at: 0,
        });
        state.handle_event(&IrcEvent::Mode {
            network: "net".into(),
            target: "#chan".into(),
            modes: "+I-e".into(),
            params: vec!["*!*@new.example".into(), "*!*@friend.example".into()],
            by: None,
        });
        let buffer = &state.networks["net"].buffers["#chan"];
        assert!(buffer.excepts.is_empty());
        let masks: Vec<&str> = buffer.invites.iter().map(|b| b.mask.as_str()).collect();
        assert_eq!(masks, ["guest!*@*", "*!*@new.example"]);
        assert!(buffer.bans.is_empty());
    }

    #[test]
    fn disconnect_and_own_part_clear_user_lists() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        state.networks.get_mut("net").unwrap().nick = "me".into();
        join(&mut state, "me", None);
        assert_eq!(state.networks["net"].buffers["#chan"].users.len(), 2);
        state.handle_event(&IrcEvent::Part { address: None,
            network: "net".into(),
            channel: "#chan".into(),
            nick: "me".into(),
            message: None,
        });
        assert!(state.networks["net"].buffers["#chan"].users.is_empty());

        join(&mut state, "Alice", None);
        state.handle_event(&IrcEvent::Disconnected { network: "net".into(), reason: "x".into() });
        assert!(state.networks["net"].buffers["#chan"].users.is_empty());
    }

    #[test]
    fn part_removes_user() {
        let mut state = AppState::new();
        join(&mut state, "Alice", Some("a.example.com"));
        state.handle_event(&IrcEvent::Part { address: None,
            network: "net".into(),
            channel: "#chan".into(),
            nick: "Alice".into(),
            message: None,
        });
        assert!(state.networks["net"].buffers["#chan"].users.is_empty());
    }
    fn msg(state: &mut AppState, target: &str, sender: &str, text: &str) {
        state.handle_event(&IrcEvent::Message { sender_address: None,
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
        });
    }

    #[test]
    fn welcome_sets_the_server_assigned_nick() {
        let mut state = AppState::new();
        state.handle_event(&IrcEvent::Raw {
            network: "net".into(),
            raw: ":irc.example.net 001 Assigned_ :Welcome".into(),
        });
        assert_eq!(state.networks["net"].nick, "Assigned_");
        state.handle_event(&IrcEvent::Connected { network: "net".into(), server: "s".into() });
        assert_eq!(state.networks["net"].nick, "Assigned_");
        // A PM to the assigned nick lands in the sender's query buffer.
        msg(&mut state, "Assigned_", "Bob", "hello");
        assert!(state.networks["net"].buffers.contains_key("bob"));
        // A user cannot fake it.
        state.handle_event(&IrcEvent::Raw {
            network: "net".into(),
            raw: ":evil!u@h 001 Mallory :x".into(),
        });
        assert_eq!(state.networks["net"].nick, "Assigned_");
    }

    #[test]
    fn channel_prefixes_decide_channel_buffers() {
        let mut state = AppState::new();
        for name in ["#a", "&b", "+c", "!d"] {
            msg(&mut state, name, "Bob", "x");
            assert!(state.networks["net"].buffers[&name.to_lowercase()].is_channel, "{name}");
        }
        msg(&mut state, "Carol", "Bob", "x");
        assert!(!state.networks["net"].buffers["carol"].is_channel);

        // With CHANTYPES=# only, "&b" is a query target.
        let mut state = AppState::new();
        isupport_line(&mut state, "CHANTYPES=#");
        msg(&mut state, "&b", "Bob", "x");
        assert!(!state.networks["net"].buffers["&b"].is_channel);
    }

    #[test]
    fn query_buffers_are_capped_and_the_stalest_is_dropped() {
        let mut state = AppState::new();
        for i in 0..MAX_QUERY_BUFFERS {
            msg(&mut state, "FerricUser", &format!("user{i}"), "hi");
        }
        msg(&mut state, "#chan", "Bob", "x");
        // user0 is the stalest but active, so user1 goes instead.
        state.networks.get_mut("net").unwrap().active_buffer = "user0".into();
        msg(&mut state, "FerricUser", "newcomer", "hi");
        let buffers = &state.networks["net"].buffers;
        let queries = buffers.values().filter(|b| !b.is_channel && b.name != "status").count();
        assert_eq!(queries, MAX_QUERY_BUFFERS);
        assert!(buffers.contains_key("user0"));
        assert!(!buffers.contains_key("user1"));
        assert!(buffers.contains_key("newcomer"));
        assert!(buffers.contains_key("#chan"));
        // A reply in an existing query does not evict anything.
        msg(&mut state, "FerricUser", "user5", "again");
        assert_eq!(state.networks["net"].buffers.values().filter(|b| !b.is_channel && b.name != "status").count(), MAX_QUERY_BUFFERS);
    }

    #[test]
    fn stale_active_buffer_heals_to_status() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        {
            let net = state.networks.get_mut("net").unwrap();
            net.active_buffer = "gone".to_string();
        }
        state.heal_active_buffers();
        assert_eq!(state.networks["net"].active_buffer, "status");
    }

    #[test]
    fn no_op_mutations_do_not_mark_dirty() {
        let mut state = AppState::new();
        join(&mut state, "Alice", None);
        let before = state.events_counter;
        assert!(!state.push_local_message("net", "missing", "n".into()));
        assert!(!state.clear_buffer_messages("net", "missing"));
        assert!(!state.remove_network("unknown"));
        assert_eq!(state.events_counter, before);
        assert!(state.remove_network("net"));
        assert_ne!(state.events_counter, before);
    }

    #[test]
    fn old_snapshot_without_new_fields_loads_with_defaults() {
        let json = r#"{"networks":{"net":{"id":"net","server":"irc.test","nick":"Bob","connected":true,"active_buffer":"status","buffers":{"status":{"name":"status","is_channel":false,"topic":"Network status","users":{},"messages":[{"id":1,"sender":"*","text":"hi","is_notice":true,"is_action":false,"is_encrypted":false,"is_highlight":false,"timestamp":1}],"unread_count":0,"highlight_count":0}}}},"active_network":null,"network_prefix":{},"next_msg_id":1,"revision":1}"#;
        let state: AppState = serde_json::from_str(json).expect("old shape loads");
        assert_eq!(state.networks["net"].buffers["status"].messages.len(), 1);
    }

}

/// Characters an IRC nick can contain, besides letters and digits.
fn is_nick_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '[' | ']' | '\\' | '`' | '^' | '{' | '}' | '|' | '_')
}

/// Whether `text` names `nick` as a whole word, ignoring case. Uses the same
/// Unicode lowercasing as buffer/user keys so keys and mentions agree.
fn mentions_nick(text: &str, nick: &str) -> bool {
    if nick.is_empty() {
        return false;
    }
    let lower_text = text.to_lowercase();
    let lower_nick = nick.to_lowercase();
    lower_text.match_indices(&lower_nick).any(|(at, _)| {
        let before = lower_text[..at].chars().next_back();
        let after = lower_text[at + lower_nick.len()..].chars().next();
        !before.is_some_and(is_nick_char) && !after.is_some_and(is_nick_char)
    })
}

#[cfg(test)]
mod mention_tests {
    use super::mentions_nick;

    #[test]
    fn a_nick_mentions_only_as_a_whole_word() {
        assert!(mentions_nick("hi al!", "al"));
        assert!(mentions_nick("Alice: hello", "alice"));
        assert!(!mentions_nick("alpha beta", "al"));
        assert!(!mentions_nick("need a sec", "ed"));
        assert!(!mentions_nick("[al]x", "al"));
        assert!(!mentions_nick("hello", ""));
    }
}
