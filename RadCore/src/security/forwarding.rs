//! Velocity Modern Forwarding (`HMAC-SHA256`) encoding and verification.
//!
//! Allows `RadProxy` to securely pass the player's real IP address, UUID, username,
//! and profile properties to `RadLimbo`, `RadServer`, or Paper/Velocity backends
//! over the `velocity:player_info` login plugin channel.

use bytes::{Buf, BytesMut};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::packet::login::GameProfileProperty;
use crate::types::{Decode, Encode, VarInt, read_string, write_string};

type HmacSha256 = Hmac<Sha256>;

/// Plugin channel identifier used by Velocity Modern Forwarding.
pub const VELOCITY_FORWARDING_CHANNEL: &str = "velocity:player_info";

/// Default Velocity Modern Forwarding protocol version (`1`).
pub const VELOCITY_FORWARDING_VERSION: i32 = 1;

/// Player identity payload signed and forwarded by `RadProxy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardingData {
    /// Player's real IP address string (without port).
    pub address: String,
    /// Player's UUID.
    pub uuid: Uuid,
    /// Player's username.
    pub username: String,
    /// Signed Mojang profile properties (skins/capes).
    pub properties: Vec<GameProfileProperty>,
}

/// Encodes and signs [`ForwardingData`] with `secret` using `HMAC-SHA256`.
///
/// Wire layout: `[32-byte HMAC-SHA256] [version (VarInt)] [address] [uuid] [username] [properties]`.
pub fn encode_velocity_forwarding(data: &ForwardingData, secret: &[u8]) -> Result<Vec<u8>> {
    if secret.is_empty() {
        return Err(Error::Crypto("forwarding secret must not be empty".into()));
    }
    if data.properties.len() > 64 {
        return Err(Error::Crypto(
            "forwarding property count exceeds limit 64".into(),
        ));
    }
    let mut payload = BytesMut::with_capacity(128);
    VarInt(VELOCITY_FORWARDING_VERSION).encode(&mut payload)?;
    write_string(&mut payload, &data.address, 255)?;
    data.uuid.encode(&mut payload)?;
    write_string(&mut payload, &data.username, 16)?;
    VarInt(data.properties.len() as i32).encode(&mut payload)?;
    for prop in &data.properties {
        write_string(&mut payload, &prop.name, 64)?;
        prop.value.encode(&mut payload)?;
        prop.signature.encode(&mut payload)?;
    }

    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|e| Error::Crypto(format!("invalid forwarding secret: {e}")))?;
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();

    let mut out = Vec::with_capacity(32 + payload.len());
    out.extend_from_slice(&signature);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Verifies the `HMAC-SHA256` signature and decodes [`ForwardingData`].
pub fn decode_velocity_forwarding(raw: &[u8], secret: &[u8]) -> Result<ForwardingData> {
    if secret.is_empty() {
        return Err(Error::Crypto("forwarding secret must not be empty".into()));
    }
    if raw.len() < 32 {
        return Err(Error::Crypto(
            "forwarding payload shorter than 32-byte HMAC".into(),
        ));
    }

    let (signature, mut payload) = raw.split_at(32);
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|e| Error::Crypto(format!("invalid forwarding secret: {e}")))?;
    mac.update(payload);
    mac.verify_slice(signature)
        .map_err(|_| Error::Crypto("invalid Velocity forwarding HMAC signature".into()))?;

    let version = VarInt::decode(&mut payload)?.0;
    if version != VELOCITY_FORWARDING_VERSION {
        return Err(Error::Crypto(format!(
            "unsupported forwarding version: {version}"
        )));
    }

    let address = read_string(&mut payload, 255)?;
    let uuid = Uuid::decode(&mut payload)?;
    let username = read_string(&mut payload, 16)?;
    let raw_count = VarInt::decode(&mut payload)?.0;
    let count = usize::try_from(raw_count)
        .map_err(|_| Error::Crypto(format!("negative forwarding property count: {raw_count}")))?;
    if count > 64 {
        return Err(Error::Crypto(format!(
            "forwarding property count {count} exceeds limit 64"
        )));
    }
    let mut properties = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_string(&mut payload, 64)?;
        let value = String::decode(&mut payload)?;
        let signature = Option::<String>::decode(&mut payload)?;
        properties.push(GameProfileProperty {
            name,
            value,
            signature,
        });
    }

    if payload.has_remaining() {
        return Err(Error::Crypto("trailing bytes in forwarding payload".into()));
    }

    Ok(ForwardingData {
        address,
        uuid,
        username,
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_secrets_and_excessive_properties() {
        let mut data = ForwardingData {
            address: "127.0.0.1".into(),
            uuid: Uuid::nil(),
            username: "Player".into(),
            properties: Vec::new(),
        };
        assert!(encode_velocity_forwarding(&data, b"").is_err());
        assert!(decode_velocity_forwarding(&[0; 32], b"").is_err());
        data.properties = vec![
            GameProfileProperty {
                name: "test".into(),
                value: "value".into(),
                signature: None
            };
            65
        ];
        assert!(encode_velocity_forwarding(&data, b"test-key").is_err());
    }

    #[test]
    fn velocity_forwarding_roundtrip_and_tamper_check() {
        let secret = b"super-secret-radbyte-key";
        let data = ForwardingData {
            address: "203.0.113.42".into(),
            uuid: Uuid::nil(),
            username: "RadPlayer".into(),
            properties: vec![GameProfileProperty {
                name: "textures".into(),
                value: "e30=".into(),
                signature: None,
            }],
        };

        let mut encoded = encode_velocity_forwarding(&data, secret).unwrap();
        let decoded = decode_velocity_forwarding(&encoded, secret).unwrap();
        assert_eq!(decoded, data);

        // Tamper with a byte and ensure HMAC verification fails.
        encoded[35] ^= 0xFF;
        assert!(decode_velocity_forwarding(&encoded, secret).is_err());
    }
}
