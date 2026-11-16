//! Core Play state packets (shared foundation for `RadLimbo`, `RadServer`, and `RadProxy`).

mod entity;
mod gameplay;
mod world;

pub use entity::*;
pub use gameplay::*;
pub use world::*;

use bytes::{Buf, BufMut};
use uuid::Uuid;

use crate::error::Result;
use crate::nbt::{NbtTag, decode_network_nbt, encode_network_nbt};
use crate::packet::{Packet, PacketDirection};
use crate::state::ConnectionState;
use crate::types::{BlockPos, Decode, Encode, VarInt, read_string, write_string};

// CommonPlayerSpawnInfo in 26.3 uses enum VarInts and OPTIONAL_VAR_INT
// (zero = absent; otherwise mode + 1), rather than the old signed byte.
fn decode_game_mode(buf: &mut impl Buf, optional: bool) -> Result<i8> {
    let value = VarInt::decode(buf)?.0;
    let max = if optional { 4 } else { 3 };
    if !(0..=max).contains(&value) {
        return Err(crate::error::Error::InvalidPacket(format!(
            "invalid game mode {value}"
        )));
    }
    Ok((value - i32::from(optional)) as i8)
}

fn encode_game_mode(buf: &mut impl BufMut, value: i32, optional: bool) -> Result<()> {
    let min = if optional { -1 } else { 0 };
    if !(min..=3).contains(&value) {
        return Err(crate::error::Error::InvalidPacket(format!(
            "invalid game mode {value}"
        )));
    }
    VarInt(value + i32::from(optional)).encode(buf)
}

/// Clientbound `Keep Alive` packet (`0x26` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAliveClientbound {
    /// Server-generated keep-alive token.
    pub id: i64,
}

impl Packet for KeepAliveClientbound {
    const ID: i32 = 0x2D;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.id.encode(buf)
    }
}

/// Serverbound `Keep Alive` packet (`0x1A` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAliveServerbound {
    /// Echoed keep-alive token.
    pub id: i64,
}

impl Packet for KeepAliveServerbound {
    const ID: i32 = 0x1C;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            id: i64::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.id.encode(buf)
    }
}

/// Clientbound `Play Disconnect` packet (`0x1D` in Play).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayDisconnect {
    /// Disconnect reason as Network NBT text component.
    pub reason: NbtTag,
}

impl Packet for PlayDisconnect {
    const ID: i32 = 0x20;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            reason: decode_network_nbt(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        encode_network_nbt(&self.reason, buf)
    }
}

/// Clientbound `System Chat Message` packet (`0x72` in Play).
#[derive(Debug, Clone, PartialEq)]
pub struct SystemChatMessage {
    /// Chat content as Network NBT text component.
    pub content: NbtTag,
    /// `true` to display on the action bar, `false` for chat box.
    pub overlay: bool,
}

