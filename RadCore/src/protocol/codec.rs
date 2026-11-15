//! Zero-copy Minecraft packet framing, compression, and encoding helpers.
//!
//! ## Uncompressed Wire Format
//! ```text
//! +------------------------+--------------------+------------------------+
//! | Length (VarInt)        | Packet ID (VarInt) | Payload (Bytes)        |
//! +------------------------+--------------------+------------------------+
//! ```
//!
//! ## Compressed Wire Format (after `SetCompression`)
//! ```text
//! +------------------------+------------------------+----------------------------+
//! | Packet Length (VarInt) | Data Length (VarInt)   | Compressed/Raw (ID+Body)   |
//! +------------------------+------------------------+----------------------------+
//! ```
//! If `Data Length == 0`, the remaining bytes are uncompressed `(Packet ID + Payload)`.
//! Otherwise `Data Length` is the expected uncompressed length of the zlib stream.

use std::io::{Read, Write};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::error::{Error, Result};
use crate::packet::Packet;
use crate::types::{Decode, Encode, VarInt};

/// Default maximum allowed packet frame size (2 MiB).
pub const DEFAULT_MAX_PACKET_SIZE: usize = 2 * 1024 * 1024;

/// Default maximum allowed uncompressed payload size (8 MiB, zip-bomb protection).
pub const DEFAULT_MAX_UNCOMPRESSED_SIZE: usize = 8 * 1024 * 1024;

/// Configuration for Minecraft `zlib` packet compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionConfig {
    /// Minimum uncompressed `(Packet ID + Payload)` size in bytes to trigger zlib compression.
    pub threshold: usize,
    /// Maximum allowed uncompressed payload size (protects against zip-bombs).
    pub max_uncompressed_size: usize,
}

impl CompressionConfig {
    /// Creates a new [`CompressionConfig`] with the given `threshold` and default max uncompressed size.
    #[must_use]
    pub const fn new(threshold: usize) -> Self {
        Self {
            threshold,
            max_uncompressed_size: DEFAULT_MAX_UNCOMPRESSED_SIZE,
        }
    }
}

/// A zero-copy decoded packet frame containing its `id` and raw `payload`.
///
/// `payload` is an immutable [`Bytes`] slice backed by the original receive buffer
/// (or the decompressed buffer when compression is active).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPacket {
    /// The decoded packet ID.
    pub id: i32,
    /// The remaining packet payload bytes after the packet ID.
    pub payload: Bytes,
}

impl RawPacket {
    /// Decodes a strongly-typed [`Packet`] from this raw frame's payload.
    ///
    /// Returns [`Error::InvalidPacket`] if the raw packet ID does not match `P::ID`
    /// or if unread trailing bytes remain after decoding the body.
    pub fn decode_as<P: Packet>(&self) -> Result<P> {
        if self.id != P::ID {
            return Err(Error::InvalidPacket(format!(
                "packet ID mismatch: expected {:#04x}, got {:#04x}",
                P::ID,
                self.id
            )));
        }
        let mut cursor = self.payload.clone();
        let packet = P::decode_body(&mut cursor)?;
        if cursor.has_remaining() {
            return Err(Error::InvalidPacket(format!(
                "packet {:#04x} has {} trailing unread byte(s)",
                P::ID,
                cursor.remaining()
            )));
        }
        Ok(packet)
    }
}

/// Attempts to decode one complete uncompressed packet frame from `src`.
///
/// Shorthand for [`decode_frame_with_compression(src, max_packet_size, None)`](decode_frame_with_compression).
pub fn decode_frame(src: &mut BytesMut, max_packet_size: usize) -> Result<Option<RawPacket>> {
    decode_frame_with_compression(src, max_packet_size, None)
}

/// Attempts to decode one complete packet frame from `src`, optionally handling `zlib` compression.
///
/// - Returns `Ok(None)` if `src` does not yet contain a complete frame (without consuming `src`).
/// - Returns `Ok(Some(RawPacket))` and advances `src` past the frame when a full packet is present.
/// - Returns `Err(...)` on malformed headers, oversized frames, or zip-bomb payloads.
pub fn decode_frame_with_compression(
    src: &mut BytesMut,
    max_packet_size: usize,
    compression: Option<CompressionConfig>,
) -> Result<Option<RawPacket>> {
    let mut peek = &src[..];

    let frame_len = match VarInt::decode(&mut peek) {
        Ok(v) => v.0,
        Err(Error::UnexpectedEof { .. }) => return Ok(None),
        Err(e) => return Err(e),
    };

    if frame_len <= 0 || (frame_len as usize) > max_packet_size {
        return Err(Error::InvalidPacketLength {
            length: frame_len,
            max: max_packet_size,
        });
    }

    let frame_len = frame_len as usize;
    let header_len = src.len() - peek.len();

    if peek.len() < frame_len {
        return Ok(None);
    }

    // Consume header + frame from `src` without copying payload bytes.
    src.advance(header_len);
    let frame_bytes = src.split_to(frame_len).freeze();
    decode_frame_body(frame_bytes, compression).map(Some)
}

