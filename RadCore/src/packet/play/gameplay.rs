//! Inventory, survival health/respawn, and combat Play packets.

use super::*;
use crate::types::LpVec3;
/// Serverbound `Set Carried Item` (hotbar slot change) packet (`0x33` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCarriedItem {
    /// Selected hotbar slot (`0..=8`).
    pub slot: i16,
}

impl Packet for SetCarriedItem {
    const ID: i32 = 0x36;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            slot: i16::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.slot.encode(buf)
    }
}

/// Clientbound selected hotbar slot for restoring the player's selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetHeldSlot {
    /// Selected hotbar slot (`0..=8`).
    pub slot: VarInt,
}

impl Packet for SetHeldSlot {
    const ID: i32 = 0x6b;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;
    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            slot: VarInt::decode(buf)?,
        })
    }
    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.slot.encode(buf)
    }
}

/// Minimal item slot representation for container packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimpleSlot {
    /// Item count (`0` = empty slot).
    pub count: i32,
    /// Registry item ID when `count > 0`.
    pub item_id: i32,
}

/// Creative inventory slot update sent by a creative-mode client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCreativeModeSlot {
    /// Inventory slot, or -1 to drop the supplied item stack.
    pub slot: i16,
    /// The item stack supplied by the client.
    pub item: ItemStack,
}

impl Packet for SetCreativeModeSlot {
    const ID: i32 = 0x39;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            slot: i16::decode(buf)?,
            item: ItemStack::decode_slot(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.slot.encode(buf)?;
        self.item.encode_slot(buf)
    }
}

/// Supported item data components carried in `ItemStack` wire/persistence patches.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ItemComponentPatch {
    /// Custom display name (`minecraft:custom_name`, component ID `5`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name: Option<String>,
    /// Current item durability damage (`minecraft:damage`, component ID `3`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<i32>,
    /// Maximum durability (`minecraft:max_damage`, component ID `2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_damage: Option<i32>,
    /// Anvil repair cost (`minecraft:repair_cost`, component ID `15`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repair_cost: Option<i32>,
    /// Unbreakable flag (`minecraft:unbreakable`, component ID `4`).
    #[serde(default)]
    pub unbreakable: bool,
    /// Removed default component IDs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<i32>,
}

impl ItemComponentPatch {
    /// Returns `true` when no components are added or removed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.custom_name.is_none()
            && self.damage.is_none()
            && self.max_damage.is_none()
            && self.repair_cost.is_none()
            && !self.unbreakable
            && self.removed.is_empty()
    }
}

/// Full item stack with optional data component patch.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ItemStack {
    /// Item count (`0` = empty slot).
    pub count: i32,
    /// Registry item ID when `count > 0`.
    pub item_id: i32,
    /// Component additions/removals.
    #[serde(default, skip_serializing_if = "ItemComponentPatch::is_empty")]
    pub components: ItemComponentPatch,
}

impl ItemStack {
    /// Converts to a lightweight [`SimpleSlot`].
    #[must_use]
    pub const fn to_simple(&self) -> SimpleSlot {
        SimpleSlot {
            count: self.count,
            item_id: self.item_id,
        }
    }