impl Packet for SystemChatMessage {
    const ID: i32 = 0x7C;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let content = decode_network_nbt(buf)?;
        let overlay = bool::decode(buf)?;
        Ok(Self { content, overlay })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        encode_network_nbt(&self.content, buf)?;
        self.overlay.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Synchronize Player Position` packet (`0x41` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynchronizePlayerPosition {
    /// Teleport confirmation ID.
    pub teleport_id: VarInt,
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
    /// X velocity.
    pub vx: f64,
    /// Y velocity.
    pub vy: f64,
    /// Z velocity.
    pub vz: f64,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Relative position/velocity bitmask flags.
    pub flags: i32,
}

impl Packet for SynchronizePlayerPosition {
    const ID: i32 = 0x49;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            teleport_id: VarInt::decode(buf)?,
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            vx: f64::decode(buf)?,
            vy: f64::decode(buf)?,
            vz: f64::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            flags: i32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.teleport_id.encode(buf)?;
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.vx.encode(buf)?;
        self.vy.encode(buf)?;
        self.vz.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Confirm Teleportation` packet (`0x00` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfirmTeleportation {
    /// Echoed teleport ID from [`SynchronizePlayerPosition`].
    pub teleport_id: VarInt,
    /// Client X coordinate.
    pub x: f64,
    /// Client Y coordinate.
    pub y: f64,
    /// Client Z coordinate.
    pub z: f64,
    /// Client yaw angle.
    pub yaw: f32,
    /// Client pitch angle.
    pub pitch: f32,
}

impl Packet for ConfirmTeleportation {
    const ID: i32 = 0x00;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            teleport_id: VarInt::decode(buf)?,
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.teleport_id.encode(buf)?;
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Start Configuration` packet (`0x6F` in Play).
///
/// Requests the client to transition back from `Play` to `Configuration` (used for
/// seamless server switching on `RadProxy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StartConfiguration;

impl Packet for StartConfiguration {
    const ID: i32 = 0x78;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// Serverbound `Configuration Acknowledged` packet (`0x10` in Play, protocol `777`).
///
/// Sent by the client after receiving [`StartConfiguration`] to enter `Configuration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfigurationAcknowledged;

impl Packet for ConfigurationAcknowledged {
    const ID: i32 = 0x10;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(_buf: &mut impl Buf) -> Result<Self> {
        Ok(Self)
    }

    fn encode_body(&self, _buf: &mut impl BufMut) -> Result<()> {
        Ok(())
    }
}

/// Serverbound `Chat Command` packet (`0x05` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCommand {
    /// Command string without the leading `/` (e.g. `"server lobby"`).
    pub command: String,
}

impl Packet for ChatCommand {
    const ID: i32 = 0x07;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            command: read_string(buf, 32_767)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.command, 32_767)
    }
}

/// Clientbound `Game Event` packet (`0x22` in Play).
///
/// Event `13` (`StartWaitingForLevelChunks`) tells the client to exit the
/// "Loading terrain..." screen once initial chunks are sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameEvent {
    /// Event ID (e.g., `13` = start waiting for level chunks).
    pub event: u8,
    /// Event-specific float parameter (usually `0.0`).
    pub value: f32,
}

impl GameEvent {
    /// Event ID changing the player's game mode (`value` = mode `0..=3`).
    pub const CHANGE_GAME_MODE: u8 = 3;
    /// Event ID telling the client to start waiting for level chunks.
    pub const START_WAITING_FOR_CHUNKS: u8 = 13;
}

impl Packet for GameEvent {
    const ID: i32 = 0x27;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            event: u8::decode(buf)?,
            value: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.event.encode(buf)?;
        self.value.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Set Default Spawn Position` packet (`0x63` in Play, protocol `777`).
#[derive(Debug, Clone, PartialEq)]
pub struct SetDefaultSpawnPosition {
    /// Target dimension identifier (e.g., `"minecraft:overworld"`).
    pub dimension_name: String,
    /// Spawn block coordinates.
    pub location: BlockPos,
    /// Spawn yaw angle.
    pub yaw: f32,
    /// Spawn pitch angle.
    pub pitch: f32,
}

impl Packet for SetDefaultSpawnPosition {
    const ID: i32 = 0x63;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            dimension_name: read_string(buf, 32_767)?,
            location: BlockPos::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.dimension_name, 32_767)?;
        self.location.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Plugin Message` (`CustomPayload`) packet (`0x14` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayServerboundPluginMessage {
    /// Namespaced plugin channel (e.g., `"bungeecord:main"`).
    pub channel: String,
    /// Raw payload bytes.
    pub data: Vec<u8>,
}

impl Packet for PlayServerboundPluginMessage {
    const ID: i32 = 0x16;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let channel = read_string(buf, 32767)?;
        let remaining = buf.remaining();
        let mut data = vec![0u8; remaining];
        buf.copy_to_slice(&mut data);
        Ok(Self { channel, data })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.channel, 32767)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

/// Clientbound `Plugin Message` (`CustomPayload`) packet (`0x19` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayClientboundPluginMessage {
    /// Namespaced plugin channel (e.g., `"bungeecord:main"`, `"minecraft:brand"`).
    pub channel: String,
    /// Raw payload bytes.
    pub data: Vec<u8>,
}

impl Packet for PlayClientboundPluginMessage {
    const ID: i32 = 0x18;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let channel = read_string(buf, 32767)?;
        let remaining = buf.remaining();
        let mut data = vec![0u8; remaining];
        buf.copy_to_slice(&mut data);
        Ok(Self { channel, data })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.channel, 32767)?;
        buf.put_slice(&self.data);
        Ok(())
    }
}

