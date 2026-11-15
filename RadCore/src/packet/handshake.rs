//! Handshaking state packets.

use bytes::{Buf, BufMut};

use crate::error::Result;
use crate::packet::{Packet, PacketDirection};
use crate::state::ConnectionState;
use crate::types::{Decode, Encode, VarInt, read_string, write_string};

/// Serverbound `Handshake` packet (`0x00`).
///
/// Sent immediately by the client after opening a TCP connection to indicate
/// protocol version, target hostname/port, and the next [`ConnectionState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// Minecraft protocol version number (e.g., `777` for 26.3).
    pub protocol_version: VarInt,
    /// Hostname or IP address used by the client to connect (max 255 chars).
    pub server_address: String,
    /// TCP port used by the client.
    pub server_port: u16,
    /// Requested next state (`1` = Status, `2` = Login, `3` = Transfer).
    pub next_state: ConnectionState,
}

impl Handshake {
    /// Maximum allowed length of `server_address` in the Minecraft protocol.
    pub const MAX_ADDRESS_LEN: usize = 255;

    /// Raw `next_state` integer representation for wire encoding.
    #[must_use]
    pub const fn next_state_id(&self) -> i32 {
        match self.next_state {
            ConnectionState::Status => 1,
            ConnectionState::Login => 2,
            _ => 2,
        }
    }

    /// Zero-allocation fast-path to inspect `next_state` directly from a raw `Handshake` payload.
    pub fn peek_next_state(mut payload: &[u8]) -> Result<ConnectionState> {
        let _protocol_version = VarInt::decode(&mut payload)?;
        let addr_len = VarInt::decode(&mut payload)?.0;
        if addr_len < 0 || (addr_len as usize) > Self::MAX_ADDRESS_LEN * 4 {
            return Err(crate::error::Error::InvalidString(format!(
                "handshake address byte length {addr_len} out of range"
            )));
        }
        let addr_len = addr_len as usize;
        let needed = addr_len + 2;
        if payload.len() < needed {
            return Err(crate::error::Error::UnexpectedEof {
                needed: needed - payload.len(),
            });
        }
        let addr_bytes = &payload[..addr_len];
        let addr = core::str::from_utf8(addr_bytes)
            .map_err(|e| crate::error::Error::InvalidString(e.to_string()))?;
        if addr.encode_utf16().count() > Self::MAX_ADDRESS_LEN {
            return Err(crate::error::Error::InvalidString(format!(
                "handshake address exceeds {} UTF-16 code units",
                Self::MAX_ADDRESS_LEN
            )));
        }
        payload.advance(needed);
        let next_state_raw = VarInt::decode(&mut payload)?.0;
        ConnectionState::from_handshake_next_state(next_state_raw)
    }
}

impl Packet for Handshake {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Handshaking;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let protocol_version = VarInt::decode(buf)?;
        let server_address = read_string(buf, Self::MAX_ADDRESS_LEN)?;
        let server_port = u16::decode(buf)?;
        let next_state_raw = VarInt::decode(buf)?.0;
        let next_state = ConnectionState::from_handshake_next_state(next_state_raw)?;

        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.protocol_version.encode(buf)?;
        write_string(buf, &self.server_address, Self::MAX_ADDRESS_LEN)?;
        self.server_port.encode(buf)?;
        VarInt(self.next_state_id()).encode(buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    #[test]
    fn handshake_roundtrip() {
        let packet = Handshake {
            protocol_version: VarInt(769),
            server_address: "play.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Status,
        };

        let mut buf = BytesMut::new();
        packet.encode_body(&mut buf).unwrap();
        let decoded = Handshake::decode_body(&mut buf).unwrap();
        assert_eq!(decoded, packet);
    }
}