    /// Encodes this `ItemStack` using protocol 777 slot + data component patch format.
    pub fn encode_slot(&self, buf: &mut impl BufMut) -> Result<()> {
        if self.count <= 0 {
            return VarInt(0).encode(buf);
        }
        VarInt(self.count).encode(buf)?;
        VarInt(self.item_id).encode(buf)?;
        let mut added_count = 0i32;
        if self.components.max_damage.is_some() {
            added_count += 1;
        }
        if self.components.damage.is_some() {
            added_count += 1;
        }
        if self.components.unbreakable {
            added_count += 1;
        }
        if self.components.custom_name.is_some() {
            added_count += 1;
        }
        if self.components.repair_cost.is_some() {
            added_count += 1;
        }
        VarInt(added_count).encode(buf)?;
        VarInt(i32::try_from(self.components.removed.len()).unwrap_or(0)).encode(buf)?;
        if let Some(max_dmg) = self.components.max_damage {
            VarInt(2).encode(buf)?;
            VarInt(max_dmg).encode(buf)?;
        }
        if let Some(dmg) = self.components.damage {
            VarInt(3).encode(buf)?;
            VarInt(dmg).encode(buf)?;
        }
        if self.components.unbreakable {
            VarInt(4).encode(buf)?;
        }
        if let Some(ref name) = self.components.custom_name {
            VarInt(5).encode(buf)?;
            crate::nbt::encode_network_nbt(&crate::nbt::NbtTag::String(name.clone()), buf)?;
        }
        if let Some(rc) = self.components.repair_cost {
            VarInt(15).encode(buf)?;
            VarInt(rc).encode(buf)?;
        }
        for &rem_id in &self.components.removed {
            VarInt(rem_id).encode(buf)?;
        }
        Ok(())
    }

    /// Decodes an `ItemStack` with supported protocol 777 data components.
    pub fn decode_slot(buf: &mut impl Buf) -> Result<Self> {
        let count = VarInt::decode(buf)?.0;
        if count < 0 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "negative slot count: {count}"
            )));
        }
        if count == 0 {
            return Ok(Self::default());
        }
        let item_id = VarInt::decode(buf)?.0;
        let added = VarInt::decode(buf)?.0;
        let removed_count = VarInt::decode(buf)?.0;
        if added < 0 || removed_count < 0 || added > 64 || removed_count > 64 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "invalid item component counts (added={added}, removed={removed_count})"
            )));
        }
        let mut components = ItemComponentPatch::default();
        for _ in 0..added {
            let comp_id = VarInt::decode(buf)?.0;
            match comp_id {
                2 => components.max_damage = Some(VarInt::decode(buf)?.0),
                3 => components.damage = Some(VarInt::decode(buf)?.0),
                4 => components.unbreakable = true,
                5 => {
                    let tag = crate::nbt::decode_network_nbt(buf)?;
                    if let crate::nbt::NbtTag::String(s) = tag {
                        components.custom_name = Some(s);
                    }
                }
                15 => components.repair_cost = Some(VarInt::decode(buf)?.0),
                other => {
                    return Err(crate::error::Error::InvalidPacket(format!(
                        "unsupported item data component id {other}"
                    )));
                }
            }
        }
        for _ in 0..removed_count {
            components.removed.push(VarInt::decode(buf)?.0);
        }
        Ok(Self {
            count,
            item_id,
            components,
        })
    }
}

impl SimpleSlot {
    fn encode_slot(&self, buf: &mut impl BufMut) -> Result<()> {
        if self.count <= 0 {
            VarInt(0).encode(buf)?;
        } else {
            VarInt(self.count).encode(buf)?;
            VarInt(self.item_id).encode(buf)?;
            VarInt(0).encode(buf)?; // 0 added data components
            VarInt(0).encode(buf)?; // 0 removed data components
        }
        Ok(())
    }

    fn decode_slot(buf: &mut impl Buf) -> Result<Self> {
        let stack = ItemStack::decode_slot(buf)?;
        Ok(stack.to_simple())
    }
}

/// Clientbound `Container Set Content` packet (`0x12` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSetContent {
    /// Window ID (`0` = player inventory).
    pub window_id: VarInt,
    /// Container state revision ID.
    pub state_id: VarInt,
    /// Inventory slots.
    pub slots: Vec<SimpleSlot>,
    /// Item currently carried by cursor.
    pub carried: SimpleSlot,
}

