//! World chunks and block interaction Play packets.

use super::*;
/// Clientbound `Set Chunk Cache Center` packet (`0x57` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetChunkCacheCenter {
    /// Center chunk X coordinate.
    pub chunk_x: VarInt,
    /// Center chunk Z coordinate.
    pub chunk_z: VarInt,
}

impl Packet for SetChunkCacheCenter {
    const ID: i32 = 0x60;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            chunk_x: VarInt::decode(buf)?,
            chunk_z: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.chunk_x.encode(buf)?;
        self.chunk_z.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Set Chunk Cache Radius` packet (`0x58` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetChunkCacheRadius {
    /// Server view distance in chunks (`2..=32`).
    pub view_distance: VarInt,
}

impl Packet for SetChunkCacheRadius {
    const ID: i32 = 0x61;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            view_distance: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.view_distance.encode(buf)
    }
}

/// Clientbound `Unload Chunk` (`ForgetLevelChunk`) packet (`0x21` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnloadChunk {
    /// Chunk X coordinate.
    pub chunk_x: i32,
    /// Chunk Z coordinate.
    pub chunk_z: i32,
}

impl Packet for UnloadChunk {
    const ID: i32 = 0x26;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let chunk_z = i32::decode(buf)?;
        let chunk_x = i32::decode(buf)?;
        Ok(Self { chunk_x, chunk_z })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.chunk_z.encode(buf)?;
        self.chunk_x.encode(buf)?;
        Ok(())
    }
}

/// A single heightmap entry in [`LevelChunkWithLight`] (MC 1.21.5+ / 26.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeightmapEntry {
    /// Heightmap type ID (e.g., `1` = `WORLD_SURFACE`, `4` = `MOTION_BLOCKING`).
    pub kind: VarInt,
    /// Bit-packed height values (`i64` words).
    pub data: Vec<i64>,
}

/// Clientbound `Level Chunk With Light` packet (`0x2E` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelChunkWithLight {
    /// Chunk X coordinate.
    pub chunk_x: i32,
    /// Chunk Z coordinate.
    pub chunk_z: i32,
    /// Heightmaps list (`type: VarInt, longs: Vec<i64>`).
    pub heightmaps: Vec<HeightmapEntry>,
    /// Serialized chunk section bytes.
    pub data: Vec<u8>,
    /// Sky light bitmask bytes (`BitSet` byte array in 777).
    pub sky_light_mask: Vec<u8>,
    /// Block light bitmask bytes (`BitSet` byte array in 777).
    pub block_light_mask: Vec<u8>,
    /// Empty sky light bitmask bytes (`BitSet` byte array in 777).
    pub empty_sky_light_mask: Vec<u8>,
    /// Empty block light bitmask bytes (`BitSet` byte array in 777).
    pub empty_block_light_mask: Vec<u8>,
    /// Sky light arrays (2048 bytes each).
    pub sky_lights: Vec<Vec<u8>>,
    /// Block light arrays (2048 bytes each).
    pub block_lights: Vec<Vec<u8>>,
}

impl Packet for LevelChunkWithLight {
    const ID: i32 = 0x2E;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let chunk_x = i32::decode(buf)?;
        let chunk_z = i32::decode(buf)?;
        let heightmaps = decode_heightmaps(buf)?;
        let data_len = decode_bounded_count(buf, 4 * 1024 * 1024, "chunk data")?;
        if buf.remaining() < data_len {
            return Err(crate::error::Error::UnexpectedEof { needed: data_len });
        }
        let mut data = vec![0u8; data_len];
        buf.copy_to_slice(&mut data);

        // Block entities (skip for MVP)
        let be_count = decode_bounded_count(buf, 4096, "block entities")?;
        for _ in 0..be_count {
            let _ = u8::decode(buf)?;
            let _ = i16::decode(buf)?;
            let _ = VarInt::decode(buf)?;
            let _ = decode_network_nbt(buf)?;
        }

        let sky_light_mask = decode_byte_mask(buf)?;
        let block_light_mask = decode_byte_mask(buf)?;
        let empty_sky_light_mask = decode_byte_mask(buf)?;
        let empty_block_light_mask = decode_byte_mask(buf)?;
        let sky_lights = decode_light_arrays(buf)?;
        let block_lights = decode_light_arrays(buf)?;

        Ok(Self {
            chunk_x,
            chunk_z,
            heightmaps,
            data,
            sky_light_mask,
            block_light_mask,
            empty_sky_light_mask,
            empty_block_light_mask,
            sky_lights,
            block_lights,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.chunk_x.encode(buf)?;
        self.chunk_z.encode(buf)?;
        encode_heightmaps(&self.heightmaps, buf)?;
        VarInt(self.data.len() as i32).encode(buf)?;
        buf.put_slice(&self.data);
        VarInt(0).encode(buf)?; // 0 block entities
        encode_byte_mask(&self.sky_light_mask, buf)?;
        encode_byte_mask(&self.block_light_mask, buf)?;
        encode_byte_mask(&self.empty_sky_light_mask, buf)?;
        encode_byte_mask(&self.empty_block_light_mask, buf)?;
        encode_light_arrays(&self.sky_lights, buf)?;
        encode_light_arrays(&self.block_lights, buf)?;
        Ok(())
    }
}

fn decode_bounded_count(buf: &mut impl Buf, limit: usize, label: &str) -> Result<usize> {
    let raw = VarInt::decode(buf)?.0;
    let count = usize::try_from(raw).map_err(|_| {
        crate::error::Error::InvalidPacket(format!("negative {label} count: {raw}"))
    })?;
    if count > limit {
        return Err(crate::error::Error::InvalidPacket(format!(
            "{label} count {count} exceeds limit {limit}"
        )));
    }
    Ok(count)
}

fn decode_heightmaps(buf: &mut impl Buf) -> Result<Vec<HeightmapEntry>> {
    let count = decode_bounded_count(buf, 16, "heightmaps")?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = VarInt::decode(buf)?;
        let long_len = decode_bounded_count(buf, 64, "heightmap longs")?;
        let mut data = Vec::with_capacity(long_len);
        for _ in 0..long_len {
            data.push(i64::decode(buf)?);
        }
        out.push(HeightmapEntry { kind, data });
    }
    Ok(out)
}

fn encode_heightmaps(entries: &[HeightmapEntry], buf: &mut impl BufMut) -> Result<()> {
    VarInt(entries.len() as i32).encode(buf)?;
    for entry in entries {
        entry.kind.encode(buf)?;
        VarInt(entry.data.len() as i32).encode(buf)?;
        for &word in &entry.data {
            word.encode(buf)?;
        }
    }
    Ok(())
}

fn decode_byte_mask(buf: &mut impl Buf) -> Result<Vec<u8>> {
    let len = decode_bounded_count(buf, 64, "light mask")?;
    if buf.remaining() < len {
        return Err(crate::error::Error::UnexpectedEof { needed: len });
    }
    let mut out = vec![0u8; len];
    buf.copy_to_slice(&mut out);
    Ok(out)
}

fn encode_byte_mask(bytes: &[u8], buf: &mut impl BufMut) -> Result<()> {
    VarInt(bytes.len() as i32).encode(buf)?;
    buf.put_slice(bytes);
    Ok(())
}

fn decode_light_arrays(buf: &mut impl Buf) -> Result<Vec<Vec<u8>>> {
    let count = decode_bounded_count(buf, 64, "light arrays")?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let len = decode_bounded_count(buf, 4096, "light array bytes")?;
        if buf.remaining() < len {
            return Err(crate::error::Error::UnexpectedEof { needed: len });
        }
        let mut arr = vec![0u8; len];
        buf.copy_to_slice(&mut arr);
        out.push(arr);
    }
    Ok(out)
}

fn encode_light_arrays(arrays: &[Vec<u8>], buf: &mut impl BufMut) -> Result<()> {
    VarInt(arrays.len() as i32).encode(buf)?;
    for arr in arrays {
        VarInt(arr.len() as i32).encode(buf)?;
        buf.put_slice(arr);
    }
    Ok(())
}

/// Clientbound `Block Update` packet (`0x09` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockUpdate {
    /// Block coordinates.
    pub pos: BlockPos,
    /// Global block state ID.
    pub block_state: VarInt,
}

impl Packet for BlockUpdate {
    const ID: i32 = 0x08;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            pos: BlockPos::decode(buf)?,
            block_state: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.pos.encode(buf)?;
        self.block_state.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Acknowledge Block Change` packet (`0x04` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcknowledgeBlockChange {
    /// Prediction sequence ID echoed from `PlayerAction` / `UseItemOn`.
    pub sequence: VarInt,
}

impl Packet for AcknowledgeBlockChange {
    const ID: i32 = 0x04;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            sequence: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.sequence.encode(buf)
    }
}

/// Serverbound `Player Action` packet (`0x28` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAction {
    /// Action status (`0`=started digging, `1`=cancelled, `2`=finished digging, etc.).
    pub status: VarInt,
    /// Target block position.
    pub location: BlockPos,
    /// Block face (`0`=-Y, `1`=+Y, `2`=-Z, `3`=+Z, `4`=-X, `5`=+X).
    pub face: u8,
    /// Prediction sequence ID.
    pub sequence: VarInt,
}

