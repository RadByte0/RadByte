//! Status (Server List Ping) state packets.

use bytes::{Buf, BufMut};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::packet::{Packet, PacketDirection};
use crate::state::ConnectionState;
use crate::text::TextComponent;
use crate::types::{Decode, Encode};

/// Serverbound `Status Request` packet (`0x00`).
///
/// Empty body; requests the server's [`StatusResponse`] JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusRequest;

impl Packet for StatusRequest {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// Serverbound `Ping Request` packet (`0x01`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingRequest {
    /// Client-chosen payload (usually epoch milliseconds) echoed back in [`PongResponse`].
    pub timestamp: i64,
}

impl Packet for PingRequest {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            timestamp: i64::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.timestamp.encode(buf)
    }
}

/// Clientbound `Pong Response` packet (`0x01`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PongResponse {
    /// Echoed timestamp from [`PingRequest`].
    pub timestamp: i64,
}

impl Packet for PongResponse {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            timestamp: i64::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.timestamp.encode(buf)
    }
}

/// Version metadata inside a [`StatusPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusVersion {
    /// Display name of the version (e.g., `"RadByte 26.3"`).
    pub name: String,
    /// Protocol version number.
    pub protocol: i32,
}

/// A single player entry in the hover sample list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSample {
    /// Player display name.
    pub name: String,
    /// Player UUID.
    pub id: Uuid,
}

/// Online/max player counts inside a [`StatusPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPlayers {
    /// Maximum player slots.
    pub max: i32,
    /// Currently online players.
    pub online: i32,
    /// Optional sample of online players shown on hover.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample: Vec<PlayerSample>,
}

/// Structured JSON payload for [`StatusResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusPayload {
    /// Server version information.
    pub version: StatusVersion,
    /// Player count and optional sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub players: Option<StatusPlayers>,
    /// Server MOTD description.
    pub description: TextComponent,
    /// Optional base64-encoded 64x64 PNG (`data:image/png;base64,...`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
    /// Whether the server enforces secure chat profiles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforces_secure_chat: Option<bool>,
}

/// Clientbound `Status Response` packet (`0x00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    /// Raw JSON string sent over the wire.
    pub json: String,
}

impl StatusResponse {
    /// Creates a [`StatusResponse`] by serializing a structured [`StatusPayload`].
    pub fn from_payload(payload: &StatusPayload) -> Result<Self> {
        Ok(Self {
            json: serde_json::to_string(payload)?,
        })
    }

    /// Parses the inner JSON into a structured [`StatusPayload`].
    pub fn parse_payload(&self) -> Result<StatusPayload> {
        Ok(serde_json::from_str(&self.json)?)
    }
}

impl Packet for StatusResponse {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Status;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            json: String::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.json.encode(buf)
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    #[test]
    fn status_response_roundtrip() {
        let payload = StatusPayload {
            version: StatusVersion {
                name: "RadByte 1.21.4".into(),
                protocol: 769,
            },
            players: Some(StatusPlayers {
                max: 1000,
                online: 42,
                sample: vec![],
            }),
            description: TextComponent::text("RadByte Ecosystem").color("aqua"),
            favicon: None,
            enforces_secure_chat: Some(false),
        };

        let packet = StatusResponse::from_payload(&payload).unwrap();
        let mut buf = BytesMut::new();
        packet.encode_body(&mut buf).unwrap();

        let decoded = StatusResponse::decode_body(&mut buf).unwrap();
        let parsed = decoded.parse_payload().unwrap();
        assert_eq!(parsed, payload);
    }
}
