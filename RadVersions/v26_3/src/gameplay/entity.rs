//! Entity & basic mob AI simulation with chunk-bucket spatial indexing for `RadServer`.

use std::collections::HashMap;

use rad_core::packet::play::{ItemStack, MetadataEntry, MetadataValue, SetEntityData, SpawnEntity};
use rad_core::types::{LpVec3, VarInt};
use uuid::Uuid;

use crate::registries::EntityIds;
use crate::world::WorldState;
use crate::world::physics::{Aabb, WorldAabbExt};
use crate::world::sim::SimulationArea;
use crate::world::worldgen::DimensionId;

/// Supported mob archetypes in `RadServer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    /// Hostile melee undead mob.
    Zombie,
    /// Passive farm mob.
    Pig,
    /// Hostile ranged/melee skeleton mob.
    Skeleton,
    /// Hostile creeper mob.
    Creeper,
    /// Hostile spider mob.
    Spider,
    /// Neutral enderman mob.
    Enderman,
    /// Passive cow mob.
    Cow,
    /// Passive sheep mob.
    Sheep,
    /// Passive chicken mob.
    Chicken,
    /// Passive villager mob.
    Villager,
    /// Neutral iron golem mob.
    IronGolem,
}

impl MobKind {
    /// Parses a mob name from `/summon`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::from_resource_name(name)
    }

    /// Minecraft 26.3 entity type registry ID from `entities`.
    #[must_use]
    pub const fn entity_type_id_with(self, entities: &EntityIds) -> i32 {
        match self {
            Self::Zombie => entities.zombie,
            Self::Pig => entities.pig,
            Self::Skeleton => entities.skeleton,
            Self::Creeper => entities.creeper,
            Self::Spider => entities.spider,
            Self::Enderman => entities.enderman,
            Self::Cow => entities.cow,
            Self::Sheep => entities.sheep,
            Self::Chicken => entities.chicken,
            Self::Villager => entities.villager,
            Self::IronGolem => entities.iron_golem,
        }
    }

    /// Bounding box `(half_width, height)`.
    #[must_use]
    pub const fn dimensions(self) -> (f64, f64) {
        match self {
            Self::Zombie | Self::Skeleton | Self::Villager => (0.3, 1.95),
            Self::Creeper => (0.3, 1.7),
            Self::Spider => (0.7, 0.9),
            Self::Enderman => (0.3, 2.9),
            Self::Pig => (0.45, 0.9),
            Self::Cow => (0.45, 1.4),
            Self::Sheep => (0.45, 1.3),
            Self::Chicken => (0.2, 0.7),
            Self::IronGolem => (0.7, 2.7),
        }
    }

    /// Maximum health points.
    #[must_use]
    pub const fn max_hp(self) -> f32 {
        match self {
            Self::Zombie | Self::Skeleton | Self::Creeper | Self::Villager => 20.0,
            Self::Spider => 16.0,
            Self::Enderman => 40.0,
            Self::Pig | Self::Cow => 10.0,
            Self::Sheep => 8.0,
            Self::Chicken => 4.0,
            Self::IronGolem => 100.0,
        }
    }

    /// Movement speed in blocks per tick.
    #[must_use]
    pub const fn speed_per_tick(self) -> f64 {
        match self {
            Self::Zombie | Self::Creeper => 0.11,
            Self::Skeleton => 0.12,
            Self::Spider => 0.13,
            Self::Enderman => 0.14,
            Self::Villager | Self::IronGolem => 0.09,
            Self::Pig | Self::Cow | Self::Sheep | Self::Chicken => 0.08,
        }
    }

    /// Whether this mob actively targets survival players.
    #[must_use]
    pub const fn is_hostile(self) -> bool {
        matches!(
            self,
            Self::Zombie | Self::Skeleton | Self::Creeper | Self::Spider
        )
    }

    /// Vanilla namespaced entity ID.
    #[must_use]
    pub const fn resource_name(self) -> &'static str {
        match self {
            Self::Zombie => "minecraft:zombie",
            Self::Pig => "minecraft:pig",
            Self::Skeleton => "minecraft:skeleton",
            Self::Creeper => "minecraft:creeper",
            Self::Spider => "minecraft:spider",
            Self::Enderman => "minecraft:enderman",
            Self::Cow => "minecraft:cow",
            Self::Sheep => "minecraft:sheep",
            Self::Chicken => "minecraft:chicken",
            Self::Villager => "minecraft:villager",
            Self::IronGolem => "minecraft:iron_golem",
        }
    }

    /// Parses a Vanilla namespaced entity ID into [`MobKind`].
    #[must_use]
    pub fn from_resource_name(name: &str) -> Option<Self> {
        let trimmed = name.trim().to_ascii_lowercase();
        match trimmed.strip_prefix("minecraft:").unwrap_or(&trimmed) {
            "zombie" => Some(Self::Zombie),
            "pig" => Some(Self::Pig),
            "skeleton" => Some(Self::Skeleton),
            "creeper" => Some(Self::Creeper),
            "spider" => Some(Self::Spider),
            "enderman" => Some(Self::Enderman),
            "cow" => Some(Self::Cow),
            "sheep" => Some(Self::Sheep),
            "chicken" => Some(Self::Chicken),
            "villager" => Some(Self::Villager),
            "iron_golem" => Some(Self::IronGolem),
            _ => None,
        }
    }
}

/// Active mob instance in the world.
#[derive(Debug, Clone)]
pub struct MobEntity {
    /// Server-assigned entity ID.
    pub entity_id: i32,
    /// Unique entity UUID.
    pub uuid: Uuid,
    /// Mob archetype.
    pub kind: MobKind,
    /// Dimension containing this mob.
    pub dimension: DimensionId,
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Facing yaw in degrees.
    pub yaw: f32,
    /// Facing pitch in degrees.
    pub pitch: f32,
    /// Velocity X (blocks/tick).
    pub vx: f64,
    /// Velocity Y (blocks/tick).
    pub vy: f64,
    /// Velocity Z (blocks/tick).
    pub vz: f64,
    /// Current health points.
    pub hp: f32,
    /// Ticks remaining before next melee attack.
    pub attack_cooldown: u8,
    /// Internal wander direction timer.
    pub wander_ticks: u16,
    /// Current wander yaw in radians.
    pub wander_angle: f64,
    /// Remaining panic-flee ticks after taking damage (for passive mobs).
    pub flee_ticks: u16,
}

/// Supported projectile archetypes.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    /// Fired arrow (`entity_type = 4`, base damage `6.0`).
    Arrow,
    /// Thrown snowball (`entity_type = 123`, base damage `1.0`).
    Snowball,
}

#[cfg_attr(not(test), allow(dead_code))]
impl ProjectileKind {
    /// Minecraft 26.3 entity type registry ID from `entities`.
    #[must_use]
    pub const fn entity_type_id_with(self, entities: &EntityIds) -> i32 {
        match self {
            Self::Arrow => entities.arrow,
            Self::Snowball => entities.snowball,
        }
    }

    /// Damage dealt on direct hit.
    #[must_use]
    pub const fn damage(self) -> f32 {
        match self {
            Self::Arrow => 6.0,
            Self::Snowball => 1.0,
        }
    }
}

/// Active projectile entity in flight.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct ProjectileEntity {
    /// Server-assigned entity ID.
    pub entity_id: i32,
    /// Unique entity UUID.
    pub uuid: Uuid,
    /// Projectile archetype.
    pub kind: ProjectileKind,
    /// Shooter entity ID (excluded from self-hit).
    pub shooter_entity_id: i32,
    /// Dimension containing this projectile.
    pub dimension: DimensionId,
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Velocity X (blocks/tick).
    pub vx: f64,
    /// Velocity Y (blocks/tick).
    pub vy: f64,
    /// Velocity Z (blocks/tick).
    pub vz: f64,
    /// Ticks alive.
    pub age_ticks: u16,
}

#[cfg_attr(not(test), allow(dead_code))]
impl ProjectileEntity {
    /// Builds a [`SpawnEntity`] packet for this projectile using `entities`.
    #[must_use]
    pub fn to_spawn_packet_with(&self, entities: &EntityIds) -> SpawnEntity {
        SpawnEntity {
            entity_id: VarInt(self.entity_id),
            uuid: self.uuid,
            entity_type: VarInt(self.kind.entity_type_id_with(entities)),
            x: self.x,
            y: self.y,
            z: self.z,
            velocity: rad_core::types::LpVec3::new(self.vx, self.vy, self.vz),
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: VarInt(self.shooter_entity_id),
        }
    }
}

