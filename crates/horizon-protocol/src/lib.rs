//! The Minecraft Java Edition network protocol (of the version in `horizon_core::version`):
//! primitive codecs, framing, compression, encryption, and typed packets for
//! every connection state, shared by the Horizon client and (later) server.
#![forbid(unsafe_code)]
pub mod buf;
pub mod chunk;
pub mod connection;
pub mod crypto;
pub mod entity_data;
pub mod frame;
pub mod hash_ops;
pub mod ids;
pub mod item;
pub mod mod_net;
pub mod packets;
pub mod quiet;
pub mod registry;
mod resolver;
pub use resolver::server_redirect;
pub mod rsa;
pub mod secure_chat;
pub mod servers;
pub mod text;

/// SharedConstants.getProtocolVersion().
pub const PROTOCOL_VERSION: i32 = horizon_core::version::PROTOCOL_VERSION;
/// The game version servers report in their status.
pub const GAME_VERSION: &str = horizon_core::VANILLA_VERSION;

/// Connection protocol states (ConnectionProtocol).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    Handshake,
    Status,
    Login,
    Configuration,
    Play,
}
/// PacketFlow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Clientbound,
    Serverbound,
}

/// A malformed or unsupported packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl Error {
    const TIMED_OUT: &str = "Timed out";
    const END_OF_STREAM: &str = "Connection closed by the server";
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
    /// The read timeout passed with nothing received (netty's
    /// `ReadTimeoutException`, which `Connection.exceptionCaught` reports as
    /// `disconnect.timeout`).
    pub fn timed_out() -> Self {
        Self::new(Self::TIMED_OUT)
    }
    pub fn is_timed_out(&self) -> bool {
        self.0 == Self::TIMED_OUT
    }
    /// The peer closed the stream (`Connection.channelInactive`).
    pub fn end_of_stream() -> Self {
        Self::new(Self::END_OF_STREAM)
    }
    pub fn is_end_of_stream(&self) -> bool {
        self.0 == Self::END_OF_STREAM
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;