/// Brigadier command node kind in [`DeclareCommands`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandNodeKind {
    /// Root node (`0x00`).
    Root,
    /// Literal subcommand node (`0x01`).
    Literal {
        /// Literal command/subcommand name.
        name: String,
    },
    /// Greedy string argument node (`0x02` with `brigadier:string` parser id `5`, `GREEDY_PHRASE`).
    GreedyStringArg {
        /// Argument name.
        name: String,
    },
    /// Single word string argument node (`0x02` with `brigadier:string` parser id `5`, `SINGLE_WORD`).
    WordStringArg {
        /// Argument name.
        name: String,
    },
}

/// A single node inside [`DeclareCommands`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandNode {
    /// Node type and payload.
    pub kind: CommandNodeKind,
    /// Whether this node is executable as a command.
    pub executable: bool,
    /// Indices of child nodes in [`DeclareCommands::nodes`].
    pub children: Vec<VarInt>,
    /// Optional Brigadier suggestions type identifier (e.g. `"minecraft:ask_server"`).
    pub suggestions_type: Option<String>,
}

impl CommandNode {
    /// Creates a root node (`0x00`).
    #[must_use]
    pub fn root(children: Vec<VarInt>) -> Self {
        Self {
            kind: CommandNodeKind::Root,
            executable: false,
            children,
            suggestions_type: None,
        }
    }

    /// Creates a literal command/subcommand node (`0x01`).
    #[must_use]
    pub fn literal(name: impl Into<String>, executable: bool, children: Vec<VarInt>) -> Self {
        Self {
            kind: CommandNodeKind::Literal { name: name.into() },
            executable,
            children,
            suggestions_type: None,
        }
    }

    /// Creates a single-word string argument node (`0x02`) with optional suggestions type.
    #[must_use]
    pub fn word_arg(
        name: impl Into<String>,
        executable: bool,
        children: Vec<VarInt>,
        suggestions_type: Option<String>,
    ) -> Self {
        Self {
            kind: CommandNodeKind::WordStringArg { name: name.into() },
            executable,
            children,
            suggestions_type,
        }
    }

    /// Creates a greedy string argument node (`0x02`) with optional suggestions type.
    #[must_use]
    pub fn greedy_arg(
        name: impl Into<String>,
        executable: bool,
        children: Vec<VarInt>,
        suggestions_type: Option<String>,
    ) -> Self {
        Self {
            kind: CommandNodeKind::GreedyStringArg { name: name.into() },
            executable,
            children,
            suggestions_type,
        }
    }
}

/// Clientbound `Commands` (`DeclareCommands`) packet (`0x11` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclareCommands {
    /// Flat list of Brigadier command nodes.
    pub nodes: Vec<CommandNode>,
    /// Index of the root node (typically `VarInt(0)`).
    pub root_index: VarInt,
}

