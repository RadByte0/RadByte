//! Reusable headless Minecraft network client primitives (`Status`, `Login`, `Configuration`, `Play`).
//!
//! Runtime-agnostic (`std::net::TcpStream` with bounded frames and explicit I/O timeouts),
//! suitable for both automated protocol verification and the future `RadClient` network layer,
//! with zero secret/token logging.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use bytes::BytesMut;
use uuid::Uuid;

use crate::PROTOCOL_VERSION;
use crate::codec::{
    CompressionConfig, DEFAULT_MAX_PACKET_SIZE, RawPacket, decode_frame_with_compression,
    encode_packet_with_compression,
};
use crate::crypto::offline_player_uuid;
use crate::error::Error;
use crate::packet::Packet;
use crate::packet::configuration::{
    AcknowledgeFinishConfiguration, ClientboundKnownPacks, ConfigurationDisconnect,
    FinishConfiguration, ServerboundKnownPacks,
};
use crate::packet::handshake::Handshake;
use crate::packet::login::{
    LoginAcknowledged, LoginDisconnect, LoginStart, LoginSuccess, SetCompression,
};
use crate::packet::play::{
    ConfirmTeleportation, KeepAliveClientbound, KeepAliveServerbound, PlayDisconnect, PlayLogin,
    SynchronizePlayerPosition,
};
use crate::packet::status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
use crate::state::ConnectionState;
use crate::types::VarInt;

/// Configuration for [`HeadlessClient`] connections.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Target protocol version sent in [`Handshake`].
    pub protocol_version: i32,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Per-read/write network timeout.
    pub io_timeout: Duration,
    /// Maximum allowed incoming frame size.
    pub max_packet_size: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            connect_timeout: Duration::from_secs(5),
            io_timeout: Duration::from_secs(5),
            max_packet_size: DEFAULT_MAX_PACKET_SIZE,
        }
    }
}

/// Errors returned by [`HeadlessClient`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Underlying I/O error.
    #[error("network I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Protocol decode/encode error.
    #[error("protocol error: {0}")]
    Protocol(#[from] Error),
    /// Remote peer closed connection before completing the handshake/phase.
    #[error("connection closed by peer")]
    UnexpectedEof,
    /// Remote server cleanly disconnected with a reason component.
    #[error("disconnected by server: {0}")]
    Disconnected(String),
    /// Server requested online-mode encryption, which is not configured on this headless client.
    #[error("server requested encryption (online-mode)")]
    EncryptionRequired,
}

/// Result of a Server List Ping (`Status` + `Ping`/`Pong`).
#[derive(Debug, Clone)]
pub struct StatusPingResult {
    /// JSON status payload returned by the server.
    pub json: String,
    /// Roundtrip ping timestamp echo (`timestamp == sent`).
    pub ping_echoed: bool,
}

/// Connected headless client session after reaching `Play` state.
pub struct HeadlessPlaySession {
    stream: TcpStream,
    read_buf: BytesMut,
    compression_threshold: Option<usize>,
    cfg: ClientConfig,
    /// Initial `Login (Play)` packet received upon entering `Play`.
    pub login_play: PlayLogin,
    /// Authenticated/assigned player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub username: String,
}

impl HeadlessPlaySession {
    /// Sends any `Play` serverbound packet.
    pub fn send_packet<P: Packet>(&mut self, packet: &P) -> Result<(), ClientError> {
        send_framed(&mut self.stream, packet, self.compression_threshold)
    }

    /// Reads the next raw `Play` clientbound frame, respecting configured timeouts and compression.
    pub fn next_raw_packet(&mut self) -> Result<RawPacket, ClientError> {
        read_framed(
            &mut self.stream,
            &mut self.read_buf,
            self.compression_threshold,
            self.cfg.max_packet_size,
        )
    }

    /// Handles common background `Play` control packets (`KeepAlive`, `SynchronizePlayerPosition`, `PlayDisconnect`).
    /// Returns `Ok(Some(raw))` for other packets, or `Err(ClientError::Disconnected(..))` on disconnect.
    pub fn poll_event(&mut self) -> Result<Option<RawPacket>, ClientError> {
        let raw = self.next_raw_packet()?;
        match raw.id {
            KeepAliveClientbound::ID => {
                let ka: KeepAliveClientbound = raw.decode_as()?;
                self.send_packet(&KeepAliveServerbound { id: ka.id })?;
                Ok(None)
            }
            SynchronizePlayerPosition::ID => {
                let sync: SynchronizePlayerPosition = raw.decode_as()?;
                self.send_packet(&ConfirmTeleportation {
                    teleport_id: sync.teleport_id,
                    x: sync.x,
                    y: sync.y,
                    z: sync.z,
                    yaw: sync.yaw,
                    pitch: sync.pitch,
                })?;
                Ok(Some(raw))
            }
            PlayDisconnect::ID => {
                let dc: PlayDisconnect = raw.decode_as()?;
                Err(ClientError::Disconnected(format!("{:?}", dc.reason)))
            }
            _ => Ok(Some(raw)),
        }
    }
}

