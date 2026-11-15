//! Player info, entity spawning, movement, and chat Play packets.

use super::*;
use crate::packet::login::GameProfileProperty;
use crate::types::LpVec3;
/// Single player entry for [`PlayerInfoUpdate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoAddEntry {
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub name: String,
    /// Profile properties (such as `textures` skin data).
    pub properties: Vec<GameProfileProperty>,
    /// Initial game mode (`0..=3`).
    pub game_mode: VarInt,
    /// Whether the player is shown in the tab list.
    pub listed: bool,
    /// Ping latency in ms.
    pub latency: VarInt,
}

/// Clientbound `Player Info Update` packet (`0x3E` in Play) supporting `AddPlayer | UpdateGameMode | UpdateListed | UpdateLatency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoUpdate {
    /// Players to add/update in the tab list.
    pub entries: Vec<PlayerInfoAddEntry>,
}

impl PlayerInfoUpdate {
    /// Action bitmask: `ADD_PLAYER (0x01) | UPDATE_GAME_MODE (0x04) | UPDATE_LISTED (0x08) | UPDATE_LATENCY (0x10)`.
    const ACTION_MASK: u8 = 0x01 | 0x04 | 0x08 | 0x10;
}

impl Packet for PlayerInfoUpdate {
    const ID: i32 = 0x47;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let mask = u8::decode(buf)?;
        let count = usize::try_from(VarInt::decode(buf)?.0)
            .unwrap_or(0)
            .min(1024);
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let uuid = Uuid::decode(buf)?;
            let (name, properties) = if mask & 0x01 != 0 {
                let n = read_string(buf, 16)?;
                let prop_count = usize::try_from(VarInt::decode(buf)?.0).unwrap_or(0).min(16);
                let mut props = Vec::with_capacity(prop_count);
                for _ in 0..prop_count {
                    let pname = read_string(buf, 64)?;
                    let value = read_string(buf, 32_767)?;
                    let signature = if bool::decode(buf)? {
                        Some(read_string(buf, 32_767)?)
                    } else {
                        None
                    };
                    props.push(GameProfileProperty {
                        name: pname,
                        value,
                        signature,
                    });
                }
                (n, props)
            } else {
                (String::new(), Vec::new())
            };
            let game_mode = if mask & 0x04 != 0 {
                VarInt::decode(buf)?
            } else {
                VarInt(0)
            };
            let listed = if mask & 0x08 != 0 {
                bool::decode(buf)?
            } else {
                true
            };
            let latency = if mask & 0x10 != 0 {
                VarInt::decode(buf)?
            } else {
                VarInt(0)
            };
            entries.push(PlayerInfoAddEntry {
                uuid,
                name,
                properties,
                game_mode,
                listed,
                latency,
            });
        }
        Ok(Self { entries })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        Self::ACTION_MASK.encode(buf)?;
        VarInt(self.entries.len() as i32).encode(buf)?;
        for entry in &self.entries {
            entry.uuid.encode(buf)?;
            write_string(buf, &entry.name, 16)?;
            VarInt(i32::try_from(entry.properties.len()).unwrap_or(0)).encode(buf)?;
            for prop in &entry.properties {
                write_string(buf, &prop.name, 64)?;
                write_string(buf, &prop.value, 32_767)?;
                match &prop.signature {
                    Some(sig) => {
                        true.encode(buf)?;
                        write_string(buf, sig, 32_767)?;
                    }
                    None => false.encode(buf)?,
                }
            }
            entry.game_mode.encode(buf)?;
            entry.listed.encode(buf)?;
            entry.latency.encode(buf)?;
        }
        Ok(())
    }
}

/// Clientbound `Player Info Remove` packet (`0x3D` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoRemove {
    /// Player UUIDs to remove from the tab list.
    pub uuids: Vec<Uuid>,
}

impl Packet for PlayerInfoRemove {
    const ID: i32 = 0x46;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let count = usize::try_from(VarInt::decode(buf)?.0)
            .unwrap_or(0)
            .min(1024);
        let mut uuids = Vec::with_capacity(count);
        for _ in 0..count {
            uuids.push(Uuid::decode(buf)?);
        }
        Ok(Self { uuids })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        VarInt(self.uuids.len() as i32).encode(buf)?;
        for uuid in &self.uuids {
            uuid.encode(buf)?;
        }
        Ok(())
    }
}

