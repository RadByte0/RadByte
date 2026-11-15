//! Error types for `rad-core`.

use thiserror::Error;

/// A specialized `Result` type for `rad-core` operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur when decoding, encoding, or validating Minecraft protocol data.
#[derive(Debug, Error)]
pub enum Error {
    /// Not enough bytes were available in the buffer to complete reading a value.
    #[error("unexpected end of buffer (needed {needed} more bytes, or incomplete frame)")]
    UnexpectedEof {
        /// Minimum number of additional bytes expected, if known.
        needed: usize,
    },

    /// A `VarInt` exceeded the maximum allowed 5-byte encoding.
    #[error("VarInt exceeded maximum length of 5 bytes")]
    VarIntTooLong,

    /// A `VarLong` exceeded the maximum allowed 10-byte encoding.
    #[error("VarLong exceeded maximum length of 10 bytes")]
    VarLongTooLong,

    /// A protocol string exceeded its allowed character/byte limit or was invalid UTF-8.
    #[error("invalid string: {0}")]
    InvalidString(String),

    /// A packet failed structural validation (e.g. ID mismatch, trailing bytes, or count limit).
    #[error("invalid packet: {0}")]
    InvalidPacket(String),

    /// A packet frame length was negative or exceeded the configured maximum.
    #[error("invalid packet length {length} (max allowed: {max})")]
    InvalidPacketLength {
        /// The decoded packet length.
        length: i32,
        /// The configured maximum packet length.
        max: usize,
    },

    /// An unknown or unexpected packet ID was encountered for the current state.
    #[error("unexpected packet id 0x{id:02X} in state {state}")]
    UnexpectedPacketId {
        /// The raw packet ID.
        id: i32,
        /// The connection state name.
        state: &'static str,
    },

    /// An invalid connection state transition or state ID was requested.
    #[error("invalid connection state id: {0}")]
    InvalidState(i32),

    /// Failed to serialize or deserialize JSON payload (e.g. StatusResponse or TextComponent).
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Compression or decompression failed, or exceeded uncompressed size limit.
    #[error("compression error: {0}")]
    Compression(String),

    /// Invalid or overly deep NBT payload.
    #[error("nbt error: {0}")]
    Nbt(String),

    /// Cryptographic or forwarding signature verification error.
    #[error("crypto/forwarding error: {0}")]
    Crypto(String),

    /// An I/O error occurred.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