/// Stateless entrypoints for headless Minecraft client connections.
pub struct HeadlessClient;

impl HeadlessClient {
    /// Executes `Handshake(Status)` -> `StatusRequest` -> `PingRequest` against `addr`.
    pub fn fetch_status(
        addr: SocketAddr,
        host: &str,
        port: u16,
        cfg: &ClientConfig,
    ) -> Result<StatusPingResult, ClientError> {
        let mut stream = connect_with_timeout(addr, cfg)?;
        let mut read_buf = BytesMut::with_capacity(4096);

        send_framed(
            &mut stream,
            &Handshake {
                protocol_version: VarInt(cfg.protocol_version),
                server_address: host.to_owned(),
                server_port: port,
                next_state: ConnectionState::Status,
            },
            None,
        )?;
        send_framed(&mut stream, &StatusRequest, None)?;

        let raw = read_framed(&mut stream, &mut read_buf, None, cfg.max_packet_size)?;
        if raw.id != StatusResponse::ID {
            return Err(ClientError::Protocol(Error::UnexpectedPacketId {
                id: raw.id,
                state: "status",
            }));
        }
        let status: StatusResponse = raw.decode_as()?;

        let timestamp = 0x1234_5678_9abc_def0_i64;
        send_framed(&mut stream, &PingRequest { timestamp }, None)?;
        let pong_raw = read_framed(&mut stream, &mut read_buf, None, cfg.max_packet_size)?;
        let pong: PongResponse = pong_raw.decode_as()?;

        Ok(StatusPingResult {
            json: status.json,
            ping_echoed: pong.timestamp == timestamp,
        })
    }

    /// Connects through `Handshake(Login)` -> `Login` -> `Configuration` -> `Play` (offline mode).
    pub fn connect_play(
        addr: SocketAddr,
        host: &str,
        port: u16,
        username: &str,
        cfg: &ClientConfig,
    ) -> Result<HeadlessPlaySession, ClientError> {
        let mut stream = connect_with_timeout(addr, cfg)?;
        let mut compression_threshold: Option<usize> = None;
        let mut read_buf = BytesMut::with_capacity(8192);

        send_framed(
            &mut stream,
            &Handshake {
                protocol_version: VarInt(cfg.protocol_version),
                server_address: host.to_owned(),
                server_port: port,
                next_state: ConnectionState::Login,
            },
            None,
        )?;

        let login_uuid = offline_player_uuid(username);
        send_framed(
            &mut stream,
            &LoginStart {
                username: username.to_owned(),
                player_uuid: login_uuid,
            },
            None,
        )?;

        let success: LoginSuccess = loop {
            let raw = read_framed(
                &mut stream,
                &mut read_buf,
                compression_threshold,
                cfg.max_packet_size,
            )?;
            match raw.id {
                LoginDisconnect::ID => {
                    let dc: LoginDisconnect = raw.decode_as()?;
                    return Err(ClientError::Disconnected(dc.reason_json));
                }
                0x01 => return Err(ClientError::EncryptionRequired),
                SetCompression::ID => {
                    let sc: SetCompression = raw.decode_as()?;
                    if sc.threshold.0 >= 0 {
                        compression_threshold = Some(sc.threshold.0 as usize);
                    }
                }
                LoginSuccess::ID => break raw.decode_as()?,
                other => {
                    return Err(ClientError::Protocol(Error::UnexpectedPacketId {
                        id: other,
                        state: "login",
                    }));
                }
            }
        };

        send_framed(&mut stream, &LoginAcknowledged, compression_threshold)?;

        // Configuration phase
        loop {
            let raw = read_framed(
                &mut stream,
                &mut read_buf,
                compression_threshold,
                cfg.max_packet_size,
            )?;
            match raw.id {
                ConfigurationDisconnect::ID => {
                    let dc: ConfigurationDisconnect = raw.decode_as()?;
                    return Err(ClientError::Disconnected(format!("{:?}", dc.reason)));
                }
                ClientboundKnownPacks::ID => {
                    let kp: ClientboundKnownPacks = raw.decode_as()?;
                    send_framed(
                        &mut stream,
                        &ServerboundKnownPacks { packs: kp.packs },
                        compression_threshold,
                    )?;
                }
                FinishConfiguration::ID => {
                    send_framed(
                        &mut stream,
                        &AcknowledgeFinishConfiguration,
                        compression_threshold,
                    )?;
                    break;
                }
                _ => {}
            }
        }

        // First Play packet must be PlayLogin (0x30) or PlayDisconnect
        let login_play: PlayLogin = loop {
            let raw = read_framed(
                &mut stream,
                &mut read_buf,
                compression_threshold,
                cfg.max_packet_size,
            )?;
            match raw.id {
                PlayDisconnect::ID => {
                    let dc: PlayDisconnect = raw.decode_as()?;
                    return Err(ClientError::Disconnected(format!("{:?}", dc.reason)));
                }
                PlayLogin::ID => break raw.decode_as()?,
                _ => {}
            }
        };

        Ok(HeadlessPlaySession {
            stream,
            read_buf,
            compression_threshold,
            cfg: cfg.clone(),
            login_play,
            uuid: success.uuid,
            username: success.username,
        })
    }
}