impl Packet for PlayerAction {
    const ID: i32 = 0x29;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            status: VarInt::decode(buf)?,
            location: BlockPos::decode(buf)?,
            face: u8::decode(buf)?,
            sequence: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.status.encode(buf)?;
        self.location.encode(buf)?;
        self.face.encode(buf)?;
        self.sequence.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Use Item On` (block placement / interaction) packet (`0x3B` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseItemOn {
    /// Hand (`0` = main hand, `1` = off hand).
    pub hand: VarInt,
    /// Clicked block position.
    pub location: BlockPos,
    /// Clicked block face (`0`=-Y, `1`=+Y, `2`=-Z, `3`=+Z, `4`=-X, `5`=+X).
    pub face: VarInt,
    /// Cursor X on face (`0.0..=1.0`).
    pub cursor_x: f32,
    /// Cursor Y on face (`0.0..=1.0`).
    pub cursor_y: f32,
    /// Cursor Z on face (`0.0..=1.0`).
    pub cursor_z: f32,
    /// Whether player head is inside a block.
    pub inside_block: bool,
    /// Whether world border was hit.
    pub world_border_hit: bool,
    /// Prediction sequence ID.
    pub sequence: VarInt,
}

impl Packet for UseItemOn {
    const ID: i32 = 0x42;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            hand: VarInt::decode(buf)?,
            location: BlockPos::decode(buf)?,
            face: VarInt::decode(buf)?,
            cursor_x: f32::decode(buf)?,
            cursor_y: f32::decode(buf)?,
            cursor_z: f32::decode(buf)?,
            inside_block: bool::decode(buf)?,
            world_border_hit: bool::decode(buf)?,
            sequence: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.hand.encode(buf)?;
        self.location.encode(buf)?;
        self.face.encode(buf)?;
        self.cursor_x.encode(buf)?;
        self.cursor_y.encode(buf)?;
        self.cursor_z.encode(buf)?;
        self.inside_block.encode(buf)?;
        self.world_border_hit.encode(buf)?;
        self.sequence.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Use Item` (consume food / activate held item) packet (`0x43` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseItem {
    /// Hand (`0` = main hand, `1` = off hand).
    pub hand: VarInt,
    /// Prediction sequence ID.
    pub sequence: VarInt,
    /// Player yaw in degrees.
    pub yaw: f32,
    /// Player pitch in degrees.
    pub pitch: f32,
}

impl Packet for UseItem {
    const ID: i32 = 0x43;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            hand: VarInt::decode(buf)?,
            sequence: VarInt::decode(buf)?,
            yaw: f32::decode(buf)?,
            pitch: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.hand.encode(buf)?;
        self.sequence.encode(buf)?;
        self.yaw.encode(buf)?;
        self.pitch.encode(buf)?;
        Ok(())
    }
}

/// Network state for a synchronized `minecraft:world_clock` entry in Minecraft 26.3 (`777`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockNetworkState {
    /// Total accumulated ticks for this clock.
    pub total_ticks: crate::types::VarLong,
    /// Fractional tick interpolation offset (`0.0` on whole ticks).
    pub partial_tick: f32,
    /// Tick rate multiplier (`1.0` for normal speed, `0.0` when paused).
    pub rate: f32,
}

