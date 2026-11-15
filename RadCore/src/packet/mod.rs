//! Strongly-typed Minecraft protocol packets organized by [`ConnectionState`].
//!
//! Implementing [`Packet`] for new or custom packets only requires defining its
//! metadata (`ID`, `STATE`, `DIRECTION`) and body serialization (`decode_body` / `encode_body`).

pub mod configuration;
pub mod handshake;
pub mod login;
pub mod play;
pub mod status;

use bytes::{Buf, BufMut};

use crate::error::Result;
use crate::state::ConnectionState;

/// Direction of a Minecraft protocol packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PacketDirection {
    /// Sent from the Minecraft client to the server/proxy.
    Serverbound,
    /// Sent from the server/proxy to the Minecraft client.
    Clientbound,
}

/// A strongly-typed Minecraft protocol packet.
pub trait Packet: Sized {
    /// Wire packet ID within its [`ConnectionState`] and [`PacketDirection`].
    const ID: i32;
    /// Connection state in which this packet is valid.
    const STATE: ConnectionState;
    /// Direction of travel for this packet.
    const DIRECTION: PacketDirection;

    /// Decodes the packet body (excluding length prefix and packet ID) from `buf`.
    fn decode_body(buf: &mut impl Buf) -> Result<Self>;

    /// Encodes the packet body (excluding length prefix and packet ID) into `buf`.
    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()>;
}
