//! # `rad-core`
//!
//! Core zero-copy Minecraft protocol, packet framing, and shared primitives for the
//! **RadByte** ecosystem (`RadProxy`, `RadLimbo`, `RadServer`).
//!
//! ## Architecture & Extensibility
//! - [`types`] — Primitive wire types (`VarInt`, `VarLong`, strings, `Uuid`) and the
//!   [`Decode`](types::Decode) / [`Encode`](types::Encode) traits over [`bytes::Buf`] / [`bytes::BufMut`].
//! - [`codec`] — Zero-copy packet framing ([`RawPacket`](codec::RawPacket), [`decode_frame`](codec::decode_frame),
//!   [`encode_packet`](codec::encode_packet)).
//! - [`packet`] — Strongly-typed packets grouped by [`ConnectionState`](state::ConnectionState) via the
//!   [`Packet`](packet::Packet) trait. Easy to extend with custom or new version packets.
//! - [`text`] — Ergonomic builder for Minecraft JSON [`TextComponent`](text::TextComponent).
//! - [`state`] — Protocol [`ConnectionState`](state::ConnectionState) enum and handshake transitions.
//! - [`error`] — Unified [`Error`](error::Error) and [`Result`](error::Result) types.
//!
//! ## Quick Example
//! ```
//! use bytes::BytesMut;
//! use rad_core::prelude::*;
//! use rad_core::packet::handshake::Handshake;
//!
//! let mut buf = BytesMut::new();
//! let hs = Handshake {
//!     protocol_version: VarInt(PROTOCOL_VERSION),
//!     server_address: "play.radbyte.xyz".into(),
//!     server_port: 25565,
//!     next_state: ConnectionState::Status,
//! };
//!
//! encode_packet(&hs, &mut buf).unwrap();
//!
//! let frame = decode_frame(&mut buf, DEFAULT_MAX_PACKET_SIZE).unwrap().unwrap();
//! let decoded: Handshake = frame.decode_as().unwrap();
//! assert_eq!(decoded, hs);
//! ```

#![warn(missing_docs)]

/// Target Minecraft Java Edition version string.
pub const MINECRAFT_VERSION: &str = "26.3";

/// Target Minecraft Java Edition network protocol version (`26.3`).
pub const PROTOCOL_VERSION: i32 = 777;

pub mod app;
pub mod client;
pub mod packet;
pub mod protocol;
pub mod registry_provider;
pub mod security;
pub mod version_package;

pub use app::{api_runtime, auto_update, cli, console_ui, error, radon, text};
pub use protocol::{codec, nbt, state, types, version};
pub use security::{crypto, forwarding};
pub use version::GameVersion;

/// Convenient re-exports of the most commonly used `rad-core` types and functions.
pub mod prelude {
    pub use crate::codec::{
        CompressionConfig, DEFAULT_MAX_PACKET_SIZE, DEFAULT_MAX_UNCOMPRESSED_SIZE, RawPacket,
        decode_frame, decode_frame_with_compression, encode_frame, encode_frame_with_compression,
        encode_packet, encode_packet_with_compression,
    };
    pub use crate::crypto::{MinecraftCipher, offline_player_uuid};
    pub use crate::error::{Error, Result};
    pub use crate::forwarding::{
        ForwardingData, VELOCITY_FORWARDING_CHANNEL, decode_velocity_forwarding,
        encode_velocity_forwarding,
    };
    pub use crate::nbt::{NbtTag, decode_network_nbt, encode_network_nbt};
    pub use crate::packet::{Packet, PacketDirection};
    pub use crate::radon::{RadonManifest, RadonTarget};
    pub use crate::state::ConnectionState;
    pub use crate::text::TextComponent;
    pub use crate::types::{BlockPos, Decode, Encode, VarInt, VarLong, read_string, write_string};
    pub use crate::version::{GameVersion, ProtocolVersion};
    pub use crate::{MINECRAFT_VERSION, PROTOCOL_VERSION};
}