impl ClockNetworkState {
    fn decode_from(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            total_ticks: crate::types::VarLong::decode(buf)?,
            partial_tick: f32::decode(buf)?,
            rate: f32::decode(buf)?,
        })
    }

    fn encode_to(&self, buf: &mut impl BufMut) -> Result<()> {
        self.total_ticks.encode(buf)?;
        self.partial_tick.encode(buf)?;
        self.rate.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Set Time` packet (`0x73` in Play, Minecraft 26.3 / protocol 777).
#[derive(Debug, Clone, PartialEq)]
pub struct SetTime {
    /// Total world age in ticks (`Time`).
    pub game_time: i64,
    /// Synchronized world clock updates: `(clock_registry_id + 1, state)`.
    pub clock_updates: Vec<(VarInt, ClockNetworkState)>,
}

impl SetTime {
    /// Builds a `SetTime` packet updating the default `minecraft:overworld` clock (registry ID `0`,
    /// encoded via `holderRegistry` as `VarInt(0)`).
    #[must_use]
    pub fn overworld(game_time: i64, total_ticks: i64, rate: f32) -> Self {
        Self {
            game_time,
            clock_updates: vec![(
                VarInt(0),
                ClockNetworkState {
                    total_ticks: crate::types::VarLong(total_ticks.max(0)),
                    partial_tick: 0.0,
                    rate,
                },
            )],
        }
    }
}

impl Packet for SetTime {
    const ID: i32 = 0x73;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let game_time = i64::decode(buf)?;
        let count = VarInt::decode(buf)?.0;
        if !(0..=128).contains(&count) {
            return Err(crate::error::Error::InvalidPacket(format!(
                "invalid clock_updates count {count}"
            )));
        }
        let mut clock_updates = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let clock_holder = VarInt::decode(buf)?;
            let state = ClockNetworkState::decode_from(buf)?;
            clock_updates.push((clock_holder, state));
        }
        Ok(Self {
            game_time,
            clock_updates,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.game_time.encode(buf)?;
        VarInt(
            i32::try_from(self.clock_updates.len())
                .map_err(|_| crate::error::Error::InvalidPacket("too many clock updates".into()))?,
        )
        .encode(buf)?;
        for (clock_holder, state) in &self.clock_updates {
            clock_holder.encode(buf)?;
            state.encode_to(buf)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod world_time_tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn set_time_roundtrip_26_3() {
        let pkt = SetTime::overworld(12_345, 18_000, 1.0);
        let mut buf = BytesMut::new();
        crate::codec::encode_packet(&pkt, &mut buf).unwrap();
        let frame = crate::codec::decode_frame(&mut buf, 1024).unwrap().unwrap();
        assert_eq!(frame.id, SetTime::ID);
        let decoded: SetTime = frame.decode_as().unwrap();
        assert_eq!(decoded, pkt);
    }

    #[test]
    fn pick_item_from_block_roundtrip_26_3() {
        let pkt = PickItemFromBlock {
            location: BlockPos::new(10, 64, -5),
            include_data: true,
        };
        let mut buf = BytesMut::new();
        crate::codec::encode_packet(&pkt, &mut buf).unwrap();
        let frame = crate::codec::decode_frame(&mut buf, 1024).unwrap().unwrap();
        assert_eq!(frame.id, PickItemFromBlock::ID);
        let decoded: PickItemFromBlock = frame.decode_as().unwrap();
        assert_eq!(decoded, pkt);
    }
}

/// Serverbound `Pick Item From Block` packet (`0x24` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickItemFromBlock {
    /// Target block position.
    pub location: BlockPos,
    /// Whether block entity data should be included.
    pub include_data: bool,
}

impl Packet for PickItemFromBlock {
    const ID: i32 = 0x24;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let location = BlockPos::decode(buf)?;
        let include_data = bool::decode(buf)?;
        Ok(Self {
            location,
            include_data,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.location.encode(buf)?;
        self.include_data.encode(buf)?;
        Ok(())
    }
}