impl Packet for ContainerSetContent {
    const ID: i32 = 0x12;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let window_id = VarInt::decode(buf)?;
        let state_id = VarInt::decode(buf)?;
        let raw_count = VarInt::decode(buf)?.0;
        let count = usize::try_from(raw_count).map_err(|_| {
            crate::error::Error::InvalidPacket(format!("negative slot list count: {raw_count}"))
        })?;
        if count > 256 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "slot list count {count} exceeds limit 256"
            )));
        }
        let mut slots = Vec::with_capacity(count);
        for _ in 0..count {
            slots.push(SimpleSlot::decode_slot(buf)?);
        }
        let carried = SimpleSlot::decode_slot(buf)?;
        Ok(Self {
            window_id,
            state_id,
            slots,
            carried,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.state_id.encode(buf)?;
        VarInt(self.slots.len() as i32).encode(buf)?;
        for slot in &self.slots {
            slot.encode_slot(buf)?;
        }
        self.carried.encode_slot(buf)?;
        Ok(())
    }
}

/// Clientbound `Container Set Content` packet (`0x12` in Play) preserving full [`ItemStack`] components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSetContentStacks {
    /// Window ID (`0` = player inventory).
    pub window_id: VarInt,
    /// Container state revision ID.
    pub state_id: VarInt,
    /// Inventory slots with component patches.
    pub slots: Vec<ItemStack>,
    /// Item currently carried by cursor.
    pub carried: ItemStack,
}

impl Packet for ContainerSetContentStacks {
    const ID: i32 = 0x12;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let window_id = VarInt::decode(buf)?;
        let state_id = VarInt::decode(buf)?;
        let count = VarInt::decode(buf)?.0;
        if !(0..=256).contains(&count) {
            return Err(crate::error::Error::InvalidPacket(format!(
                "container slot count {count} out of bounds"
            )));
        }
        let mut slots = Vec::with_capacity(count as usize);
        for _ in 0..count {
            slots.push(ItemStack::decode_slot(buf)?);
        }
        let carried = ItemStack::decode_slot(buf)?;
        Ok(Self {
            window_id,
            state_id,
            slots,
            carried,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.state_id.encode(buf)?;
        VarInt(self.slots.len() as i32).encode(buf)?;
        for slot in &self.slots {
            slot.encode_slot(buf)?;
        }
        self.carried.encode_slot(buf)?;
        Ok(())
    }
}

/// Serverbound `Container Click` packet (`0x12` in Play, protocol `777`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerClick {
    /// Window ID (`0` = player inventory).
    pub window_id: VarInt,
    /// Container state revision ID.
    pub state_id: VarInt,
    /// Clicked slot index (`-999` = outside window).
    pub slot: i16,
    /// Mouse button / hotbar key.
    pub button: i8,
    /// Click mode (`0` = pickup, `1` = quick-move/shift, `2` = swap, `4` = drop).
    pub mode: VarInt,
    /// Client-reported modified slots `(slot_index, new_item)`.
    pub changed_slots: Vec<(i16, SimpleSlot)>,
    /// Item currently carried on the cursor after click.
    pub carried_item: SimpleSlot,
}

impl Packet for ContainerClick {
    const ID: i32 = 0x12;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let window_id = VarInt::decode(buf)?;
        let state_id = VarInt::decode(buf)?;
        let slot = i16::decode(buf)?;
        let button = u8::decode(buf)? as i8;
        let mode = VarInt::decode(buf)?;
        let raw_count = VarInt::decode(buf)?.0;
        let count = usize::try_from(raw_count).map_err(|_| {
            crate::error::Error::InvalidPacket(format!("negative changed_slots count: {raw_count}"))
        })?;
        if count > 128 {
            return Err(crate::error::Error::InvalidPacket(format!(
                "changed_slots count {count} exceeds limit 128"
            )));
        }
        let mut changed_slots = Vec::with_capacity(count);
        for _ in 0..count {
            let idx = i16::decode(buf)?;
            let item = SimpleSlot::decode_slot(buf)?;
            changed_slots.push((idx, item));
        }
        let carried_item = SimpleSlot::decode_slot(buf)?;
        Ok(Self {
            window_id,
            state_id,
            slot,
            button,
            mode,
            changed_slots,
            carried_item,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.state_id.encode(buf)?;
        self.slot.encode(buf)?;
        (self.button as u8).encode(buf)?;
        self.mode.encode(buf)?;
        VarInt(self.changed_slots.len() as i32).encode(buf)?;
        for (idx, item) in &self.changed_slots {
            idx.encode(buf)?;
            item.encode_slot(buf)?;
        }
        self.carried_item.encode_slot(buf)?;
        Ok(())
    }
}

/// Serverbound `Container Close` packet (`0x13` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerClose {
    /// Window ID being closed.
    pub window_id: VarInt,
}