impl Packet for DeclareCommands {
    const ID: i32 = 0x10;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let count = VarInt::decode(buf)?.0.max(0) as usize;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            let flags = u8::decode(buf)?;
            let executable = (flags & 0x04) != 0;
            let child_count = VarInt::decode(buf)?.0.max(0) as usize;
            let mut children = Vec::with_capacity(child_count);
            for _ in 0..child_count {
                children.push(VarInt::decode(buf)?);
            }
            if (flags & 0x08) != 0 {
                let _redirect = VarInt::decode(buf)?;
            }
            let node_type = flags & 0x03;
            let kind = match node_type {
                0 => CommandNodeKind::Root,
                1 => CommandNodeKind::Literal {
                    name: read_string(buf, 32767)?,
                },
                2 => {
                    let name = read_string(buf, 32767)?;
                    let _parser_id = VarInt::decode(buf)?;
                    let mode = VarInt::decode(buf)?.0;
                    if mode == 2 {
                        CommandNodeKind::GreedyStringArg { name }
                    } else {
                        CommandNodeKind::WordStringArg { name }
                    }
                }
                _ => CommandNodeKind::Root,
            };
            let suggestions_type = if (flags & 0x10) != 0 {
                Some(read_string(buf, 32767)?)
            } else {
                None
            };
            nodes.push(CommandNode {
                kind,
                executable,
                children,
                suggestions_type,
            });
        }
        let root_index = VarInt::decode(buf)?;
        Ok(Self { nodes, root_index })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        VarInt(self.nodes.len() as i32).encode(buf)?;
        for node in &self.nodes {
            let type_bits: u8 = match node.kind {
                CommandNodeKind::Root => 0x00,
                CommandNodeKind::Literal { .. } => 0x01,
                CommandNodeKind::GreedyStringArg { .. } | CommandNodeKind::WordStringArg { .. } => {
                    0x02
                }
            };
            let exec_bit: u8 = if node.executable { 0x04 } else { 0x00 };
            let suggest_bit: u8 = if node.suggestions_type.is_some()
                && !matches!(
                    node.kind,
                    CommandNodeKind::Root | CommandNodeKind::Literal { .. }
                ) {
                0x10
            } else {
                0x00
            };
            (type_bits | exec_bit | suggest_bit).encode(buf)?;
            VarInt(node.children.len() as i32).encode(buf)?;
            for child in &node.children {
                child.encode(buf)?;
            }
            match &node.kind {
                CommandNodeKind::Root => {}
                CommandNodeKind::Literal { name } => write_string(buf, name, 32767)?,
                CommandNodeKind::WordStringArg { name } => {
                    write_string(buf, name, 32767)?;
                    VarInt(5).encode(buf)?; // brigadier:string
                    VarInt(0).encode(buf)?; // SINGLE_WORD
                    if let Some(ref st) = node.suggestions_type {
                        write_string(buf, st, 32767)?;
                    }
                }
                CommandNodeKind::GreedyStringArg { name } => {
                    write_string(buf, name, 32767)?;
                    VarInt(5).encode(buf)?; // brigadier:string
                    VarInt(2).encode(buf)?; // GREEDY_PHRASE
                    if let Some(ref st) = node.suggestions_type {
                        write_string(buf, st, 32767)?;
                    }
                }
            }
        }
        self.root_index.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Command Suggestions Request` packet (`0x0F` in Play, protocol 777).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSuggestionsRequest {
    /// Transaction ID echoed back in [`CommandSuggestions`].
    pub id: VarInt,
    /// Current command input text (typically starting with `/`).
    pub text: String,
}

impl Packet for CommandSuggestionsRequest {
    const ID: i32 = 0x0F;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            id: VarInt::decode(buf)?,
            text: read_string(buf, 32500)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.id.encode(buf)?;
        write_string(buf, &self.text, 32500)
    }
}

/// A single tab-completion match in [`CommandSuggestions`].
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSuggestionMatch {
    /// Suggested completion text.
    pub text: String,
    /// Optional Network NBT text tooltip shown alongside the suggestion.
    pub tooltip: Option<NbtTag>,
}

/// Clientbound `Command Suggestions` packet (`0x0F` in Play, protocol 777).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSuggestions {
    /// Transaction ID matching [`CommandSuggestionsRequest::id`].
    pub id: VarInt,
    /// Start character index in the input text where replacement begins.
    pub start: VarInt,
    /// Length of the substring to replace.
    pub length: VarInt,
    /// Suggested matches.
    pub matches: Vec<CommandSuggestionMatch>,
}

impl Packet for CommandSuggestions {
    const ID: i32 = 0x0F;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let id = VarInt::decode(buf)?;
        let start = VarInt::decode(buf)?;
        let length = VarInt::decode(buf)?;
        let count = VarInt::decode(buf)?.0.max(0) as usize;
        let mut matches = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let text = read_string(buf, 32767)?;
            let has_tooltip = bool::decode(buf)?;
            let tooltip = if has_tooltip {
                Some(decode_network_nbt(buf)?)
            } else {
                None
            };
            matches.push(CommandSuggestionMatch { text, tooltip });
        }
        Ok(Self {
            id,
            start,
            length,
            matches,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.id.encode(buf)?;
        self.start.encode(buf)?;
        self.length.encode(buf)?;
        VarInt(self.matches.len() as i32).encode(buf)?;
        for m in &self.matches {
            write_string(buf, &m.text, 32767)?;
            m.tooltip.is_some().encode(buf)?;
            if let Some(ref tip) = m.tooltip {
                encode_network_nbt(tip, buf)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_frame, encode_packet};
    use bytes::BytesMut;

    #[test]
    fn play_plugin_message_and_declare_commands_roundtrip() {
        let pm = PlayServerboundPluginMessage {
            channel: "bungeecord:main".into(),
            data: vec![0, 7, b'C', b'o', b'n', b'n', b'e', b'c', b't'],
        };
        let mut buf = BytesMut::new();
        encode_packet(&pm, &mut buf).unwrap();
        let decoded_pm: PlayServerboundPluginMessage = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_pm, pm);

        let cmds = DeclareCommands {
            nodes: vec![
                CommandNode::root(vec![VarInt(1)]),
                CommandNode::literal("server", true, vec![VarInt(2)]),
                CommandNode::word_arg("name", true, vec![], Some("minecraft:ask_server".into())),
            ],
            root_index: VarInt(0),
        };
        buf.clear();
        encode_packet(&cmds, &mut buf).unwrap();
        let decoded_cmds: DeclareCommands = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_cmds, cmds);

        let req = CommandSuggestionsRequest {
            id: VarInt(7),
            text: "/gamemode ".into(),
        };
        buf.clear();
        encode_packet(&req, &mut buf).unwrap();
        let decoded_req: CommandSuggestionsRequest = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_req, req);

        let resp = CommandSuggestions {
            id: VarInt(7),
            start: VarInt(10),
            length: VarInt(0),
            matches: vec![
                CommandSuggestionMatch {
                    text: "survival".into(),
                    tooltip: None,
                },
                CommandSuggestionMatch {
                    text: "creative".into(),
                    tooltip: Some(NbtTag::String("Creative mode".into())),
                },
            ],
        };
        buf.clear();
        encode_packet(&resp, &mut buf).unwrap();
        let decoded_resp: CommandSuggestions = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_resp, resp);
    }

    #[test]
    fn play_login_and_chunk_center_roundtrip() {
        let login = PlayLogin {
            entity_id: 1,
            is_hardcore: false,
            dimension_names: vec!["minecraft:overworld".into()],
            max_players: VarInt(100),
            view_distance: VarInt(2),
            simulation_distance: VarInt(2),
            reduced_debug_info: false,
            enable_respawn_screen: false,
            do_limited_crafting: false,
            dimension_type: VarInt(0),
            dimension_name: "minecraft:overworld".into(),
            hashed_seed: 0,
            game_mode: 3,
            previous_game_mode: -1,
            is_debug: false,
            is_flat: true,
            portal_cooldown: VarInt(0),
            sea_level: VarInt(64),
            online_mode: false,
            enforces_secure_chat: false,
        };
        let mut buf = BytesMut::new();
        encode_packet(&login, &mut buf).unwrap();
        // Vanilla CommonPlayerSpawnInfo + Login suffix, independently specified:
        // spectator, no previous mode, debug, flat, death, cooldown, sea, online, chat.
        assert_eq!(&buf[buf.len() - 9..], &[3, 0, 0, 1, 0, 0, 64, 0, 0]);
        let decoded_login: PlayLogin = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_login, login);

        let center = SetChunkCacheCenter {
            chunk_x: VarInt(0),
            chunk_z: VarInt(0),
        };
        buf.clear();
        encode_packet(&center, &mut buf).unwrap();
        let decoded_center: SetChunkCacheCenter = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(decoded_center, center);
    }
}

/// Clientbound `Play Login` (`JoinGame`) packet (`0x2B` in Play).
/// Clientbound `Play Login` (`JoinGame`) packet (`0x2B` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayLogin {
    /// Player entity ID.
    pub entity_id: i32,
    /// Hardcore mode flag.
    pub is_hardcore: bool,
    /// Available dimension identifiers.
    pub dimension_names: Vec<String>,
    /// Reported maximum players.
    pub max_players: VarInt,
    /// Client view distance in chunks.
    pub view_distance: VarInt,
    /// Simulation distance in chunks.
    pub simulation_distance: VarInt,
    /// Reduced F3 debug info flag.
    pub reduced_debug_info: bool,
    /// Enable respawn screen flag.
    pub enable_respawn_screen: bool,
    /// Limited crafting flag.
    pub do_limited_crafting: bool,
    /// Index into `minecraft:dimension_type` registry.
    pub dimension_type: VarInt,
    /// Current dimension identifier (e.g., `"minecraft:overworld"`).
    pub dimension_name: String,
    /// First 8 bytes of SHA-256 world seed.
    pub hashed_seed: i64,
    /// Current game mode (`0`=Survival, `1`=Creative, `2`=Adventure, `3`=Spectator).
    pub game_mode: u8,
    /// Previous game mode (`-1` if none).
    pub previous_game_mode: i8,
    /// Debug world flag.
    pub is_debug: bool,
    /// Superflat world flag.
    pub is_flat: bool,
    /// Portal cooldown ticks.
    pub portal_cooldown: VarInt,
    /// Sea level Y coordinate.
    pub sea_level: VarInt,
    /// Online mode / Mojang authentication flag (introduced in 26.3 / 777).
    pub online_mode: bool,
    /// Enforces secure chat flag.
    pub enforces_secure_chat: bool,
}

impl Packet for PlayLogin {
    const ID: i32 = 0x32;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let entity_id = i32::decode(buf)?;
        let is_hardcore = bool::decode(buf)?;
        let dim_count = usize::try_from(VarInt::decode(buf)?.0)
            .unwrap_or(0)
            .min(1024);
        let mut dimension_names = Vec::with_capacity(dim_count);
        for _ in 0..dim_count {
            dimension_names.push(read_string(buf, 32_767)?);
        }
        let max_players = VarInt::decode(buf)?;
        let view_distance = VarInt::decode(buf)?;
        let simulation_distance = VarInt::decode(buf)?;
        let reduced_debug_info = bool::decode(buf)?;
        let enable_respawn_screen = bool::decode(buf)?;
        let do_limited_crafting = bool::decode(buf)?;
        let dimension_type = VarInt::decode(buf)?;
        let dimension_name = read_string(buf, 32_767)?;
        let hashed_seed = i64::decode(buf)?;
        let game_mode = decode_game_mode(buf, false)? as u8;
        let previous_game_mode = decode_game_mode(buf, true)?;
        let is_debug = bool::decode(buf)?;
        let is_flat = bool::decode(buf)?;
        let has_death_location = bool::decode(buf)?;
        if has_death_location {
            let _ = read_string(buf, 32_767)?;
            let _ = BlockPos::decode(buf)?;
        }
        let portal_cooldown = VarInt::decode(buf)?;
        let sea_level = VarInt::decode(buf)?;
        let online_mode = bool::decode(buf)?;
        let enforces_secure_chat = bool::decode(buf)?;
        Ok(Self {
            entity_id,
            is_hardcore,
            dimension_names,
            max_players,
            view_distance,
            simulation_distance,
            reduced_debug_info,
            enable_respawn_screen,
            do_limited_crafting,
            dimension_type,
            dimension_name,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            portal_cooldown,
            sea_level,
            online_mode,
            enforces_secure_chat,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.is_hardcore.encode(buf)?;
        VarInt(self.dimension_names.len() as i32).encode(buf)?;
        for dim in &self.dimension_names {
            write_string(buf, dim, 32_767)?;
        }
        self.max_players.encode(buf)?;
        self.view_distance.encode(buf)?;
        self.simulation_distance.encode(buf)?;
        self.reduced_debug_info.encode(buf)?;
        self.enable_respawn_screen.encode(buf)?;
        self.do_limited_crafting.encode(buf)?;
        self.dimension_type.encode(buf)?;
        write_string(buf, &self.dimension_name, 32_767)?;
        self.hashed_seed.encode(buf)?;
        encode_game_mode(buf, i32::from(self.game_mode), false)?;
        // 26.3 GameType.OPTIONAL_STREAM_CODEC: 0=None, otherwise id+1.
        encode_game_mode(buf, i32::from(self.previous_game_mode), true)?;
        self.is_debug.encode(buf)?;
        self.is_flat.encode(buf)?;
        false.encode(buf)?; // has_death_location = false
        self.portal_cooldown.encode(buf)?;
        self.sea_level.encode(buf)?;
        self.online_mode.encode(buf)?;
        self.enforces_secure_chat.encode(buf)?;
        Ok(())
    }
}
