//! Login state packets (authentication, compression, and transition to Configuration).

use bytes::{Buf, BufMut};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::packet::{Packet, PacketDirection};
use crate::state::ConnectionState;
use crate::text::TextComponent;
use crate::types::{Decode, Encode, VarInt, read_string, write_string};

fn read_varint_bytes(buf: &mut impl Buf, max_len: usize) -> Result<Vec<u8>> {
    let len = VarInt::decode(buf)?.0;
    if len < 0 || (len as usize) > max_len {
        return Err(Error::InvalidPacketLength {
            length: len,
            max: max_len,
        });
    }
    let len = len as usize;
    if buf.remaining() < len {
        return Err(Error::UnexpectedEof {
            needed: len - buf.remaining(),
        });
    }
    let mut out = vec![0u8; len];
    buf.copy_to_slice(&mut out);
    Ok(out)
}

fn write_varint_bytes(bytes: &[u8], buf: &mut impl BufMut) -> Result<()> {
    VarInt(i32::try_from(bytes.len()).unwrap_or(i32::MAX)).encode(buf)?;
    buf.put_slice(bytes);
    Ok(())
}

/// Serverbound `Login Start` packet (`0x00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStart {
    /// Player's username (max 16 chars).
    pub username: String,
    /// Player's UUID (1.20.2+ always sends this field).
    pub player_uuid: Uuid,
}

impl LoginStart {
    /// Maximum Minecraft username length.
    pub const MAX_USERNAME_LEN: usize = 16;
}

impl Packet for LoginStart {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let username = read_string(buf, Self::MAX_USERNAME_LEN)?;
        let player_uuid = Uuid::decode(buf)?;
        Ok(Self {
            username,
            player_uuid,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.username, Self::MAX_USERNAME_LEN)?;
        self.player_uuid.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Login Disconnect` packet (`0x00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDisconnect {
    /// JSON-encoded disconnect reason shown to the client.
    pub reason_json: String,
}

impl LoginDisconnect {
    /// Creates a [`LoginDisconnect`] from a [`TextComponent`].
    pub fn from_text(component: &TextComponent) -> Result<Self> {
        Ok(Self {
            reason_json: component.to_json()?,
        })
    }
}

impl Packet for LoginDisconnect {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            reason_json: String::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.reason_json.encode(buf)
    }
}

/// A signed Mojang profile property (e.g., `"textures"` for player skins).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfileProperty {
    /// Property name (e.g., `"textures"`).
    pub name: String,
    /// Base64-encoded property value.
    pub value: String,
    /// Optional Mojang Yggdrasil signature.
    pub signature: Option<String>,
}

/// Clientbound `Login Success` / `login_finished` packet (`0x02`).
///
/// In Minecraft 26.2+ (including 26.3 / protocol `777`), `login_finished` carries
/// the player's game profile (`uuid`, `username`, `properties`) followed by a
/// 16-byte `session_id` (`UUID`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSuccess {
    /// Authenticated or offline player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub username: String,
    /// Profile properties (skins, capes).
    pub properties: Vec<GameProfileProperty>,
    /// Server session UUID (16 bytes, introduced in 26.2+).
    pub session_id: Uuid,
}