impl Packet for ContainerClose {
    const ID: i32 = 0x13;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            window_id: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)
    }
}

/// Clientbound `Open Screen` packet (`0x39` in Play, protocol `777`).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenScreen {
    /// Assigned window ID (`>= 1`).
    pub window_id: VarInt,
    /// Menu type registry ID (`2` = `generic_9x3` chest).
    pub window_type: VarInt,
    /// Window title as Network NBT text component.
    pub title: crate::nbt::NbtTag,
}

impl Packet for OpenScreen {
    const ID: i32 = 0x39;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            window_id: VarInt::decode(buf)?,
            window_type: VarInt::decode(buf)?,
            title: crate::nbt::decode_network_nbt(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.window_type.encode(buf)?;
        crate::nbt::encode_network_nbt(&self.title, buf)
    }
}

/// Clientbound `Container Set Data` packet (`0x13` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerSetData {
    /// Window ID.
    pub window_id: VarInt,
    /// Property index (e.g. `0`=fuel left, `1`=max fuel, `2`=cook progress, `3`=cook total).
    pub property: i16,
    /// Property value.
    pub value: i16,
}

impl Packet for ContainerSetData {
    const ID: i32 = 0x13;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            window_id: VarInt::decode(buf)?,
            property: i16::decode(buf)?,
            value: i16::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.property.encode(buf)?;
        self.value.encode(buf)
    }
}

/// Clientbound `Container Set Slot` packet (`0x14` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerSetSlot {
    /// Window ID (`0` = player inventory).
    pub window_id: VarInt,
    /// Container state revision ID.
    pub state_id: VarInt,
    /// Target slot index.
    pub slot: i16,
    /// New slot contents.
    pub item: SimpleSlot,
}

impl Packet for ContainerSetSlot {
    const ID: i32 = 0x14;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            window_id: VarInt::decode(buf)?,
            state_id: VarInt::decode(buf)?,
            slot: i16::decode(buf)?,
            item: SimpleSlot::decode_slot(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.window_id.encode(buf)?;
        self.state_id.encode(buf)?;
        self.slot.encode(buf)?;
        self.item.encode_slot(buf)
    }
}

#[cfg(test)]
mod phase3_tests {
    use super::*;
    use crate::codec::{decode_frame, encode_packet};
    use bytes::BytesMut;

    #[test]
    fn block_and_inventory_packets_roundtrip() {
        let mut buf = BytesMut::new();

        let ack = AcknowledgeBlockChange {
            sequence: VarInt(42),
        };
        encode_packet(&ack, &mut buf).unwrap();
        let dec_ack: AcknowledgeBlockChange = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_ack, ack);

        let pa = PlayerAction {
            status: VarInt(2),
            location: BlockPos::new(10, 64, -5),
            face: 1,
            sequence: VarInt(7),
        };
        buf.clear();
        encode_packet(&pa, &mut buf).unwrap();
        let dec_pa: PlayerAction = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_pa, pa);

        let use_on = UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(10, 63, -5),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(8),
        };
        buf.clear();
        encode_packet(&use_on, &mut buf).unwrap();
        let dec_use: UseItemOn = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_use, use_on);

        let inv = ContainerSetContent {
            window_id: VarInt(0),
            state_id: VarInt(1),
            slots: vec![
                SimpleSlot {
                    count: 0,
                    item_id: 0,
                },
                SimpleSlot {
                    count: 64,
                    item_id: 1,
                },
            ],
            carried: SimpleSlot::default(),
        };
        buf.clear();
        encode_packet(&inv, &mut buf).unwrap();
        let dec_inv: ContainerSetContent = decode_frame(&mut buf, 4096)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_inv, inv);
    }
}