impl MobEntity {
    /// Builds a [`SpawnEntity`] packet for this mob using `entities`.
    #[must_use]
    pub fn to_spawn_packet_with(&self, entities: &EntityIds) -> SpawnEntity {
        SpawnEntity {
            entity_id: VarInt(self.entity_id),
            uuid: self.uuid,
            entity_type: VarInt(self.kind.entity_type_id_with(entities)),
            x: self.x,
            y: self.y,
            z: self.z,
            velocity: rad_core::types::LpVec3::new(self.vx, self.vy, self.vz),
            pitch: degrees_to_angle_byte(self.pitch),
            yaw: degrees_to_angle_byte(self.yaw),
            head_yaw: degrees_to_angle_byte(self.yaw),
            data: VarInt(0),
        }
    }

    fn chunk_coords(&self) -> (DimensionId, i32, i32) {
        (
            self.dimension,
            (self.x.floor() as i32).div_euclid(16),
            (self.z.floor() as i32).div_euclid(16),
        )
    }
}

/// Outcome of damaging a mob.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobDamageOutcome {
    /// Remaining HP after damage.
    pub remaining_hp: f32,
    /// Whether the mob died from this hit.
    pub killed: bool,
    /// Knockback velocity X in `1/8000` units.
    pub knockback_vx: i16,
    /// Knockback velocity Y in `1/8000` units.
    pub knockback_vy: i16,
    /// Knockback velocity Z in `1/8000` units.
    pub knockback_vz: i16,
}

/// Events emitted by [`EntityManager::tick`] for network broadcast.
#[derive(Debug, Clone, PartialEq)]
pub enum MobTickEvent {
    /// Mob moved and rotated.
    Moved {
        /// Dimension of the mob.
        dimension: DimensionId,
        /// Mob entity ID.
        entity_id: i32,
        /// Delta X in `1/4096` block units.
        dx: i16,
        /// Delta Y in `1/4096` block units.
        dy: i16,
        /// Delta Z in `1/4096` block units.
        dz: i16,
        /// Quantized yaw byte.
        yaw: i8,
        /// Quantized pitch byte.
        pitch: i8,
        /// Whether mob is on ground.
        on_ground: bool,
    },
    /// Hostile mob or projectile attacked a survival player.
    AttackedPlayer {
        /// Dimension of the attack.
        dimension: DimensionId,
        /// Target player entity ID.
        player_entity_id: i32,
        /// Damage dealt in half-hearts.
        damage: f32,
        /// Knockback X in blocks/tick.
        knockback_x: f64,
        /// Knockback Z in blocks/tick.
        knockback_z: f64,
        /// Attack yaw for hurt animation.
        yaw: f32,
    },
    /// Projectile moved along its trajectory.
    ProjectileMoved {
        /// Dimension of the projectile.
        dimension: DimensionId,
        /// Projectile entity ID.
        entity_id: i32,
        /// Delta X in `1/4096` block units.
        dx: i16,
        /// Delta Y in `1/4096` block units.
        dy: i16,
        /// Delta Z in `1/4096` block units.
        dz: i16,
    },
    /// Projectile impacted or expired and was removed.
    ProjectileDespawned {
        /// Dimension of the projectile.
        dimension: DimensionId,
        /// Projectile entity ID.
        entity_id: i32,
    },
    /// Mob took damage (e.g. from a projectile).
    MobDamaged {
        /// Dimension of the mob.
        dimension: DimensionId,
        /// Mob entity ID.
        entity_id: i32,
        /// Knockback velocity X in `1/8000` units.
        knockback_vx: i16,
        /// Knockback velocity Y in `1/8000` units.
        knockback_vy: i16,
        /// Knockback velocity Z in `1/8000` units.
        knockback_vz: i16,
    },
    /// Mob died and was removed.
    MobKilled {
        /// Dimension of the mob.
        dimension: DimensionId,
        /// Mob entity ID.
        entity_id: i32,
    },
    /// Dropped item moved during physics simulation.
    ItemMoved {
        /// Dimension of the dropped item.
        dimension: DimensionId,
        /// Dropped item entity ID.
        entity_id: i32,
        /// Absolute X coordinate after movement.
        x: f64,
        /// Absolute Y coordinate after movement.
        y: f64,
        /// Absolute Z coordinate after movement.
        z: f64,
        /// Delta X in `1/4096` blocks.
        dx: i16,
        /// Delta Y in `1/4096` blocks.
        dy: i16,
        /// Delta Z in `1/4096` blocks.
        dz: i16,
        /// Whether the item is resting on ground.
        on_ground: bool,
    },
    /// Two compatible dropped item stacks merged into one entity.
    ItemMerged {
        /// Dimension of both items.
        dimension: DimensionId,
        /// Surviving entity ID whose stack count increased.
        kept_entity_id: i32,
        /// Removed entity ID that was absorbed.
        removed_entity_id: i32,
        /// Updated stack of the surviving entity.
        stack: ItemStack,
    },
    /// Dropped item expired (`age_ticks >= 6000`) and despawned.
    ItemDespawned {
        /// Dimension of the dropped item.
        dimension: DimensionId,
        /// Despawned entity ID.
        entity_id: i32,
    },
}

/// Active dropped item entity (`minecraft:item`, type ID `72`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DroppedItemEntity {
    /// Server-assigned entity ID.
    pub entity_id: i32,
    /// Unique entity UUID.
    pub uuid: Uuid,
    /// Dimension containing this dropped item.
    pub dimension: DimensionId,
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Velocity X in blocks/tick.
    pub vx: f64,
    /// Velocity Y in blocks/tick.
    pub vy: f64,
    /// Velocity Z in blocks/tick.
    pub vz: f64,
    /// Item stack (ID, count, and component patches).
    pub stack: ItemStack,
    /// Remaining ticks before any player can pick up this item (`40` on player throw, `10` on block/mob drop).
    pub pickup_delay_ticks: u16,
    /// Ticks lived in loaded simulation chunks (despawns at `6000`).
    pub age_ticks: u16,
    /// Whether this item is currently resting on a solid surface.
    pub on_ground: bool,
}

impl DroppedItemEntity {
    /// Chunk coordinates `(cx, cz)` containing this item.
    #[must_use]
    pub fn chunk_coords(&self) -> (DimensionId, i32, i32) {
        (
            self.dimension,
            (self.x.floor() as i32).div_euclid(16),
            (self.z.floor() as i32).div_euclid(16),
        )
    }

    /// Builds a clientbound `SpawnEntity` packet for `minecraft:item` using `entities`.
    #[must_use]
    pub fn to_spawn_packet_with(&self, entities: &EntityIds) -> SpawnEntity {
        SpawnEntity {
            entity_id: VarInt(self.entity_id),
            uuid: self.uuid,
            entity_type: VarInt(entities.item),
            x: self.x,
            y: self.y,
            z: self.z,
            velocity: LpVec3::new(self.vx, self.vy, self.vz),
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: VarInt(0),
        }
    }

    /// Builds a clientbound `SetEntityData` packet carrying `ItemEntity.DATA_ITEM` (index `8`).
    #[must_use]
    pub fn to_metadata_packet(&self) -> SetEntityData {
        SetEntityData {
            entity_id: VarInt(self.entity_id),
            entries: vec![MetadataEntry {
                index: 8,
                value: MetadataValue::ItemStack(self.stack.clone()),
            }],
        }
    }
}

/// Spatial-indexed mob, projectile, and dropped item manager.
#[derive(Debug, Default)]
pub struct EntityManager {
    mobs: HashMap<i32, MobEntity>,
    chunk_buckets: HashMap<(DimensionId, i32, i32), Vec<i32>>,
    projectiles: HashMap<i32, ProjectileEntity>,
    dropped_items: HashMap<i32, DroppedItemEntity>,
}