/// Clientbound `Spawn Entity` packet (`0x01` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnEntity {
    /// Entity ID.
    pub entity_id: VarInt,
    /// Entity UUID.
    pub uuid: Uuid,
    /// Registry ID of entity type (e.g., player).
    pub entity_type: VarInt,
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
    /// Quantized initial velocity (`LpVec3`, encoded before angles in 777).
    pub velocity: LpVec3,
    /// Pitch in 1/256ths of a full turn.
    pub pitch: i8,
    /// Yaw in 1/256ths of a full turn.
    pub yaw: i8,
    /// Head yaw in 1/256ths of a full turn.
    pub head_yaw: i8,
    /// Entity-specific data.
    pub data: VarInt,
}

impl Packet for SpawnEntity {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            uuid: Uuid::decode(buf)?,
            entity_type: VarInt::decode(buf)?,
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            velocity: LpVec3::decode(buf)?,
            pitch: u8::decode(buf)? as i8,
            yaw: u8::decode(buf)? as i8,
            head_yaw: u8::decode(buf)? as i8,
            data: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.uuid.encode(buf)?;
        self.entity_type.encode(buf)?;
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.velocity.encode(buf)?;
        (self.pitch as u8).encode(buf)?;
        (self.yaw as u8).encode(buf)?;
        (self.head_yaw as u8).encode(buf)?;
        self.data.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Remove Entities` packet (`0x46` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveEntities {
    /// Entity IDs to despawn.
    pub entity_ids: Vec<VarInt>,
}

impl Packet for RemoveEntities {
    const ID: i32 = 0x4E;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let count = usize::try_from(VarInt::decode(buf)?.0)
            .unwrap_or(0)
            .min(4096);
        let mut entity_ids = Vec::with_capacity(count);
        for _ in 0..count {
            entity_ids.push(VarInt::decode(buf)?);
        }
        Ok(Self { entity_ids })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        VarInt(self.entity_ids.len() as i32).encode(buf)?;
        for id in &self.entity_ids {
            id.encode(buf)?;
        }
        Ok(())
    }
}

/// Entity metadata value variants used by [`SetEntityData`] (protocol `777`).
#[derive(Debug, Clone, PartialEq)]
pub enum MetadataValue {
    /// Serializer `0`: signed byte.
    Byte(i8),
    /// Serializer `1`: VarInt.
    VarInt(VarInt),
    /// Serializer `3`: 32-bit float.
    Float(f32),
    /// Serializer `7`: `ItemStack` (with component patches).
    ItemStack(ItemStack),
    /// Serializer `8`: boolean.
    Boolean(bool),
}

impl MetadataValue {
    /// Returns the protocol `777` `EntityDataSerializers` ID for this value.
    #[must_use]
    pub const fn serializer_id(&self) -> i32 {
        match self {
            Self::Byte(_) => 0,
            Self::VarInt(_) => 1,
            Self::Float(_) => 3,
            Self::ItemStack(_) => 7,
            Self::Boolean(_) => 8,
        }
    }
}

/// Single tracked entity data entry in [`SetEntityData`].
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataEntry {
    /// SynchedEntityData index (e.g. `8` for `ItemEntity.DATA_ITEM`).
    pub index: u8,
    /// Serialized value.
    pub value: MetadataValue,
}

/// Clientbound `Set Entity Data` packet (`0x65` in Play, protocol `777`).
#[derive(Debug, Clone, PartialEq)]
pub struct SetEntityData {
    /// Target entity ID.
    pub entity_id: VarInt,
    /// Tracked data entries (terminated on the wire by `0xFF`).
    pub entries: Vec<MetadataEntry>,
}

impl Packet for SetEntityData {
    const ID: i32 = 0x65;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let entity_id = VarInt::decode(buf)?;
        let mut entries = Vec::new();
        loop {
            let index = u8::decode(buf)?;
            if index == 0xFF {
                break;
            }
            if entries.len() >= 64 {
                return Err(crate::error::Error::InvalidPacket(
                    "too many SetEntityData entries".to_owned(),
                ));
            }
            let type_id = VarInt::decode(buf)?.0;
            let value = match type_id {
                0 => MetadataValue::Byte(u8::decode(buf)? as i8),
                1 => MetadataValue::VarInt(VarInt::decode(buf)?),
                3 => MetadataValue::Float(f32::decode(buf)?),
                7 => MetadataValue::ItemStack(ItemStack::decode_slot(buf)?),
                8 => MetadataValue::Boolean(bool::decode(buf)?),
                other => {
                    return Err(crate::error::Error::InvalidPacket(format!(
                        "unsupported SetEntityData serializer id {other}"
                    )));
                }
            };
            entries.push(MetadataEntry { index, value });
        }
        Ok(Self { entity_id, entries })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        for entry in &self.entries {
            if entry.index == 0xFF {
                return Err(crate::error::Error::InvalidPacket(
                    "SetEntityData entry index cannot be 0xFF".to_owned(),
                ));
            }
            entry.index.encode(buf)?;
            VarInt(entry.value.serializer_id()).encode(buf)?;
            match &entry.value {
                MetadataValue::Byte(v) => (*v as u8).encode(buf)?,
                MetadataValue::VarInt(v) => v.encode(buf)?,
                MetadataValue::Float(v) => v.encode(buf)?,
                MetadataValue::ItemStack(stack) => stack.encode_slot(buf)?,
                MetadataValue::Boolean(v) => v.encode(buf)?,
            }
        }
        0xFFu8.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Take Item Entity` packet (`0x7F` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakeItemEntity {
    /// Entity ID of the dropped item (or arrow/orb) being picked up.
    pub collected_entity_id: VarInt,
    /// Entity ID of the player/entity collecting it.
    pub collector_entity_id: VarInt,
    /// Number of items picked up for the animation.
    pub pickup_item_count: VarInt,
}

impl Packet for TakeItemEntity {
    const ID: i32 = 0x7F;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            collected_entity_id: VarInt::decode(buf)?,
            collector_entity_id: VarInt::decode(buf)?,
            pickup_item_count: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.collected_entity_id.encode(buf)?;
        self.collector_entity_id.encode(buf)?;
        self.pickup_item_count.encode(buf)?;
        Ok(())
    }
}

/// A single stepped movement knot inside [`MoveEntityPosRot`] (MC 26.3 / 777).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityMoveStep {
    /// Tick delay for this step.
    pub ticks: VarInt,
    /// Delta X (`1/4096` block units).
    pub dx: i16,
    /// Delta Y (`1/4096` block units).
    pub dy: i16,
    /// Delta Z (`1/4096` block units).
    pub dz: i16,
}