/// Clientbound `Set Health` packet (`0x66` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetHealth {
    /// Health (`0.0..=20.0`, `<= 0.0` triggers death screen).
    pub health: f32,
    /// Food level (`0..=20`).
    pub food: VarInt,
    /// Food saturation (`0.0..=5.0`).
    pub food_saturation: f32,
}

impl Packet for SetHealth {
    const ID: i32 = 0x6A;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            health: f32::decode(buf)?,
            food: VarInt::decode(buf)?,
            food_saturation: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.health.encode(buf)?;
        self.food.encode(buf)?;
        self.food_saturation.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Client Command` packet (`0x0A` in Play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientCommand {
    /// Action (`0` = Perform Respawn, `1` = Request Stats).
    pub action_id: VarInt,
}

impl Packet for ClientCommand {
    const ID: i32 = 0x0C;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            action_id: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.action_id.encode(buf)
    }
}

/// Clientbound `Respawn` packet (`0x50` in Play).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respawn {
    /// Dimension registry ID.
    pub dimension_type: VarInt,
    /// Dimension identifier (e.g. `"minecraft:overworld"`).
    pub dimension_name: String,
    /// Hashed world seed.
    pub hashed_seed: i64,
    /// Current game mode.
    pub game_mode: u8,
    /// Previous game mode (`-1` for none).
    pub previous_game_mode: i8,
    /// Whether this is a debug world.
    pub is_debug: bool,
    /// Whether this is a superflat world.
    pub is_flat: bool,
    /// Optional last death `(dimension, position)`.
    pub death_location: Option<(String, BlockPos)>,
    /// Nether portal cooldown in ticks.
    pub portal_cooldown: VarInt,
    /// Sea level Y coordinate.
    pub sea_level: VarInt,
    /// Data-kept bitmask (`0x01` attributes, `0x02` metadata).
    pub data_kept: u8,
}

impl Packet for Respawn {
    const ID: i32 = 0x54;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        let dimension_type = VarInt::decode(buf)?;
        let dimension_name = String::decode(buf)?;
        let hashed_seed = i64::decode(buf)?;
        let game_mode = decode_game_mode(buf, false)? as u8;
        let previous_game_mode = decode_game_mode(buf, true)?;
        let is_debug = bool::decode(buf)?;
        let is_flat = bool::decode(buf)?;
        let has_death = bool::decode(buf)?;
        let death_location = if has_death {
            Some((String::decode(buf)?, BlockPos::decode(buf)?))
        } else {
            None
        };
        let portal_cooldown = VarInt::decode(buf)?;
        let sea_level = VarInt::decode(buf)?;
        let data_kept = u8::decode(buf)?;
        Ok(Self {
            dimension_type,
            dimension_name,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            death_location,
            portal_cooldown,
            sea_level,
            data_kept,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.dimension_type.encode(buf)?;
        self.dimension_name.encode(buf)?;
        self.hashed_seed.encode(buf)?;
        encode_game_mode(buf, i32::from(self.game_mode), false)?;
        // 26.3 GameType.OPTIONAL_STREAM_CODEC: 0=None, otherwise id+1.
        encode_game_mode(buf, i32::from(self.previous_game_mode), true)?;
        self.is_debug.encode(buf)?;
        self.is_flat.encode(buf)?;
        if let Some((dim, pos)) = &self.death_location {
            true.encode(buf)?;
            dim.encode(buf)?;
            pos.encode(buf)?;
        } else {
            false.encode(buf)?;
        }
        self.portal_cooldown.encode(buf)?;
        self.sea_level.encode(buf)?;
        self.data_kept.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Hurt Animation` packet (`0x26` in Play).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HurtAnimation {
    /// Target entity ID.
    pub entity_id: VarInt,
    /// Attack yaw in degrees.
    pub yaw: f32,
}

impl Packet for HurtAnimation {
    const ID: i32 = 0x2B;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            yaw: f32::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.yaw.encode(buf)?;
        Ok(())
    }
}