impl EntityManager {
    /// Creates an empty [`EntityManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of active mobs.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn len(&self) -> usize {
        self.mobs.len()
    }

    /// Returns `true` if there are no active mobs.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mobs.is_empty()
    }

    /// Spawns a mob with the given `entity_id` at `(x, y, z)` in `dimension`.
    pub fn spawn_mob_in(
        &mut self,
        dimension: DimensionId,
        entity_id: i32,
        kind: MobKind,
        x: f64,
        y: f64,
        z: f64,
    ) -> &MobEntity {
        let mob = MobEntity {
            entity_id,
            uuid: Uuid::new_v3(&Uuid::NAMESPACE_OID, &entity_id.to_be_bytes()),
            kind,
            dimension,
            x,
            y,
            z,
            yaw: 0.0,
            pitch: 0.0,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            hp: kind.max_hp(),
            attack_cooldown: 0,
            wander_ticks: 0,
            wander_angle: 0.0,
            flee_ticks: 0,
        };
        let chunk = mob.chunk_coords();
        self.chunk_buckets.entry(chunk).or_default().push(entity_id);
        self.mobs.insert(entity_id, mob);
        &self.mobs[&entity_id]
    }

    /// Spawns a mob in Overworld (convenience wrapper).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn spawn_mob(
        &mut self,
        entity_id: i32,
        kind: MobKind,
        x: f64,
        y: f64,
        z: f64,
    ) -> &MobEntity {
        self.spawn_mob_in(DimensionId::Overworld, entity_id, kind, x, y, z)
    }

    /// Spawns a projectile with `entity_id` in `dimension` at `pos` and velocity `vel`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn spawn_projectile_in(
        &mut self,
        dimension: DimensionId,
        entity_id: i32,
        kind: ProjectileKind,
        shooter_entity_id: i32,
        pos: [f64; 3],
        vel: [f64; 3],
    ) -> &ProjectileEntity {
        let proj = ProjectileEntity {
            entity_id,
            uuid: Uuid::new_v3(&Uuid::NAMESPACE_OID, &entity_id.to_be_bytes()),
            kind,
            shooter_entity_id,
            dimension,
            x: pos[0],
            y: pos[1],
            z: pos[2],
            vx: vel[0],
            vy: vel[1],
            vz: vel[2],
            age_ticks: 0,
        };
        self.projectiles.insert(entity_id, proj);
        &self.projectiles[&entity_id]
    }

    /// Spawns a projectile in Overworld (convenience wrapper).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn spawn_projectile(
        &mut self,
        entity_id: i32,
        kind: ProjectileKind,
        shooter_entity_id: i32,
        pos: [f64; 3],
        vel: [f64; 3],
    ) -> &ProjectileEntity {
        self.spawn_projectile_in(
            DimensionId::Overworld,
            entity_id,
            kind,
            shooter_entity_id,
            pos,
            vel,
        )
    }

    /// Number of active projectiles.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn projectile_count(&self) -> usize {
        self.projectiles.len()
    }

    /// Spawns a dropped item entity in `dimension` at `pos` with velocity `vel`.
    /// Returns `None` if `stack` is empty or invalid.
    pub fn spawn_dropped_item_in(
        &mut self,
        dimension: DimensionId,
        entity_id: i32,
        pos: [f64; 3],
        vel: [f64; 3],
        stack: ItemStack,
        pickup_delay_ticks: u16,
    ) -> Option<&DroppedItemEntity> {
        if stack.item_id <= 0 || stack.count <= 0 {
            return None;
        }
        let item = DroppedItemEntity {
            entity_id,
            uuid: Uuid::new_v3(&Uuid::NAMESPACE_OID, &entity_id.to_be_bytes()),
            dimension,
            x: pos[0],
            y: pos[1],
            z: pos[2],
            vx: vel[0],
            vy: vel[1],
            vz: vel[2],
            stack,
            pickup_delay_ticks,
            age_ticks: 0,
            on_ground: false,
        };
        self.dropped_items.insert(entity_id, item);
        self.dropped_items.get(&entity_id)
    }

    /// Inserts a pre-built/restored [`DroppedItemEntity`] (used on world load).
    pub fn insert_dropped_item(&mut self, item: DroppedItemEntity) {
        if item.stack.item_id > 0 && item.stack.count > 0 {
            self.dropped_items.insert(item.entity_id, item);
        }
    }

    /// Inserts a pre-built/restored [`MobEntity`] (used on world load).
    pub fn insert_mob(&mut self, mob: MobEntity) {
        if mob.hp > 0.0 {
            let chunk = (
                mob.dimension,
                (mob.x.floor() as i32).div_euclid(16),
                (mob.z.floor() as i32).div_euclid(16),
            );
            self.chunk_buckets
                .entry(chunk)
                .or_default()
                .push(mob.entity_id);
            self.mobs.insert(mob.entity_id, mob);
        }
    }

    /// Number of active dropped item entities.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn dropped_item_count(&self) -> usize {
        self.dropped_items.len()
    }

    /// Iterates over all active dropped items.
    pub fn all_dropped_items(&self) -> impl Iterator<Item = &DroppedItemEntity> {
        self.dropped_items.values()
    }

    /// Iterates over active dropped items in `dimension`.
    pub fn dropped_items_in_dimension(
        &self,
        dimension: DimensionId,
    ) -> impl Iterator<Item = &DroppedItemEntity> {
        self.dropped_items
            .values()
            .filter(move |item| item.dimension == dimension)
    }

    /// Mutable reference to a dropped item by entity ID.
    pub fn get_dropped_item_mut(&mut self, entity_id: i32) -> Option<&mut DroppedItemEntity> {
        self.dropped_items.get_mut(&entity_id)
    }

    /// Removes and returns a dropped item by entity ID.
    pub fn remove_dropped_item(&mut self, entity_id: i32) -> Option<DroppedItemEntity> {
        self.dropped_items.remove(&entity_id)
    }

    /// Returns all active mobs.
    pub fn all_mobs(&self) -> impl Iterator<Item = &MobEntity> {
        self.mobs.values()
    }

    /// Returns all active mobs in `dimension`.
    pub fn mobs_in_dimension(&self, dimension: DimensionId) -> impl Iterator<Item = &MobEntity> {
        self.mobs.values().filter(move |m| m.dimension == dimension)
    }

    /// Returns mob IDs currently tracked inside `(cx, cz)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn mobs_in_chunk(&self, cx: i32, cz: i32) -> &[i32] {
        self.chunk_buckets
            .get(&(DimensionId::Overworld, cx, cz))
            .map_or(&[], Vec::as_slice)
    }

    /// Looks up a mob by `entity_id`.
    #[must_use]
    pub fn get(&self, entity_id: i32) -> Option<&MobEntity> {
        self.mobs.get(&entity_id)
    }

    /// Applies melee damage and knockback to a mob, removing it if `hp <= 0`.
    pub fn damage_mob(
        &mut self,
        entity_id: i32,
        amount: f32,
        dir_x: f64,
        dir_z: f64,
    ) -> Option<MobDamageOutcome> {
        let mob = self.mobs.get_mut(&entity_id)?;
        mob.hp = (mob.hp - amount).max(0.0);

        let len = (dir_x * dir_x + dir_z * dir_z).sqrt().max(0.001);
        let nx = dir_x / len;
        let nz = dir_z / len;
        mob.vx += nx * 0.35;
        mob.vy = 0.32;
        mob.vz += nz * 0.35;
        if !mob.kind.is_hostile() {
            mob.flee_ticks = 60;
            mob.wander_angle = nz.atan2(nx);
        }

        let outcome = MobDamageOutcome {
            remaining_hp: mob.hp,
            killed: mob.hp <= 0.0,
            knockback_vx: (mob.vx * 8000.0).clamp(-32000.0, 32000.0) as i16,
            knockback_vy: (mob.vy * 8000.0).clamp(-32000.0, 32000.0) as i16,
            knockback_vz: (mob.vz * 8000.0).clamp(-32000.0, 32000.0) as i16,
        };

        if outcome.killed {
            let chunk = mob.chunk_coords();
            self.mobs.remove(&entity_id);
            if let Some(bucket) = self.chunk_buckets.get_mut(&chunk) {
                bucket.retain(|&id| id != entity_id);
                if bucket.is_empty() {
                    self.chunk_buckets.remove(&chunk);
                }
            }
        }

        Some(outcome)
    }

    /// Advances mob AI, gravity, voxel collision, and melee attacks by one tick (`50 ms`)
    /// (treating all chunks as active).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn tick(
        &mut self,
        world: &mut WorldState,
        players: &[(i32, DimensionId, f64, f64, f64, bool)],
    ) -> Vec<MobTickEvent> {
        self.tick_in_area(world, players, &SimulationArea::all_active())
    }

    /// Advances mob AI, gravity, voxel collision, and projectiles within `area` by one tick (`50 ms`),
    /// pausing mobs and projectiles outside `area` without loading inactive chunks.
    ///
    /// `players` contains `(entity_id, dimension, x, y, z, can_be_targeted)`.
    pub fn tick_in_area(
        &mut self,
        world: &mut WorldState,
        players: &[(i32, DimensionId, f64, f64, f64, bool)],
        area: &SimulationArea,
    ) -> Vec<MobTickEvent> {
        type ChunkMove = (i32, (DimensionId, i32, i32), (DimensionId, i32, i32));
        let mut events = Vec::new();
        let mut chunk_moves: Vec<ChunkMove> = Vec::new();

        for mob in self.mobs.values_mut() {
            if !area.contains_pos(mob.dimension, mob.x, mob.z) {
                continue;
            }
            if mob.attack_cooldown > 0 {
                mob.attack_cooldown -= 1;
            }

            let old_x = mob.x;
            let old_y = mob.y;
            let old_z = mob.z;
            let old_chunk = mob.chunk_coords();

            // AI steering (only target players in the same dimension)
            let mut target_player: Option<(i32, f64, f64, f64, f64)> = None;
            if mob.kind.is_hostile() {
                let mut best_dist_sq = 16.0 * 16.0;
                for &(pid, pdim, px, py, pz, targetable) in players {
                    if !targetable || pdim != mob.dimension {
                        continue;
                    }
                    let dx = px - mob.x;
                    let dy = py - mob.y;
                    let dz = pz - mob.z;
                    let d2 = dx * dx + dy * dy + dz * dz;
                    if d2 < best_dist_sq {
                        best_dist_sq = d2;
                        target_player = Some((pid, px, py, pz, d2));
                    }
                }
            }

            let speed = mob.kind.speed_per_tick();
            if let Some((pid, px, _py, pz, dist_sq)) = target_player {
                let dx = px - mob.x;
                let dz = pz - mob.z;
                let dist = dist_sq.sqrt().max(0.001);
                mob.yaw = (-dx).atan2(dz).to_degrees() as f32;
                if dist > 1.3 {
                    mob.vx = mob.vx * 0.5 + (dx / dist) * speed * 0.5;
                    mob.vz = mob.vz * 0.5 + (dz / dist) * speed * 0.5;
                }
                if dist <= 1.8 && mob.attack_cooldown == 0 {
                    mob.attack_cooldown = 20;
                    events.push(MobTickEvent::AttackedPlayer {
                        dimension: mob.dimension,
                        player_entity_id: pid,
                        damage: 3.0,
                        knockback_x: (dx / dist) * 0.35,
                        knockback_z: (dz / dist) * 0.35,
                        yaw: mob.yaw,
                    });
                }
            } else if mob.flee_ticks > 0 {
                mob.flee_ticks -= 1;
                let dx = mob.wander_angle.cos();
                let dz = mob.wander_angle.sin();
                mob.vx = mob.vx * 0.5 + dx * speed * 1.6 * 0.5;
                mob.vz = mob.vz * 0.5 + dz * speed * 1.6 * 0.5;
                mob.yaw = (-dx).atan2(dz).to_degrees() as f32;
            } else {
                // Gentle passive wander
                if mob.wander_ticks == 0 {
                    mob.wander_ticks = 60;
                    mob.wander_angle = (mob.wander_angle + 1.1) % std::f64::consts::TAU;
                } else {
                    mob.wander_ticks -= 1;
                }
                if mob.wander_ticks > 35 {
                    let dx = mob.wander_angle.cos();
                    let dz = mob.wander_angle.sin();
                    mob.vx = mob.vx * 0.6 + dx * speed * 0.4;
                    mob.vz = mob.vz * 0.6 + dz * speed * 0.4;
                    mob.yaw = (-dx).atan2(dz).to_degrees() as f32;
                } else {
                    mob.vx *= 0.6;
                    mob.vz *= 0.6;
                }
            }

            // Gravity + axis-separated AABB resolution
            mob.vy = (mob.vy - 0.08).max(-1.5);
            let (hw, h) = mob.kind.dimensions();
            let regs = std::sync::Arc::clone(world.registries());
            let provider = regs.provider();

            if mob.vx.abs() > 1e-4 {
                let nx = mob.x + mob.vx;
                if !Aabb::from_entity(nx, mob.y, mob.z, hw, h).intersects_solid_in_with(
                    world,
                    mob.dimension,
                    provider.as_ref(),
                ) {
                    mob.x = nx;
                } else {
                    mob.vx = 0.0;
                }
            }
            if mob.vz.abs() > 1e-4 {
                let nz = mob.z + mob.vz;
                if !Aabb::from_entity(mob.x, mob.y, nz, hw, h).intersects_solid_in_with(
                    world,
                    mob.dimension,
                    provider.as_ref(),
                ) {
                    mob.z = nz;
                } else {
                    mob.vz = 0.0;
                }
            }

            let mut on_ground = false;
            let ny = mob.y + mob.vy;
            if !Aabb::from_entity(mob.x, ny, mob.z, hw, h).intersects_solid_in_with(
                world,
                mob.dimension,
                provider.as_ref(),
            ) {
                mob.y = ny;
            } else {
                if mob.vy < 0.0 {
                    on_ground = true;
                }
                mob.vy = 0.0;
            }

            let new_chunk = mob.chunk_coords();
            if new_chunk != old_chunk {
                chunk_moves.push((mob.entity_id, old_chunk, new_chunk));
            }

            let dx = ((mob.x - old_x) * 4096.0).round() as i16;
            let dy = ((mob.y - old_y) * 4096.0).round() as i16;
            let dz = ((mob.z - old_z) * 4096.0).round() as i16;
            if dx != 0 || dy != 0 || dz != 0 {
                events.push(MobTickEvent::Moved {
                    dimension: mob.dimension,
                    entity_id: mob.entity_id,
                    dx,
                    dy,
                    dz,
                    yaw: degrees_to_angle_byte(mob.yaw),
                    pitch: degrees_to_angle_byte(mob.pitch),
                    on_ground,
                });
            }
        }

        for (eid, old_c, new_c) in chunk_moves {
            if let Some(bucket) = self.chunk_buckets.get_mut(&old_c) {
                bucket.retain(|&id| id != eid);
                if bucket.is_empty() {
                    self.chunk_buckets.remove(&old_c);
                }
            }
            self.chunk_buckets.entry(new_c).or_default().push(eid);
        }

        // Advance active projectiles and resolve block/mob/player impacts.
        let proj_ids: Vec<i32> = self.projectiles.keys().copied().collect();
        for pid in proj_ids {
            if let Some(proj) = self.projectiles.get(&pid)
                && !area.contains_pos(proj.dimension, proj.x, proj.z)
            {
                continue;
            }
            let Some(mut proj) = self.projectiles.remove(&pid) else {
                continue;
            };
            proj.age_ticks = proj.age_ticks.saturating_add(1);
            if proj.age_ticks > 100 {
                events.push(MobTickEvent::ProjectileDespawned {
                    dimension: proj.dimension,
                    entity_id: pid,
                });
                continue;
            }
            proj.vy = (proj.vy - 0.05).max(-2.0);
            proj.vx *= 0.99;
            proj.vz *= 0.99;
            let nx = proj.x + proj.vx;
            let ny = proj.y + proj.vy;
            let nz = proj.z + proj.vz;

            let regs = std::sync::Arc::clone(world.registries());
            let provider = regs.provider();
            if Aabb::from_entity(nx, ny, nz, 0.125, 0.25).intersects_solid_in_with(
                world,
                proj.dimension,
                provider.as_ref(),
            ) {
                events.push(MobTickEvent::ProjectileDespawned {
                    dimension: proj.dimension,
                    entity_id: pid,
                });
                continue;
            }

            let hit_mob = self.mobs.iter().find_map(|(&mid, mob)| {
                if mid == proj.shooter_entity_id || mob.dimension != proj.dimension {
                    return None;
                }
                let (hw, h) = mob.kind.dimensions();
                let dx = (mob.x - nx).abs();
                let dz = (mob.z - nz).abs();
                let in_y = ny >= mob.y && ny <= mob.y + h;
                if dx <= hw + 0.25 && dz <= hw + 0.25 && in_y {
                    Some(mid)
                } else {
                    None
                }
            });

            if let Some(mid) = hit_mob {
                if let Some(outcome) = self.damage_mob(mid, proj.kind.damage(), proj.vx, proj.vz) {
                    if outcome.killed {
                        events.push(MobTickEvent::MobKilled {
                            dimension: proj.dimension,
                            entity_id: mid,
                        });
                    } else {
                        events.push(MobTickEvent::MobDamaged {
                            dimension: proj.dimension,
                            entity_id: mid,
                            knockback_vx: outcome.knockback_vx,
                            knockback_vy: outcome.knockback_vy,
                            knockback_vz: outcome.knockback_vz,
                        });
                    }
                }
                events.push(MobTickEvent::ProjectileDespawned {
                    dimension: proj.dimension,
                    entity_id: pid,
                });
                continue;
            }

            let hit_player =
                players
                    .iter()
                    .find_map(|&(target_eid, pdim, px, py, pz, targetable)| {
                        if !targetable
                            || target_eid == proj.shooter_entity_id
                            || pdim != proj.dimension
                        {
                            return None;
                        }
                        let dx = (px - nx).abs();
                        let dz = (pz - nz).abs();
                        if dx <= 0.45 && dz <= 0.45 && ny >= py && ny <= py + 1.8 {
                            Some(target_eid)
                        } else {
                            None
                        }
                    });
            if let Some(target_eid) = hit_player {
                let len = (proj.vx * proj.vx + proj.vz * proj.vz).sqrt().max(0.001);
                let yaw = (-proj.vx).atan2(proj.vz).to_degrees() as f32;
                events.push(MobTickEvent::AttackedPlayer {
                    dimension: proj.dimension,
                    player_entity_id: target_eid,
                    damage: proj.kind.damage(),
                    knockback_x: (proj.vx / len) * 0.35,
                    knockback_z: (proj.vz / len) * 0.35,
                    yaw,
                });
                events.push(MobTickEvent::ProjectileDespawned {
                    dimension: proj.dimension,
                    entity_id: pid,
                });
                continue;
            }

            let dx = ((nx - proj.x) * 4096.0).round() as i16;
            let dy = ((ny - proj.y) * 4096.0).round() as i16;
            let dz = ((nz - proj.z) * 4096.0).round() as i16;
            proj.x = nx;
            proj.y = ny;
            proj.z = nz;
            self.projectiles.insert(pid, proj.clone());
            if dx != 0 || dy != 0 || dz != 0 {
                events.push(MobTickEvent::ProjectileMoved {
                    dimension: proj.dimension,
                    entity_id: pid,
                    dx,
                    dy,
                    dz,
                });
            }
        }

        // Advance dropped item physics, pickup delay, despawn timer, and stack merging.
        let item_ids: Vec<i32> = self.dropped_items.keys().copied().collect();
        let mut expired_items = Vec::new();
        let mut merge_candidates: Vec<i32> = Vec::new();
        for iid in item_ids {
            let Some(item) = self.dropped_items.get_mut(&iid) else {
                continue;
            };
            if !area.contains_pos(item.dimension, item.x, item.z) {
                continue;
            }
            if item.pickup_delay_ticks > 0 && item.pickup_delay_ticks < u16::MAX {
                item.pickup_delay_ticks -= 1;
            }
            item.age_ticks = item.age_ticks.saturating_add(1);
            if item.age_ticks >= 6000 {
                expired_items.push((item.dimension, iid));
                continue;
            }

            let old_x = item.x;
            let old_y = item.y;
            let old_z = item.z;

            let feet_block = world.get_block_in(
                item.dimension,
                item.x.floor() as i32,
                (item.y + 0.05).floor() as i32,
                item.z.floor() as i32,
            );
            let water = &world.registries().blocks.water;
            let in_water = (water.min_state..=water.max_state).contains(&feet_block);

            if in_water {
                item.vy = (item.vy + 0.035).clamp(-0.2, 0.15);
                item.vx *= 0.85;
                item.vz *= 0.85;
            } else {
                item.vy = (item.vy - 0.04).max(-3.92);
            }

            let hw = 0.125;
            let h = 0.25;
            let regs = std::sync::Arc::clone(world.registries());
            let provider = regs.provider();

            let nx = item.x + item.vx;
            if !Aabb::from_entity(nx, item.y, item.z, hw, h).intersects_solid_in_with(
                world,
                item.dimension,
                provider.as_ref(),
            ) {
                item.x = nx;
            } else {
                item.vx = 0.0;
            }

            let nz = item.z + item.vz;
            if !Aabb::from_entity(item.x, item.y, nz, hw, h).intersects_solid_in_with(
                world,
                item.dimension,
                provider.as_ref(),
            ) {
                item.z = nz;
            } else {
                item.vz = 0.0;
            }

            let mut on_ground = false;
            let ny = item.y + item.vy;
            if !Aabb::from_entity(item.x, ny, item.z, hw, h).intersects_solid_in_with(
                world,
                item.dimension,
                provider.as_ref(),
            ) {
                item.y = ny;
            } else {
                if item.vy < 0.0 {
                    on_ground = true;
                }
                item.vy = 0.0;
            }
            item.on_ground = on_ground;

            let drag = if on_ground { 0.6 } else { 0.98 };
            item.vx *= drag;
            item.vz *= drag;
            if item.vx.abs() < 1e-4 {
                item.vx = 0.0;
            }
            if item.vz.abs() < 1e-4 {
                item.vz = 0.0;
            }

            let dx = ((item.x - old_x) * 4096.0).round() as i16;
            let dy = ((item.y - old_y) * 4096.0).round() as i16;
            let dz = ((item.z - old_z) * 4096.0).round() as i16;
            if dx != 0 || dy != 0 || dz != 0 {
                events.push(MobTickEvent::ItemMoved {
                    dimension: item.dimension,
                    entity_id: iid,
                    x: item.x,
                    y: item.y,
                    z: item.z,
                    dx,
                    dy,
                    dz,
                    on_ground,
                });
            }

            if item.age_ticks % 10 == 0 && item.pickup_delay_ticks < u16::MAX {
                merge_candidates.push(iid);
            }
        }

        for (dim, iid) in expired_items {
            self.dropped_items.remove(&iid);
            events.push(MobTickEvent::ItemDespawned {
                dimension: dim,
                entity_id: iid,
            });
        }

        // Merge nearby compatible dropped items (bounded per-tick check on age % 10 == 0).
        merge_candidates.sort_unstable();
        for &keeper_id in &merge_candidates {
            let Some(keeper) = self.dropped_items.get(&keeper_id).cloned() else {
                continue;
            };
            let max_stack = if keeper.stack.components.damage.is_some() {
                1
            } else {
                64
            };
            if keeper.stack.count >= max_stack {
                continue;
            }
            let other_ids: Vec<i32> = self
                .dropped_items
                .iter()
                .filter(|&(&oid, other)| {
                    oid > keeper_id
                        && other.dimension == keeper.dimension
                        && other.pickup_delay_ticks < u16::MAX
                        && other.stack.item_id == keeper.stack.item_id
                        && other.stack.components == keeper.stack.components
                        && other.stack.count < max_stack
                        && (other.x - keeper.x).abs() <= 0.5
                        && (other.y - keeper.y).abs() <= 0.5
                        && (other.z - keeper.z).abs() <= 0.5
                })
                .map(|(&oid, _)| oid)
                .collect();

            for oid in other_ids {
                let Some(keeper_mut) = self.dropped_items.get(&keeper_id) else {
                    break;
                };
                if keeper_mut.stack.count >= max_stack {
                    break;
                }
                let Some(other) = self.dropped_items.get(&oid).cloned() else {
                    continue;
                };
                let space = max_stack - keeper_mut.stack.count;
                if other.stack.count <= space {
                    let keeper_mut = self.dropped_items.get_mut(&keeper_id).unwrap();
                    keeper_mut.stack.count += other.stack.count;
                    keeper_mut.age_ticks = keeper_mut.age_ticks.min(other.age_ticks);
                    let updated_stack = keeper_mut.stack.clone();
                    let dim = keeper_mut.dimension;
                    self.dropped_items.remove(&oid);
                    events.push(MobTickEvent::ItemMerged {
                        dimension: dim,
                        kept_entity_id: keeper_id,
                        removed_entity_id: oid,
                        stack: updated_stack,
                    });
                }
            }
        }

        events
    }
}

