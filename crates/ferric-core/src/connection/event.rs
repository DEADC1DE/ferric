use serde::{Deserialize, Serialize};
use crate::protocol::StyledSpan;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IrcEvent {
    Connected {
        network: String,
        server: String,
    },
    Disconnected {
        network: String,
        reason: String,
    },
    Message {
        network: String,
        target: String,
        sender: String,
        text: String,
        #[serde(default, skip_serializing)]
        spans: Vec<StyledSpan>,
        is_notice: bool,
        is_action: bool,
        is_encrypted: bool,
        timestamp: u64,
        #[serde(default)]
        sender_host: Option<String>,
        /// `user@host` of the sender, when the server gave both.
        #[serde(default)]
        sender_address: Option<String>,
    },
    Join {
        network: String,
        channel: String,
        nick: String,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        address: Option<String>,
    },
    /// A user was removed from a channel by an operator. `nick` is the one removed.
    Kick {
        network: String,
        channel: String,
        nick: String,
        by: String,
        reason: Option<String>,
    },
    Part {
        network: String,
        channel: String,
        nick: String,
        message: Option<String>,
        #[serde(default)]
        address: Option<String>,
    },
    Quit {
        network: String,
        nick: String,
        message: Option<String>,
        #[serde(default)]
        address: Option<String>,
    },
    NickChange {
        network: String,
        old_nick: String,
        new_nick: String,
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        address: Option<String>,
    },
    Topic {
        network: String,
        channel: String,
        topic: String,
        set_by: Option<String>,
    },
    Names {
        network: String,
        channel: String,
        names: Vec<String>,
    },
    Mode {
        network: String,
        target: String,
        modes: String,
        params: Vec<String>,
        /// The nick in the prefix that set the modes; `None` when the server gave none.
        #[serde(default)]
        by: Option<String>,
    },
    /// 324 RPL_CHANNELMODEIS: the full mode string of a channel, with its parameters.
    ChannelModes {
        network: String,
        channel: String,
        modes: String,
        params: Vec<String>,
    },
    /// One entry of a channel list: `list` is 'b' (bans, 367), 'e' (excepts, 348) or 'I' (invites, 346).
    ChannelList {
        network: String,
        channel: String,
        list: char,
        mask: String,
        set_by: String,
        set_at: u64,
    },
    /// End of a channel list (368 for bans, 349 for excepts, 347 for invites).
    ChannelListEnd {
        network: String,
        channel: String,
        list: char,
    },
    Raw {
        network: String,
        raw: String,
    },
    Error {
        network: String,
        message: String,
    },
}