/// Clientbound `Set Entity Motion` packet (`0x5F` in Play).
/// Clientbound `Set Entity Motion` packet (`0x67` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetEntityMotion {
    /// Target entity ID.
    pub entity_id: VarInt,
    /// Quantized velocity vector (`LpVec3`).
    pub velocity: LpVec3,
}

impl Packet for SetEntityMotion {
    const ID: i32 = 0x67;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Clientbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            velocity: LpVec3::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.velocity.encode(buf)?;
        Ok(())
    }
}

/// Serverbound `Attack` packet (`0x01` in Play, protocol `777`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attack {
    /// Target entity ID.
    pub entity_id: VarInt,
}

impl Packet for Attack {
    const ID: i32 = 0x01;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)
    }
}

/// Serverbound `Interact` packet (`0x1A` in Play, 777 layout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interact {
    /// Target entity ID.
    pub entity_id: VarInt,
    /// Interaction hand (`0` = main hand, `1` = off hand).
    pub hand: VarInt,
    /// Interaction target offset as [`LpVec3`].
    pub location: LpVec3,
    /// Whether player is sneaking.
    pub sneaking: bool,
}

impl Packet for Interact {
    const ID: i32 = 0x1A;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: PacketDirection = PacketDirection::Serverbound;

    fn decode_body(buf: &mut impl Buf) -> Result<Self> {
        Ok(Self {
            entity_id: VarInt::decode(buf)?,
            hand: VarInt::decode(buf)?,
            location: LpVec3::decode(buf)?,
            sneaking: bool::decode(buf)?,
        })
    }

    fn encode_body(&self, buf: &mut impl BufMut) -> Result<()> {
        self.entity_id.encode(buf)?;
        self.hand.encode(buf)?;
        self.location.encode(buf)?;
        self.sneaking.encode(buf)?;
        Ok(())
    }
}
#[cfg(test)]
mod phase4_tests {
    use super::*;
    use crate::codec::{decode_frame, encode_packet};
    use bytes::BytesMut;

    #[test]
    fn physics_survival_and_combat_packets_roundtrip() {
        let mut buf = BytesMut::new();

        let health = SetHealth {
            health: 14.5,
            food: VarInt(18),
            food_saturation: 3.2,
        };
        encode_packet(&health, &mut buf).unwrap();
        let dec_health: SetHealth = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_health, health);