/// Converts degrees into Minecraft's signed angle byte (`1/256` turn).
#[must_use]
pub fn degrees_to_angle_byte(deg: f32) -> i8 {
    (((deg.rem_euclid(360.0) / 360.0) * 256.0).round() as u8) as i8
}

/// Loads persisted dropped items from `<world_dir>/dropped_items.json`.
#[must_use]
pub fn load_dropped_items_from_dir(world_dir: &std::path::Path) -> Vec<DroppedItemEntity> {
    let path = world_dir.join("dropped_items.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<DroppedItemEntity>>(&text).unwrap_or_default()
}

/// Atomically writes `items` to `<world_dir>/dropped_items.json` using `.tmp` + rename,
/// or removes the file if `items` is empty.
pub fn write_dropped_items_atomic(
    world_dir: &std::path::Path,
    items: &[DroppedItemEntity],
) -> std::io::Result<()> {
    let path = world_dir.join("dropped_items.json");
    if items.is_empty() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        return Ok(());
    }
    std::fs::create_dir_all(world_dir)?;
    let tmp = world_dir.join("dropped_items.json.tmp");
    let json = serde_json::to_string_pretty(items)
        .map_err(|e| std::io::Error::other(format!("serialize dropped_items: {e}")))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Loads dropped items from Vanilla Anvil `entities/*.mca` across all dimensions,
/// falling back to legacy `<world_dir>/dropped_items.json` if present.
#[cfg_attr(not(test), allow(dead_code))]
#[must_use]
pub fn load_dropped_items_from_world_mca(
    world_dir: &std::path::Path,
    registries: &crate::registries::Version26_3Registries,
) -> Vec<DroppedItemEntity> {
    load_entities_from_world_mca(world_dir, registries).0
}

/// Loads dropped items and mobs from Vanilla Anvil `entities/*.mca` for a single chunk `(dim, cx, cz)`.
#[must_use]
pub fn load_entities_in_chunk_from_mca(
    world_dir: &std::path::Path,
    dim: DimensionId,
    cx: i32,
    cz: i32,
    next_eid: &mut i32,
    registries: &crate::registries::Version26_3Registries,
) -> (Vec<DroppedItemEntity>, Vec<MobEntity>) {
    let mut items = Vec::new();
    let mut mobs = Vec::new();
    let dim_dir = crate::world::dimension_dir_for(world_dir, dim);
    if let Ok(Some(compressed)) =
        crate::world::anvil::AnvilRegion::read_subdir_chunk_payload_checked(
            &dim_dir, "entities", cx, cz,
        )
    {
        decode_entities_chunk_payload(
            &compressed,
            dim,
            registries,
            next_eid,
            &mut items,
            &mut mobs,
        );
    }
    (items, mobs)
}

/// Loads dropped items and mobs from Vanilla Anvil `entities/*.mca` across all dimensions,
/// falling back to legacy `<world_dir>/dropped_items.json` for items if present.
#[must_use]
pub fn load_entities_from_world_mca(
    world_dir: &std::path::Path,
    registries: &crate::registries::Version26_3Registries,
) -> (Vec<DroppedItemEntity>, Vec<MobEntity>) {
    let mut items = Vec::new();
    let mut mobs = Vec::new();
    let mut next_eid = 10_000_i32;

    for dim in [
        DimensionId::Overworld,
        DimensionId::Nether,
        DimensionId::End,
    ] {
        let dim_dir = crate::world::dimension_dir_for(world_dir, dim);
        let entities_dir = dim_dir.join("entities");
        let Ok(entries) = std::fs::read_dir(&entities_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name_str) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name_str.starts_with("r.") || !name_str.ends_with(".mca") {
                continue;
            }
            for compressed in crate::world::anvil::AnvilRegion::read_all_payloads_from_file(&path) {
                decode_entities_chunk_payload(
                    &compressed,
                    dim,
                    registries,
                    &mut next_eid,
                    &mut items,
                    &mut mobs,
                );
            }
        }
    }

    if items.is_empty() {
        items = load_dropped_items_from_dir(world_dir);
    }
    (items, mobs)
}