fn connect_with_timeout(addr: SocketAddr, cfg: &ClientConfig) -> Result<TcpStream, ClientError> {
    let stream = TcpStream::connect_timeout(&addr, cfg.connect_timeout)?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(cfg.io_timeout));
    let _ = stream.set_write_timeout(Some(cfg.io_timeout));
    Ok(stream)
}

fn send_framed<P: Packet>(
    stream: &mut TcpStream,
    packet: &P,
    compression_threshold: Option<usize>,
) -> Result<(), ClientError> {
    let mut out = BytesMut::new();
    encode_packet_with_compression(packet, &mut out, compression_threshold)?;
    stream.write_all(&out)?;
    Ok(())
}

fn read_framed(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
    compression_threshold: Option<usize>,
    max_packet_size: usize,
) -> Result<RawPacket, ClientError> {
    let compression = compression_threshold.map(CompressionConfig::new);
    loop {
        if let Some(pkt) = decode_frame_with_compression(buf, max_packet_size, compression)? {
            return Ok(pkt);
        }
        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err(ClientError::UnexpectedEof);
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_wire_fixtures_match_mc_26_3_protocol_777() {
        // Independent MC 26.3 (protocol 777) wire bytes for Handshake(Status) to "localhost":25565:
        // VarInt(777) = 0x89, 0x06
        // String("localhost") = len 9 (0x09) + b"localhost"
        // Port 25565 = 0x63, 0xDD
        // Next state Status = 0x01
        let expected_handshake_payload: Vec<u8> = vec![
            0x00, // packet ID 0x00
            0x89, 0x06, // protocol 777
            0x09, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't', // host
            0x63, 0xDD, // port 25565
            0x01, // Status
        ];
        let mut encoded = BytesMut::new();
        encode_packet_with_compression(
            &Handshake {
                protocol_version: VarInt(777),
                server_address: "localhost".to_owned(),
                server_port: 25565,
                next_state: ConnectionState::Status,
            },
            &mut encoded,
            None,
        )
        .unwrap();
        assert_eq!(encoded[0] as usize, expected_handshake_payload.len());
        assert_eq!(&encoded[1..], expected_handshake_payload.as_slice());

        // StatusRequest is always exact 2-byte frame [0x01, 0x00]
        let mut status_req = BytesMut::new();
        encode_packet_with_compression(&StatusRequest, &mut status_req, None).unwrap();
        assert_eq!(&status_req[..], &[0x01, 0x00]);

        // SetCompression(threshold=256): packet_id=0x03, VarInt(256)=[0x80, 0x02] -> frame [0x03, 0x03, 0x80, 0x02]
        let mut comp_buf = BytesMut::from(&[0x03u8, 0x03, 0x80, 0x02][..]);
        let comp_raw = decode_frame_with_compression(&mut comp_buf, 1024, None)
            .unwrap()
            .unwrap();
        let sc: SetCompression = comp_raw.decode_as().unwrap();
        assert_eq!(sc.threshold.0, 256);

        // Oversized frame header (> max_packet_size) must return InvalidPacketLength without allocating
        let mut oversized = BytesMut::from(&[0xFFu8, 0xFF, 0x7F, 0x00][..]);
        assert!(matches!(
            decode_frame_with_compression(&mut oversized, 1024, None),
            Err(Error::InvalidPacketLength { .. })
        ));
    }
}