        let cmd = ClientCommand {
            action_id: VarInt(0),
        };
        buf.clear();
        encode_packet(&cmd, &mut buf).unwrap();
        let dec_cmd: ClientCommand = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_cmd, cmd);

        let respawn = Respawn {
            dimension_type: VarInt(0),
            dimension_name: "minecraft:overworld".into(),
            hashed_seed: 42,
            game_mode: 0,
            previous_game_mode: -1,
            is_debug: false,
            is_flat: true,
            death_location: Some(("minecraft:overworld".into(), BlockPos::new(1, 64, -2))),
            portal_cooldown: VarInt(0),
            sea_level: VarInt(63),
            data_kept: 0,
        };
        buf.clear();
        encode_packet(&respawn, &mut buf).unwrap();
        let dec_respawn: Respawn = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_respawn, respawn);

        let hurt = HurtAnimation {
            entity_id: VarInt(7),
            yaw: 90.0,
        };
        buf.clear();
        encode_packet(&hurt, &mut buf).unwrap();
        let dec_hurt: HurtAnimation = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_hurt, hurt);

        let motion = SetEntityMotion {
            entity_id: VarInt(7),
            velocity: LpVec3::new(0.15, 0.35, -0.1),
        };
        buf.clear();
        encode_packet(&motion, &mut buf).unwrap();
        let dec_motion: SetEntityMotion = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_motion.entity_id, motion.entity_id);
        assert!((dec_motion.velocity.x - motion.velocity.x).abs() < 1e-4);
        assert!((dec_motion.velocity.y - motion.velocity.y).abs() < 1e-4);
        assert!((dec_motion.velocity.z - motion.velocity.z).abs() < 1e-4);

        let interact = Interact {
            entity_id: VarInt(99),
            hand: VarInt(0),
            location: LpVec3::ZERO,
            sneaking: true,
        };
        buf.clear();
        encode_packet(&interact, &mut buf).unwrap();
        let dec_interact: Interact = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_interact, interact);

        let status = MovePlayerStatusOnly { flags: 0x01 };
        assert!(status.on_ground());
        buf.clear();
        encode_packet(&status, &mut buf).unwrap();
        let dec_status: MovePlayerStatusOnly = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_status, status);
    }

    #[test]
    fn simple_slot_and_container_rejects_unsupported_components_and_negative_counts() {
        let mut buf = BytesMut::new();
        VarInt(1).encode(&mut buf).unwrap(); // count
        VarInt(54).encode(&mut buf).unwrap(); // item_id
        VarInt(1).encode(&mut buf).unwrap(); // 1 added component (unsupported)
        VarInt(0).encode(&mut buf).unwrap();
        assert!(SimpleSlot::decode_slot(&mut buf).is_err());

        buf.clear();
        VarInt(0).encode(&mut buf).unwrap(); // window_id
        VarInt(1).encode(&mut buf).unwrap(); // state_id
        VarInt(-1).encode(&mut buf).unwrap(); // negative slot count
        let bad = crate::codec::RawPacket {
            id: ContainerSetContent::ID,
            payload: buf.clone().freeze(),
        };
        assert!(bad.decode_as::<ContainerSetContent>().is_err());

        // ContainerClick, ContainerClose, OpenScreen, ContainerSetSlot roundtrip
        let click = ContainerClick {
            window_id: VarInt(1),
            state_id: VarInt(3),
            slot: 36,
            button: 0,
            mode: VarInt(0),
            changed_slots: vec![(
                36,
                SimpleSlot {
                    count: 4,
                    item_id: 25,
                },
            )],
            carried_item: SimpleSlot::default(),
        };
        buf.clear();
        encode_packet(&click, &mut buf).unwrap();
        let dec_click: ContainerClick = decode_frame(&mut buf, 1024)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_click, click);

        let close = ContainerClose {
            window_id: VarInt(1),
        };
        buf.clear();
        encode_packet(&close, &mut buf).unwrap();
        let dec_close: ContainerClose = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_close, close);

        let open = OpenScreen {
            window_id: VarInt(1),
            window_type: VarInt(2),
            title: crate::nbt::NbtTag::String("Chest".into()),
        };
        buf.clear();
        encode_packet(&open, &mut buf).unwrap();
        let dec_open: OpenScreen = decode_frame(&mut buf, 512)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_open, open);

        let set_slot = ContainerSetSlot {
            window_id: VarInt(0),
            state_id: VarInt(2),
            slot: 36,
            item: SimpleSlot {
                count: 1,
                item_id: 204,
            },
        };
        buf.clear();
        encode_packet(&set_slot, &mut buf).unwrap();
        let dec_slot: ContainerSetSlot = decode_frame(&mut buf, 256)
            .unwrap()
            .unwrap()
            .decode_as()
            .unwrap();
        assert_eq!(dec_slot, set_slot);
    }
}