fn decode_entities_chunk_payload(
    compressed: &[u8],
    dim: DimensionId,
    registries: &crate::registries::Version26_3Registries,
    next_eid: &mut i32,
    items: &mut Vec<DroppedItemEntity>,
    mobs: &mut Vec<MobEntity>,
) {
    use flate2::read::ZlibDecoder;
    use rad_core::nbt::{NbtTag, decode_named_nbt};
    use std::io::Read;

    let mut dec = ZlibDecoder::new(compressed).take(1024 * 1024);
    let mut raw = Vec::new();
    if dec.read_to_end(&mut raw).is_err() {
        return;
    }
    let mut cursor = raw.as_slice();
    let Ok((_, NbtTag::Compound(root))) = decode_named_nbt(&mut cursor) else {
        return;
    };
    let Some(NbtTag::List(entity_list)) = root.get("Entities") else {
        return;
    };
    for ent in entity_list {
        let NbtTag::Compound(map) = ent else {
            continue;
        };
        let Some(NbtTag::String(ent_id)) = map.get("id") else {
            continue;
        };
        let Some(NbtTag::List(pos)) = map.get("Pos") else {
            continue;
        };
        if pos.len() != 3 {
            continue;
        }
        let (NbtTag::Double(x), NbtTag::Double(y), NbtTag::Double(z)) = (&pos[0], &pos[1], &pos[2])
        else {
            continue;
        };
        let (vx, vy, vz) = match map.get("Motion") {
            Some(NbtTag::List(m)) if m.len() == 3 => match (&m[0], &m[1], &m[2]) {
                (NbtTag::Double(vx), NbtTag::Double(vy), NbtTag::Double(vz)) => (*vx, *vy, *vz),
                _ => (0.0, 0.0, 0.0),
            },
            _ => (0.0, 0.0, 0.0),
        };

        if ent_id == "minecraft:item" {
            let pickup_delay_ticks = match map.get("PickupDelay") {
                Some(NbtTag::Short(v)) => (*v).max(0) as u16,
                _ => 0,
            };
            let age_ticks = match map.get("Age") {
                Some(NbtTag::Short(v)) => (*v).max(0) as u16,
                _ => 0,
            };
            let Some(NbtTag::Compound(item_map)) = map.get("Item") else {
                continue;
            };
            let Some(NbtTag::String(item_name)) = item_map.get("id") else {
                continue;
            };
            let Some(item_id) = registries.item_by_name(item_name) else {
                continue;
            };
            let count = match item_map.get("count").or_else(|| item_map.get("Count")) {
                Some(NbtTag::Int(c)) => *c,
                Some(NbtTag::Byte(c)) => i32::from(*c),
                _ => 1,
            };
            let components = crate::net::decode_item_components_nbt(item_map);
            let eid = *next_eid;
            *next_eid += 1;
            items.push(DroppedItemEntity {
                entity_id: eid,
                uuid: Uuid::new_v3(&Uuid::NAMESPACE_OID, &eid.to_be_bytes()),
                dimension: dim,
                x: *x,
                y: *y,
                z: *z,
                vx,
                vy,
                vz,
                stack: ItemStack {
                    count,
                    item_id,
                    components,
                },
                pickup_delay_ticks,
                age_ticks,
                on_ground: false,
            });
        } else if let Some(kind) = MobKind::from_resource_name(ent_id) {
            let (yaw, pitch) = match map.get("Rotation") {
                Some(NbtTag::List(r)) if r.len() == 2 => match (&r[0], &r[1]) {
                    (NbtTag::Float(yaw), NbtTag::Float(pitch)) => (*yaw, *pitch),
                    _ => (0.0, 0.0),
                },
                _ => (0.0, 0.0),
            };
            let hp = match map.get("Health") {
                Some(NbtTag::Float(v)) => (*v).max(0.1),
                _ => kind.max_hp(),
            };
            let eid = *next_eid;
            *next_eid += 1;
            let uuid = match map.get("UUID") {
                Some(NbtTag::IntArray(arr)) if arr.len() == 4 => {
                    let mut bytes = [0u8; 16];
                    for (idx, word) in arr.iter().enumerate() {
                        bytes[idx * 4..(idx + 1) * 4].copy_from_slice(&word.to_be_bytes());
                    }
                    Uuid::from_bytes(bytes)
                }
                _ => Uuid::new_v3(&Uuid::NAMESPACE_OID, &eid.to_be_bytes()),
            };
            mobs.push(MobEntity {
                entity_id: eid,
                uuid,
                kind,
                dimension: dim,
                x: *x,
                y: *y,
                z: *z,
                yaw,
                pitch,
                vx,
                vy,
                vz,
                hp,
                attack_cooldown: 0,
                wander_ticks: 0,
                wander_angle: 0.0,
                flee_ticks: 0,
            });
        }
    }
}

