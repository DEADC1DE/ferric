pub mod colors;
pub mod message;

pub use colors::{parse_irc_formatting, StyledSpan};
pub use message::{IrcMessage, IrcPrefix};