impl Packet for LoginSuccess {
    const ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let uuid = Uuid::decode(buf)?;
        let username = read_string(buf, LoginStart::MAX_USERNAME_LEN)?;
        let raw_count = VarInt::decode(buf)?.0;
        let count = usize::try_from(raw_count).map_err(|_| {
            crate::error::Error::InvalidPacket(format!("negative property count: {raw_count}"))
        })?;
        if count > 64 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "property count {count} exceeds limit 64"
            )));
        }
        let mut properties = Vec::with_capacity(count);
        for _ in 0..count {
            let name = read_string(buf, 64)?;
            let value = String::decode(buf)?;
            let signature = Option::<String>::decode(buf)?;
            properties.push(GameProfileProperty {
                name,
                value,
                signature,
            });
        }
        let session_id = Uuid::decode(buf)?;
        Ok(Self {
            uuid,
            username,
            properties,
            session_id,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.uuid.encode(buf)?;
        write_string(buf, &self.username, LoginStart::MAX_USERNAME_LEN)?;
        VarInt(self.properties.len() as i32).encode(buf)?;
        for prop in &self.properties {
            write_string(buf, &prop.name, 64)?;
            prop.value.encode(buf)?;
            prop.signature.encode(buf)?;
        }
        self.session_id.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Set Compression` packet (`0x03`).
///
/// Enables `zlib` compression for all subsequent packets in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCompression {
    /// Compression threshold in bytes (`-1` disables compression).
    pub threshold: VarInt,
}

impl Packet for SetCompression {
    const ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            threshold: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.threshold.encode(buf)
    }
}

/// Serverbound `Login Acknowledged` packet (`0x03`).
///
/// Sent by the client after receiving [`LoginSuccess`] to transition into [`ConnectionState::Configuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoginAcknowledged;

impl Packet for LoginAcknowledged {
    const ID: i32 = 0x03;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// Clientbound `Login Plugin Request` packet (`0x04`).
///
/// Used for Velocity Modern Forwarding (`velocity:player_info`) and custom login negotiation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginPluginRequest {
    /// Unique message ID matched by [`LoginPluginResponse`].
    pub message_id: VarInt,
    /// Channel identifier (e.g., `"velocity:player_info"`).
    pub channel: String,
    /// Raw channel payload bytes (rest of packet).
    pub data: Vec<u8>,
}

impl Packet for LoginPluginRequest {
    const ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let message_id = VarInt::decode(buf)?;
        let channel = read_string(buf, 32_767)?;
        let mut data = vec![0u8; buf.remaining()];
        buf.copy_to_slice(&mut data);
        Ok(Self {
            message_id,
            channel,
            data,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.message_id.encode(buf)?;
        write_string(buf, &self.channel, 32_767)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

/// Serverbound `Login Plugin Response` packet (`0x04`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginPluginResponse {
    /// Message ID from the corresponding [`LoginPluginRequest`].
    pub message_id: VarInt,
    /// Whether the client/proxy understood the channel.
    pub successful: bool,
    /// Optional response payload bytes.
    pub data: Vec<u8>,
}

impl Packet for LoginPluginResponse {
    const ID: i32 = 0x02;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let message_id = VarInt::decode(buf)?;
        let successful = bool::decode(buf)?;
        let mut data = vec![0u8; buf.remaining()];
        buf.copy_to_slice(&mut data);
        Ok(Self {
            message_id,
            successful,
            data,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.message_id.encode(buf)?;
        self.successful.encode(buf)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

/// Clientbound `Encryption Request` packet (`0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionRequest {
    /// Server ID string (usually empty `""` in modern Minecraft).
    pub server_id: String,
    /// Server's DER-encoded ASN.1 RSA public key.
    pub public_key: Vec<u8>,
    /// Random verify token (4 bytes).
    pub verify_token: Vec<u8>,
    /// Whether the client should authenticate with Mojang's session server (`true` for online-mode).
    pub should_authenticate: bool,
}

impl Packet for EncryptionRequest {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let server_id = read_string(buf, 20)?;
        let public_key = read_varint_bytes(buf, 4096)?;
        let verify_token = read_varint_bytes(buf, 256)?;
        let should_authenticate = bool::decode(buf)?;
        Ok(Self {
            server_id,
            public_key,
            verify_token,
            should_authenticate,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.server_id, 20)?;
        write_varint_bytes(&self.public_key, buf)?;
        write_varint_bytes(&self.verify_token, buf)?;
        self.should_authenticate.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Encryption Response` packet (`0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionResponse {
    /// RSA-encrypted 16-byte shared secret.
    pub shared_secret: Vec<u8>,
    /// RSA-encrypted verify token.
    pub verify_token: Vec<u8>,
}

impl Packet for EncryptionResponse {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Login;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let shared_secret = read_varint_bytes(buf, 1024)?;
        let verify_token = read_varint_bytes(buf, 1024)?;
        Ok(Self {
            shared_secret,
            verify_token,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_varint_bytes(&self.shared_secret, buf)?;
        write_varint_bytes(&self.verify_token, buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;

    #[test]
    fn login_success_roundtrip() {
        let pkt = LoginSuccess {
            uuid: Uuid::nil(),
            username: "RadPlayer".into(),
            properties: vec![GameProfileProperty {
                name: "textures".into(),
                value: "e30=".into(),
                signature: Some("sig".into()),
            }],
            session_id: Uuid::from_u128(1),
        };
        let mut buf = BytesMut::new();
        pkt.encode_body(&mut buf).unwrap();
        let decoded = LoginSuccess::decode_body(&mut buf).unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn encryption_packets_roundtrip() {
        let req = EncryptionRequest {
            server_id: String::new(),
            public_key: vec![1, 2, 3, 4],
            verify_token: vec![9, 8, 7, 6],
            should_authenticate: true,
        };
        let mut buf = BytesMut::new();
        req.encode_body(&mut buf).unwrap();
        assert_eq!(EncryptionRequest::decode_body(&mut buf).unwrap(), req);

        let resp = EncryptionResponse {
            shared_secret: vec![0xAA; 128],
            verify_token: vec![0xBB; 128],
        };
        resp.encode_body(&mut buf).unwrap();
        assert_eq!(EncryptionResponse::decode_body(&mut buf).unwrap(), resp);
    }
}