/// Decodes a single complete raw frame slice (including leading length VarInt) without consuming
/// a `BytesMut`, applying bounded zlib decompression if `compression` is enabled.
pub fn decode_raw_frame_slice(
    raw_frame: &[u8],
    max_packet_size: usize,
    compression: Option<CompressionConfig>,
) -> Result<RawPacket> {
    let mut peek = raw_frame;
    let frame_len = VarInt::decode(&mut peek)?.0;
    if frame_len <= 0 || (frame_len as usize) > max_packet_size {
        return Err(Error::InvalidPacketLength {
            length: frame_len,
            max: max_packet_size,
        });
    }
    let frame_len = frame_len as usize;
    if peek.len() < frame_len {
        return Err(Error::UnexpectedEof {
            needed: frame_len - peek.len(),
        });
    }
    let frame_bytes = Bytes::copy_from_slice(&peek[..frame_len]);
    decode_frame_body(frame_bytes, compression)
}

fn decode_frame_body(
    mut frame_bytes: Bytes,
    compression: Option<CompressionConfig>,
) -> Result<RawPacket> {
    let Some(comp) = compression else {
        let packet_id = VarInt::decode(&mut frame_bytes)?.0;
        return Ok(RawPacket {
            id: packet_id,
            payload: frame_bytes,
        });
    };

    // Compressed framing: Data Length (VarInt) precedes (Packet ID + Payload).
    let data_len = VarInt::decode(&mut frame_bytes)?.0;
    if data_len < 0 {
        return Err(Error::Compression(format!(
            "negative uncompressed length: {data_len}"
        )));
    }

    if data_len == 0 {
        let packet_id = VarInt::decode(&mut frame_bytes)?.0;
        return Ok(RawPacket {
            id: packet_id,
            payload: frame_bytes,
        });
    }

    let expected_len = data_len as usize;
    if expected_len > comp.max_uncompressed_size {
        return Err(Error::Compression(format!(
            "uncompressed length {expected_len} exceeds limit {}",
            comp.max_uncompressed_size
        )));
    }
    if expected_len < comp.threshold {
        return Err(Error::Compression(format!(
            "compressed packet size {expected_len} is below threshold {}",
            comp.threshold
        )));
    }

    let mut decoder = ZlibDecoder::new(&frame_bytes[..]).take((expected_len + 1) as u64);
    let mut decompressed = BytesMut::with_capacity(expected_len);
    let mut chunk = [0u8; 4096];

    loop {
        let read = decoder
            .read(&mut chunk)
            .map_err(|e| Error::Compression(e.to_string()))?;
        if read == 0 {
            break;
        }
        decompressed.extend_from_slice(&chunk[..read]);
        if decompressed.len() > expected_len {
            return Err(Error::Compression(
                "decompressed stream exceeded declared Data Length".into(),
            ));
        }
    }

    if decompressed.len() != expected_len {
        return Err(Error::Compression(format!(
            "decompressed length {} != declared {expected_len}",
            decompressed.len()
        )));
    }

    let mut frozen = decompressed.freeze();
    let packet_id = VarInt::decode(&mut frozen)?.0;
    Ok(RawPacket {
        id: packet_id,
        payload: frozen,
    })
}

/// Encodes an uncompressed raw packet ID and payload into `dst`.
pub fn encode_frame(packet_id: i32, payload: &[u8], dst: &mut BytesMut) -> Result<()> {
    encode_frame_with_compression(packet_id, payload, dst, None)
}