/// Clientbound `Move Entity Position & Rotation` packet (`0x37` in Play, 777 stepped-delta wire format).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveEntityPosRot {
    /// Entity ID.
    pub entity_id: VarInt,
    /// Optional stepped movement knots (empty = single-step `dx/dy/dz`).
    pub steps: Vec<EntityMoveStep>,
    /// Delta X (`(newX * 4096 - oldX * 4096) as i16`) when `steps` is empty.
    pub dx: i16,
    /// Delta Y when `steps` is empty.
    pub dy: i16,
    /// Delta Z when `steps` is empty.
    pub dz: i16,
    /// Yaw in 1/256ths of a full turn.
    pub yaw: i8,
    /// Pitch in 1/256ths of a full turn.
    pub pitch: i8,
    /// On-ground flag.
    pub on_ground: bool,
}

impl Packet for MoveEntityPosRot {
    const ID: i32 = 0x37;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let entity_id = VarInt::decode(buf)?;
        let properties = VarInt::decode(buf)?.0;
        if properties < 0 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "negative MoveEntityPosRot properties: {properties}"
            )));
        }
        let step_count = (properties >> 1) as usize;
        let on_ground = (properties & 1) != 0;
        if step_count.saturating_mul(7) > buf.remaining() {
            return Err(crate::error::Error::InvalidPacket(format!(
                "MoveEntityPosRot step count {step_count} exceeds remaining {} bytes",
                buf.remaining()
            )));
        }

        let (steps, dx, dy, dz) = if step_count == 0 {
            (
                Vec::new(),
                i16::decode(buf)?,
                i16::decode(buf)?,
                i16::decode(buf)?,
            )
        } else {
            let mut steps = Vec::with_capacity(step_count);
            for _ in 0..step_count {
                steps.push(EntityMoveStep {
                    ticks: VarInt::decode(buf)?,
                    dx: i16::decode(buf)?,
                    dy: i16::decode(buf)?,
                    dz: i16::decode(buf)?,
                });
            }
            let first = steps[0];
            (steps, first.dx, first.dy, first.dz)
        };

        Ok(Self {
            entity_id,
            steps,
            dx,
            dy,
            dz,
            yaw: u8::decode(buf)? as i8,
            pitch: u8::decode(buf)? as i8,
            on_ground,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        let step_count = i32::try_from(self.steps.len()).map_err(|_| {
            crate::error::Error::InvalidPacket("too many MoveEntityPosRot steps".into())
        })?;
        let properties = (step_count << 1) | i32::from(self.on_ground);
        VarInt(properties).encode(buf)?;
        if self.steps.is_empty() {
            self.dx.encode(buf)?;
            self.dy.encode(buf)?;
            self.dz.encode(buf)?;
        } else {
            for step in &self.steps {
                step.ticks.encode(buf)?;
                step.dx.encode(buf)?;
                step.dy.encode(buf)?;
                step.dz.encode(buf)?;
            }
        }
        (self.yaw as u8).encode(buf)?;
        (self.pitch as u8).encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Teleport Entity` packet (`0x80` in Play, 777 layout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeleportEntity {
    /// Entity ID.
    pub entity_id: VarInt,
    /// Absolute X coordinate.
    pub x: f64,
    /// Absolute Y coordinate.
    pub y: f64,
    /// Absolute Z coordinate.
    pub z: f64,
    /// Velocity X.
    pub vx: f64,
    /// Velocity Y.
    pub vy: f64,
    /// Velocity Z.
    pub vz: f64,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Relative flags bitmask.
    pub flags: i32,
    /// On-ground flag.
    pub on_ground: bool,
}

impl Packet for TeleportEntity {
    const ID: i32 = 0x80;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            vx: f64::decode(buf)?,
            vy: f64::decode(buf)?,
            vz: f64::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            flags: i32::decode(buf)?,
            on_ground: bool::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.vx.encode(buf)?;
        self.vy.encode(buf)?;
        self.vz.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)?;
        self.on_ground.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Rotate Head` packet (`0x4C` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotateHead {
    /// Entity ID.
    pub entity_id: VarInt,
    /// Head yaw in 1/256ths of a full turn.
    pub head_yaw: i8,
}

impl Packet for RotateHead {
    const ID: i32 = 0x55;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            head_yaw: u8::decode(buf)? as i8,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        (self.head_yaw as u8).encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Set Player Position` packet (`0x1C` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovePlayerPos {
    /// X coordinate.
    pub x: f64,
    /// Feet Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
    /// Movement flags bitmask (`0x01` = on_ground).
    pub flags: u8,
}

impl Packet for MovePlayerPos {
    const ID: i32 = 0x1E;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            flags: u8::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.flags.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Set Player Position and Rotation` packet (`0x1D` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovePlayerPosRot {
    /// X coordinate.
    pub x: f64,
    /// Feet Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Movement flags bitmask.
    pub flags: u8,
}

impl Packet for MovePlayerPosRot {
    const ID: i32 = 0x1F;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            x: f64::decode(buf)?,
            y: f64::decode(buf)?,
            z: f64::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            flags: u8::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.x.encode(buf)?;
        self.y.encode(buf)?;
        self.z.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Set Player Rotation` packet (`0x1E` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovePlayerRot {
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Movement flags bitmask.
    pub flags: u8,
}

impl Packet for MovePlayerRot {
    const ID: i32 = 0x20;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
            flags: u8::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        self.flags.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Chat Message` packet (`0x07` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerboundChat {
    /// Plain chat message text.
    pub message: String,
    /// Client timestamp in ms.
    pub timestamp: i64,
    /// Random salt.
    pub salt: i64,
}

impl Packet for ServerboundChat {
    const ID: i32 = 0x09;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let message = read_string(buf, 256)?;
        let timestamp = i64::decode(buf)?;
        let salt = i64::decode(buf)?;
        let has_signature = bool::decode(buf)?;
        if has_signature {
            if buf.remaining() < 256 {
                return Err(crate::error::Error::UnexpectedEof {
                    needed: 256 - buf.remaining(),
                });
            }
            buf.advance(256);
        }
        let message_count = VarInt::decode(buf)?.0;
        if !(0..=20).contains(&message_count) {
            return Err(crate::error::Error::InvalidPacket(format!(
                "invalid chat ack message_count: {message_count}"
            )));
        }
        if buf.remaining() < 4 {
            return Err(crate::error::Error::UnexpectedEof {
                needed: 4 - buf.remaining(),
            });
        }
        buf.advance(3); // acknowledged bitset (20 bits -> 3 bytes)
        let _checksum = u8::decode(buf)?;
        Ok(Self {
            message,
            timestamp,
            salt,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        write_string(buf, &self.message, 256)?;
        self.timestamp.encode(buf)?;
        self.salt.encode(buf)?;
        false.encode(buf)?; // has_signature = false
        VarInt(0).encode(buf)?; // message_count
        buf.put_bytes(0, 3); // acknowledged bitset (20 bits -> 3 bytes)
        0u8.encode(buf)?; // checksum
        Ok(())
    }
}

#[cfg(test)]
mod world_tests {
    use super::*;
    use crate::codec::{decode_frame, encode_packet};
    use crate::types::LpVec3;
    use bytes::BytesMut;

    #[test]
    fn world_and_entity_packets_roundtrip() {
        let mut buf = BytesMut::new();

        let chunk = LevelChunkWithLight {
            chunk_x: 1,
            chunk_z: -2,
            heightmaps: vec![HeightmapEntry {
                kind: VarInt(4),
                data: vec![0; 37],
            }],
            data: vec![1, 2, 3, 4],
            sky_light_mask: vec![],
            block_light_mask: vec![],
            empty_sky_light_mask: vec![0xFF, 0xFF],
            empty_block_light_mask: vec![0xFF, 0xFF],
            sky_lights: vec![],
            block_lights: vec![],
        };
        encode_packet(&chunk, &mut buf).unwrap();
        let dec_chunk: LevelChunkWithLight = decode_frame(&mut buf, 65536)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_chunk, chunk);

        let info = PlayerInfoUpdate {
            entries: vec![PlayerInfoAddEntry {
                uuid: Uuid::nil(),
                name: "Steve".into(),
                properties: vec![GameProfileProperty {
                    name: "textures".into(),
                    value: "e30=".into(),
                    signature: Some("sig".into()),
                }],
                game_mode: VarInt(2),
                listed: true,
                latency: VarInt(12),
            }],
        };
        buf.clear();
        encode_packet(&info, &mut buf).unwrap();
        let dec_info: PlayerInfoUpdate = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_info, info);

        let spawn = SpawnEntity {
            entity_id: VarInt(7),
            uuid: Uuid::nil(),
            entity_type: VarInt(128),
            x: 10.5,
            y: 65.0,
            z: -3.25,
            pitch: 0,
            yaw: 64,
            head_yaw: 64,
            data: VarInt(0),
            velocity: LpVec3::ZERO,
        };
        buf.clear();
        encode_packet(&spawn, &mut buf).unwrap();
        let dec_spawn: SpawnEntity = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_spawn, spawn);

        let mv = MovePlayerPosRot {
            x: 1.0,
            y: 65.0,
            z: 2.0,
            yaw: 90.0,
            pitch: -15.0,
            flags: 1,
        };
        buf.clear();
        encode_packet(&mv, &mut buf).unwrap();
        let dec_mv: MovePlayerPosRot = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_mv, mv);

        let chat = ServerboundChat {
            message: "Hello RadServer!".into(),
            timestamp: 123456,
            salt: 789,
        };
        buf.clear();
        encode_packet(&chat, &mut buf).unwrap();
        let dec_chat: ServerboundChat = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_chat, chat);
    }
}

/// Serverbound `Set Player Movement Flags` / `MovePlayerStatusOnly` packet (`0x1F` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovePlayerStatusOnly {
    /// Movement flags (`0x01` = on ground, `0x02` = horizontal collision).
    pub flags: u8,
}

impl MovePlayerStatusOnly {
    /// Returns `true` if the `on_ground` bit is set.
    #[must_use]
    pub const fn on_ground(self) -> bool {
        (self.flags & 0x01) != 0
    }
}

impl Packet for MovePlayerStatusOnly {
    const ID: i32 = 0x21;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            flags: u8::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.flags.encode(buf)
    }
}

#[cfg(test)]
mod stage1_entity_tests {
    use super::*;
    use crate::codec::{RawPacket, decode_frame, encode_packet};
    use bytes::BytesMut;

    #[test]
    fn move_entity_pos_rot_stepped_and_zero_step_vectors() {
        // 0 steps, on_ground=true -> properties=0x01, then 3 i16, yaw, pitch
        let mut buf = BytesMut::new();
        let zero_step = MoveEntityPosRot {
            entity_id: VarInt(5),
            steps: Vec::new(),
            dx: 256,
            dy: -128,
            dz: 512,
            yaw: 64,
            pitch: -32,
            on_ground: true,
        };
        encode_packet(&zero_step, &mut buf).unwrap();
        let raw = decode_frame(&mut buf, 1024).unwrap().unwrap();
        assert_eq!(raw.id, 0x37);
        assert_eq!(
            raw.payload.as_ref(),
            &[
                5, // entity_id
                1, // (0 << 1) | 1
                0x01, 0x00, // dx=256
                0xFF, 0x80, // dy=-128
                0x02, 0x00, // dz=512
                64,   // yaw
                0xE0, // pitch=-32 as u8
            ]
        );
        assert_eq!(raw.decode_as::<MoveEntityPosRot>().unwrap(), zero_step);

        // 2 steps, on_ground=false -> properties=4
        let stepped = MoveEntityPosRot {
            entity_id: VarInt(9),
            steps: vec![
                EntityMoveStep {
                    ticks: VarInt(1),
                    dx: 100,
                    dy: 0,
                    dz: -50,
                },
                EntityMoveStep {
                    ticks: VarInt(2),
                    dx: 200,
                    dy: 10,
                    dz: -25,
                },
            ],
            dx: 100,
            dy: 0,
            dz: -50,
            yaw: 0,
            pitch: 16,
            on_ground: false,
        };
        buf.clear();
        encode_packet(&stepped, &mut buf).unwrap();
        let dec: MoveEntityPosRot = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec, stepped);

        // Implausible step count must fail cleanly
        let bad = RawPacket {
            id: MoveEntityPosRot::ID,
            payload: bytes::Bytes::from_static(&[1, 20, 0, 0]),
        };
        assert!(bad.decode_as::<MoveEntityPosRot>().is_err());
    }

    #[test]
    fn serverbound_chat_strict_validation() {
        let mut buf = BytesMut::new();
        "hi".to_owned().encode(&mut buf).unwrap();
        1i64.encode(&mut buf).unwrap();
        2i64.encode(&mut buf).unwrap();
        false.encode(&mut buf).unwrap();
        VarInt(99).encode(&mut buf).unwrap(); // invalid message_count > 20
        buf.extend_from_slice(&[0, 0, 0, 0]);
        let bad = RawPacket {
            id: ServerboundChat::ID,
            payload: buf.freeze(),
        };
        assert!(bad.decode_as::<ServerboundChat>().is_err());
    }

    #[test]
    fn set_entity_data_and_take_item_entity_roundtrip() {
        let mut buf = BytesMut::new();
        let pkt = SetEntityData {
            entity_id: VarInt(42),
            entries: vec![
                MetadataEntry {
                    index: 0,
                    value: MetadataValue::Byte(0x20),
                },
                MetadataEntry {
                    index: 8,
                    value: MetadataValue::ItemStack(ItemStack {
                        item_id: 789,
                        count: 3,
                        components: crate::packet::play::ItemComponentPatch {
                            custom_name: Some("Lucky Blade".to_owned()),
                            damage: Some(12),
                            ..Default::default()
                        },
                    }),
                },
            ],
        };
        encode_packet(&pkt, &mut buf).unwrap();
        let dec: SetEntityData = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec, pkt);

        buf.clear();
        let take = TakeItemEntity {
            collected_entity_id: VarInt(42),
            collector_entity_id: VarInt(7),
            pickup_item_count: VarInt(3),
        };
        encode_packet(&take, &mut buf).unwrap();
        let dec_take: TakeItemEntity = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_take, take);
    }
}