/// Persists dropped items to Vanilla Anvil `entities/*.mca` per dimension.
#[cfg_attr(not(test), allow(dead_code))]
pub fn write_dropped_items_to_world_mca(
    world_dir: &std::path::Path,
    items: &[DroppedItemEntity],
    registries: &crate::registries::Version26_3Registries,
) -> std::io::Result<()> {
    write_entities_to_world_mca(world_dir, items, &[], registries)
}

/// Persists dropped items and mobs to Vanilla Anvil `entities/*.mca` per dimension
/// and removes legacy `<world_dir>/dropped_items.json` if present.
pub fn write_entities_to_world_mca(
    world_dir: &std::path::Path,
    items: &[DroppedItemEntity],
    mobs: &[MobEntity],
    registries: &crate::registries::Version26_3Registries,
) -> std::io::Result<()> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use rad_core::nbt::{NbtTag, encode_named_nbt};
    use std::collections::{BTreeMap, HashMap};
    use std::io::Write;

    let legacy = world_dir.join("dropped_items.json");
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
    if items.is_empty() && mobs.is_empty() {
        return Ok(());
    }

    let mut by_chunk: HashMap<(DimensionId, i32, i32), Vec<NbtTag>> = HashMap::new();
    for item in items {
        let Some(item_name) = registries.item_name_by_id(item.stack.item_id) else {
            continue;
        };
        let mut item_tag = BTreeMap::new();
        item_tag.insert("id".to_owned(), NbtTag::String(item_name.to_owned()));
        item_tag.insert("count".to_owned(), NbtTag::Int(item.stack.count));
        if let Some(comp_tag) = crate::net::encode_item_components_nbt(&item.stack.components) {
            item_tag.insert("components".to_owned(), comp_tag);
        }

        let mut ent = BTreeMap::new();
        ent.insert("id".to_owned(), NbtTag::String("minecraft:item".to_owned()));
        ent.insert(
            "Pos".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(item.x),
                NbtTag::Double(item.y),
                NbtTag::Double(item.z),
            ]),
        );
        ent.insert(
            "Motion".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(item.vx),
                NbtTag::Double(item.vy),
                NbtTag::Double(item.vz),
            ]),
        );
        ent.insert(
            "PickupDelay".to_owned(),
            NbtTag::Short(item.pickup_delay_ticks.min(i16::MAX as u16) as i16),
        );
        ent.insert(
            "Age".to_owned(),
            NbtTag::Short(item.age_ticks.min(i16::MAX as u16) as i16),
        );
        ent.insert("Item".to_owned(), NbtTag::Compound(item_tag));
        let cx = (item.x.floor() as i32).div_euclid(16);
        let cz = (item.z.floor() as i32).div_euclid(16);
        by_chunk
            .entry((item.dimension, cx, cz))
            .or_default()
            .push(NbtTag::Compound(ent));
    }

    for mob in mobs {
        if mob.hp <= 0.0 {
            continue;
        }
        let b = mob.uuid.as_bytes();
        let uuid_ints = vec![
            i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            i32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            i32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            i32::from_be_bytes([b[12], b[13], b[14], b[15]]),
        ];
        let mut ent = BTreeMap::new();
        ent.insert(
            "id".to_owned(),
            NbtTag::String(mob.kind.resource_name().to_owned()),
        );
        ent.insert("UUID".to_owned(), NbtTag::IntArray(uuid_ints));
        ent.insert(
            "Pos".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(mob.x),
                NbtTag::Double(mob.y),
                NbtTag::Double(mob.z),
            ]),
        );
        ent.insert(
            "Motion".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(mob.vx),
                NbtTag::Double(mob.vy),
                NbtTag::Double(mob.vz),
            ]),
        );
        ent.insert(
            "Rotation".to_owned(),
            NbtTag::List(vec![NbtTag::Float(mob.yaw), NbtTag::Float(mob.pitch)]),
        );
        ent.insert("Health".to_owned(), NbtTag::Float(mob.hp));
        let cx = (mob.x.floor() as i32).div_euclid(16);
        let cz = (mob.z.floor() as i32).div_euclid(16);
        by_chunk
            .entry((mob.dimension, cx, cz))
            .or_default()
            .push(NbtTag::Compound(ent));
    }

    for ((dim, cx, cz), nbt_entities) in by_chunk {
        let mut root = BTreeMap::new();
        root.insert("DataVersion".to_owned(), NbtTag::Int(4790));
        root.insert("Position".to_owned(), NbtTag::IntArray(vec![cx, cz]));
        root.insert("Entities".to_owned(), NbtTag::List(nbt_entities));

        let mut nbt_bytes = bytes::BytesMut::new();
        encode_named_nbt("", &NbtTag::Compound(root), &mut nbt_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&nbt_bytes)?;
        let zlib = enc.finish()?;

        let dim_dir = crate::world::dimension_dir_for(world_dir, dim);
        crate::world::anvil::AnvilRegion::write_subdir_chunk_payload(
            &dim_dir, "entities", cx, cz, &zlib,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_world() -> WorldState {
        let provider: std::sync::Arc<dyn rad_core::registry_provider::RegistryProvider> =
            std::sync::Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = std::sync::Arc::new(
            crate::registries::Version26_3Registries::from_provider(provider, true).unwrap(),
        );
        WorldState::new_with_registries(None, regs)
    }

    #[test]
    fn mob_spatial_buckets_ai_and_combat_work() {
        let mut world = test_world();
        let mut mgr = EntityManager::new();

        mgr.spawn_mob(100, MobKind::Zombie, 5.5, 64.0, 0.5);
        assert_eq!(mgr.mobs_in_chunk(0, 0), &[100]);

        // Survival player at (2.0, 64.0, 0.5) should attract the zombie closer over ticks.
        let players = vec![(1, DimensionId::Overworld, 2.0, 64.0, 0.5, true)];
        let start_x = mgr.get(100).unwrap().x;
        for _ in 0..10 {
            let _ = mgr.tick(&mut world, &players);
        }
        let end_x = mgr.get(100).unwrap().x;
        assert!(end_x < start_x, "zombie should chase player toward X=2.0");

        // Damage & kill the zombie.
        let hit1 = mgr.damage_mob(100, 10.0, 1.0, 0.0).unwrap();
        assert!(!hit1.killed);
        assert_eq!(hit1.remaining_hp, 10.0);

        let hit2 = mgr.damage_mob(100, 12.0, 1.0, 0.0).unwrap();
        assert!(hit2.killed);
        assert_eq!(mgr.len(), 0);
        assert!(mgr.mobs_in_chunk(0, 0).is_empty());
    }

    #[test]
    fn passive_mob_flees_when_damaged_and_projectile_hits_mob() {
        let mut world = test_world();
        let mut mgr = EntityManager::new();

        // Pig takes damage and flees along +X.
        mgr.spawn_mob(200, MobKind::Pig, 0.5, 64.0, 0.5);
        let hit = mgr.damage_mob(200, 2.0, 1.0, 0.0).unwrap();
        assert!(!hit.killed);
        assert!(mgr.get(200).unwrap().flee_ticks > 0);
        let before_x = mgr.get(200).unwrap().x;
        for _ in 0..5 {
            let _ = mgr.tick(&mut world, &[]);
        }
        assert!(mgr.get(200).unwrap().x > before_x);

        // Arrow fired at a zombie hits and damages it, despawning the projectile.
        mgr.spawn_mob(201, MobKind::Zombie, 5.5, 64.0, 0.5);
        let entities = &world.registries().entities;
        let pkt = mgr
            .spawn_projectile(
                300,
                ProjectileKind::Arrow,
                1,
                [2.5, 65.0, 0.5],
                [1.0, 0.05, 0.0],
            )
            .to_spawn_packet_with(entities);
        assert_eq!(pkt.entity_id.0, 300);
        assert_eq!(mgr.projectile_count(), 1);

        for _ in 0..10 {
            let _ = mgr.tick(&mut world, &[]);
        }
        assert_eq!(mgr.projectile_count(), 0);
        assert!(mgr.get(201).unwrap().hp < MobKind::Zombie.max_hp());
    }

    #[test]
    fn mob_and_projectile_pause_outside_simulation_area_and_resume_inside() {
        let mut world = test_world();
        let mut mgr = EntityManager::new();

        // Spawn mob and projectile in chunk (3, 0) at x=50.5 (y=70 in air so gravity would move them)
        mgr.spawn_mob(400, MobKind::Zombie, 50.5, 70.0, 0.5);
        mgr.spawn_projectile(
            401,
            ProjectileKind::Arrow,
            1,
            [50.5, 70.0, 0.5],
            [0.5, 0.0, 0.0],
        );

        // Player at (0.5, 0.5) with simulation_distance=1 covers chunks -1..=1 (chunk (3, 0) is inactive)
        let far_area = SimulationArea::new(1, [(DimensionId::Overworld, 0.5, 0.5)]);
        for _ in 0..20 {
            let events = mgr.tick_in_area(&mut world, &[], &far_area);
            assert!(events.is_empty());
        }
        assert_eq!(mgr.get(400).unwrap().y, 70.0);
        assert_eq!(mgr.projectile_count(), 1);

        // Player moves to chunk (3, 0): mob falls and projectile advances
        let near_area = SimulationArea::new(1, [(DimensionId::Overworld, 50.5, 0.5)]);
        let events = mgr.tick_in_area(&mut world, &[], &near_area);
        assert!(!events.is_empty());
        assert!(mgr.get(400).unwrap().y < 70.0);
    }

    #[test]
    fn dropped_item_physics_merging_and_component_isolation() {
        use rad_core::packet::play::ItemComponentPatch;
        let mut world = test_world();
        // Solid floor at y=63 so items spawned at y=65 rest at y=64
        world.set_block_in(DimensionId::Overworld, 0, 63, 0, 1);

        let mut mgr = EntityManager::new();
        let plain_a = ItemStack {
            item_id: 5,
            count: 2,
            components: ItemComponentPatch::default(),
        };
        let plain_b = ItemStack {
            item_id: 5,
            count: 3,
            components: ItemComponentPatch::default(),
        };
        let named = ItemStack {
            item_id: 5,
            count: 1,
            components: ItemComponentPatch {
                custom_name: Some("Special Plank".to_owned()),
                ..Default::default()
            },
        };

        assert!(
            mgr.spawn_dropped_item_in(
                DimensionId::Overworld,
                100,
                [0.5, 65.0, 0.5],
                [0.0, 0.0, 0.0],
                plain_a,
                40,
            )
            .is_some()
        );
        assert!(
            mgr.spawn_dropped_item_in(
                DimensionId::Overworld,
                101,
                [0.6, 65.0, 0.5],
                [0.0, 0.0, 0.0],
                plain_b,
                40,
            )
            .is_some()
        );
        assert!(
            mgr.spawn_dropped_item_in(
                DimensionId::Overworld,
                102,
                [0.6, 65.0, 0.5],
                [0.0, 0.0, 0.0],
                named,
                40,
            )
            .is_some()
        );

        let area = SimulationArea::new(2, [(DimensionId::Overworld, 0.5, 0.5)]);
        let mut saw_merge = false;
        for _ in 0..20 {
            let events = mgr.tick_in_area(&mut world, &[], &area);
            for ev in events {
                if let MobTickEvent::ItemMerged {
                    kept_entity_id,
                    removed_entity_id,
                    stack,
                    ..
                } = ev
                {
                    assert_eq!(kept_entity_id, 100);
                    assert_eq!(removed_entity_id, 101);
                    assert_eq!(stack.count, 5);
                    saw_merge = true;
                }
            }
        }
        assert!(saw_merge);
        // Compatible plain stacks merged into 100 (count=5), named stack 102 stayed separate
        assert_eq!(mgr.dropped_item_count(), 2);
        let kept = mgr.get_dropped_item_mut(100).unwrap();
        assert_eq!(kept.stack.count, 5);
        assert!(kept.on_ground);
        assert_eq!(kept.pickup_delay_ticks, 20);
        let separate = mgr.get_dropped_item_mut(102).unwrap();
        assert_eq!(separate.stack.count, 1);
        assert_eq!(
            separate.stack.components.custom_name.as_deref(),
            Some("Special Plank")
        );
    }

    #[test]
    fn dropped_items_vanilla_entities_mca_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "rad-entities-mca-{}-{}",
            std::process::id(),
            crate::net::unique_tmp_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let provider: std::sync::Arc<dyn rad_core::registry_provider::RegistryProvider> =
            std::sync::Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = crate::registries::Version26_3Registries::from_provider(provider, true)
            .expect("synthetic registries");
        let item_id = regs.item_by_name("minecraft:cobblestone").unwrap_or(1);

        let items = vec![DroppedItemEntity {
            entity_id: 1,
            uuid: Uuid::from_u128(1),
            dimension: DimensionId::Overworld,
            x: 8.5,
            y: 64.25,
            z: -3.75,
            vx: 0.05,
            vy: 0.2,
            vz: -0.05,
            stack: ItemStack {
                count: 12,
                item_id,
                components: Default::default(),
            },
            pickup_delay_ticks: 10,
            age_ticks: 5,
            on_ground: false,
        }];

        write_dropped_items_to_world_mca(&dir, &items, &regs).unwrap();
        assert!(dir.join("entities").join("r.0.-1.mca").exists());
        assert!(!dir.join("dropped_items.json").exists());

        let loaded = load_dropped_items_from_world_mca(&dir, &regs);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].stack.item_id, item_id);
        assert_eq!(loaded[0].stack.count, 12);
        assert_eq!(loaded[0].pickup_delay_ticks, 10);
        assert_eq!(loaded[0].age_ticks, 5);
        assert!((loaded[0].x - 8.5).abs() < f64::EPSILON);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mobs_and_items_vanilla_entities_mca_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rad_mobs_mca_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let provider: std::sync::Arc<dyn rad_core::registry_provider::RegistryProvider> =
            std::sync::Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = crate::registries::Version26_3Registries::from_provider(provider, true).unwrap();

        let mob_uuid = Uuid::new_v3(&Uuid::NAMESPACE_OID, b"pig-1");
        let mobs = vec![MobEntity {
            entity_id: 42,
            uuid: mob_uuid,
            kind: MobKind::Pig,
            dimension: DimensionId::Overworld,
            x: 12.25,
            y: 64.0,
            z: 7.5,
            yaw: 45.0,
            pitch: -10.0,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            hp: 8.5,
            attack_cooldown: 0,
            wander_ticks: 0,
            wander_angle: 0.0,
            flee_ticks: 0,
        }];

        write_entities_to_world_mca(&dir, &[], &mobs, &regs).unwrap();
        assert!(dir.join("entities").join("r.0.0.mca").exists());

        let (loaded_items, loaded_mobs) = load_entities_from_world_mca(&dir, &regs);
        assert!(loaded_items.is_empty());
        assert_eq!(loaded_mobs.len(), 1);
        assert_eq!(loaded_mobs[0].kind, MobKind::Pig);
        assert_eq!(loaded_mobs[0].uuid, mob_uuid);
        assert!((loaded_mobs[0].hp - 8.5).abs() < f32::EPSILON);
        assert!((loaded_mobs[0].x - 12.25).abs() < f64::EPSILON);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