/// Encodes a raw packet ID and payload into `dst`, optionally compressing with `zlib`
/// when `compression_threshold` is `Some(threshold)` and the uncompressed size meets it.
pub fn encode_frame_with_compression(
    packet_id: i32,
    payload: &[u8],
    dst: &mut BytesMut,
    compression_threshold: Option<usize>,
) -> Result<()> {
    let id = VarInt(packet_id);
    let uncompressed_len = id.encoded_size() + payload.len();

    let Some(threshold) = compression_threshold else {
        let total_len_i32 =
            i32::try_from(uncompressed_len).map_err(|_| Error::InvalidPacketLength {
                length: i32::MAX,
                max: DEFAULT_MAX_PACKET_SIZE,
            })?;
        let prefix_len = VarInt(total_len_i32).encoded_size();
        dst.reserve(prefix_len + uncompressed_len);
        VarInt(total_len_i32).encode(dst)?;
        id.encode(dst)?;
        dst.put_slice(payload);
        return Ok(());
    };

    if uncompressed_len < threshold {
        // Data Length = 0 (uncompressed inside compressed framing).
        let data_len = VarInt(0);
        let frame_len = data_len.encoded_size() + uncompressed_len;
        let frame_len_i32 = i32::try_from(frame_len).map_err(|_| Error::InvalidPacketLength {
            length: i32::MAX,
            max: DEFAULT_MAX_PACKET_SIZE,
        })?;
        dst.reserve(VarInt(frame_len_i32).encoded_size() + frame_len);
        VarInt(frame_len_i32).encode(dst)?;
        data_len.encode(dst)?;
        id.encode(dst)?;
        dst.put_slice(payload);
        return Ok(());
    }

    let data_len_i32 = i32::try_from(uncompressed_len).map_err(|_| Error::InvalidPacketLength {
        length: i32::MAX,
        max: DEFAULT_MAX_PACKET_SIZE,
    })?;

    let mut encoder = ZlibEncoder::new(Vec::with_capacity(uncompressed_len), Compression::fast());
    let mut id_buf = [0u8; VarInt::MAX_BYTES];
    let mut id_slice = &mut id_buf[..];
    id.encode(&mut id_slice)?;
    let id_len = id.encoded_size();

    encoder
        .write_all(&id_buf[..id_len])
        .map_err(|e| Error::Compression(e.to_string()))?;
    encoder
        .write_all(payload)
        .map_err(|e| Error::Compression(e.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|e| Error::Compression(e.to_string()))?;

    let data_len = VarInt(data_len_i32);
    let frame_len = data_len.encoded_size() + compressed.len();
    let frame_len_i32 = i32::try_from(frame_len).map_err(|_| Error::InvalidPacketLength {
        length: i32::MAX,
        max: DEFAULT_MAX_PACKET_SIZE,
    })?;

    dst.reserve(VarInt(frame_len_i32).encoded_size() + frame_len);
    VarInt(frame_len_i32).encode(dst)?;
    data_len.encode(dst)?;
    dst.put_slice(&compressed);
    Ok(())
}

/// Encodes a strongly-typed [`Packet`] into `dst` (uncompressed).
pub fn encode_packet<P: Packet>(packet: &P, dst: &mut BytesMut) -> Result<()> {
    encode_packet_with_compression(packet, dst, None)
}

/// Encodes a strongly-typed [`Packet`] into `dst` with optional `zlib` compression.
pub fn encode_packet_with_compression<P: Packet>(
    packet: &P,
    dst: &mut BytesMut,
    compression_threshold: Option<usize>,
) -> Result<()> {
    let mut body = BytesMut::new();
    packet.encode_body(&mut body)?;
    encode_frame_with_compression(P::ID, &body, dst, compression_threshold)
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;

    use super::*;
    use crate::packet::status::{PingRequest, StatusResponse};

    #[test]
    fn partial_frame_returns_none_without_consuming() {
        let mut buf = BytesMut::new();
        encode_packet(&PingRequest { timestamp: 42 }, &mut buf).unwrap();

        let mut partial = buf.split_to(buf.len() - 1);
        let original_len = partial.len();

        assert!(
            decode_frame(&mut partial, DEFAULT_MAX_PACKET_SIZE)
                .unwrap()
                .is_none()
        );
        assert_eq!(partial.len(), original_len);
    }

    #[test]
    fn frame_roundtrip() {
        let mut buf = BytesMut::new();
        let ping = PingRequest {
            timestamp: 123_456_789,
        };
        encode_packet(&ping, &mut buf).unwrap();

        let raw = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE)
            .unwrap()
            .expect("expected complete frame");
        assert_eq!(raw.id, PingRequest::ID);
        let decoded: PingRequest = raw.decode_as().unwrap();
        assert_eq!(decoded, ping);
        assert!(buf.is_empty());
    }

    #[test]
    fn compressed_below_and_above_threshold_roundtrip() {
        let comp = CompressionConfig::new(32);

        // 1. Below threshold (PingRequest is 9 bytes total < 32).
        let mut buf = BytesMut::new();
        let ping = PingRequest { timestamp: 999 };
        encode_packet_with_compression(&ping, &mut buf, Some(comp.threshold)).unwrap();
        let raw = decode_frame_with_compression(&mut buf, DEFAULT_MAX_PACKET_SIZE, Some(comp))
            .unwrap()
            .unwrap();
        assert_eq!(raw.decode_as::<PingRequest>().unwrap(), ping);

        // 2. Above threshold (large StatusResponse).
        let big = StatusResponse {
            json: "A".repeat(512),
        };
        encode_packet_with_compression(&big, &mut buf, Some(comp.threshold)).unwrap();
        let raw = decode_frame_with_compression(&mut buf, DEFAULT_MAX_PACKET_SIZE, Some(comp))
            .unwrap()
            .unwrap();
        assert_eq!(raw.decode_as::<StatusResponse>().unwrap(), big);
    }

    #[test]
    fn decode_as_rejects_id_mismatch_and_trailing_bytes() {
        let raw_wrong_id = RawPacket {
            id: PingRequest::ID + 1,
            payload: Bytes::copy_from_slice(&0i64.to_be_bytes()),
        };
        assert!(raw_wrong_id.decode_as::<PingRequest>().is_err());

        let raw_trailing = RawPacket {
            id: PingRequest::ID,
            payload: Bytes::from_static(&[0, 0, 0, 0, 0, 0, 0, 1, 0xFF]),
        };
        assert!(raw_trailing.decode_as::<PingRequest>().is_err());
    }
}
