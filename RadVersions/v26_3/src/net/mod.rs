//! Multiplayer game loop, 20 TPS ticker, and connection handler for `RadServer`.

mod access;
mod commands;
mod fallback_tags;
mod login;
mod play;
mod play_helpers;
mod tick;

pub use access::AccessControl;
pub(crate) use commands::handle_console_command;
use commands::*;
use login::*;
use play::*;
use play_helpers::handle_player_move;
use tick::apply_player_damage;
pub use tick::{
    spawn_and_broadcast_dropped_item, spawn_tick_loop, spawn_tick_loop_with_shutdown,
    step_player_survival_tick, step_world_tick, step_world_tick_in_area,
};

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use bytes::{BufMut, Bytes, BytesMut};
use rad_api::v1::{ServerCommandAction, ServerEventResult, ServerPlayerContext};
use rad_core::MINECRAFT_VERSION;
use rad_core::codec::{DEFAULT_MAX_PACKET_SIZE, decode_frame, encode_packet};
use rad_core::forwarding::{VELOCITY_FORWARDING_CHANNEL, decode_velocity_forwarding};
use rad_core::nbt::NbtTag;
use rad_core::packet::Packet;
use rad_core::packet::configuration::{
    AcknowledgeFinishConfiguration, ClientboundKnownPacks, ConfigClientboundPluginMessage,
    FinishConfiguration, KnownPack, RegistryData,
};
use rad_core::packet::handshake::Handshake;
use rad_core::packet::login::{
    GameProfileProperty, LoginAcknowledged, LoginDisconnect, LoginPluginRequest,
    LoginPluginResponse, LoginStart, LoginSuccess,
};
use rad_core::packet::play::{
    AcknowledgeBlockChange, Attack, BlockUpdate, ChatCommand, ClientCommand, CommandNode,
    CommandSuggestionMatch, CommandSuggestions, CommandSuggestionsRequest, ConfirmTeleportation,
    ContainerClick, ContainerClose, DeclareCommands, GameEvent, HurtAnimation,
    KeepAliveClientbound, KeepAliveServerbound, MoveEntityPosRot, MovePlayerPos, MovePlayerPosRot,
    MovePlayerRot, MovePlayerStatusOnly, OpenScreen, PlayClientboundPluginMessage, PlayDisconnect,
    PlayLogin, PlayerAction, PlayerInfoAddEntry, PlayerInfoRemove, PlayerInfoUpdate,
    RemoveEntities, Respawn, RotateHead, ServerboundChat, SetCarriedItem, SetChunkCacheCenter,
    SetChunkCacheRadius, SetCreativeModeSlot, SetDefaultSpawnPosition, SetEntityMotion, SetHealth,
    SetHeldSlot, SpawnEntity, SynchronizePlayerPosition, SystemChatMessage, UnloadChunk, UseItem,
    UseItemOn,
};
use rad_core::packet::status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
use rad_core::state::ConnectionState;
use rad_core::text::TextComponent;
use rad_core::types::{BlockPos, LpVec3, VarInt, write_string};
use serde_json::json;

/// Entity event 24..28 updates the local client's operator permission level.
struct PermissionEvent {
    entity_id: i32,
    level: u8,
}

impl Packet for PermissionEvent {
    const ID: i32 = 0x22;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: rad_core::packet::PacketDirection =
        rad_core::packet::PacketDirection::Clientbound;
    fn decode_body(buf: &mut impl bytes::Buf) -> rad_core::error::Result<Self> {
        use rad_core::types::Decode;
        let entity_id = i32::decode(buf)?;
        let event = u8::decode(buf)?;
        if !(24..=28).contains(&event) {
            return Err(rad_core::error::Error::InvalidPacket(
                "invalid permission event".into(),
            ));
        }
        Ok(Self {
            entity_id,
            level: event - 24,
        })
    }
    fn encode_body(&self, buf: &mut impl bytes::BufMut) -> rad_core::error::Result<()> {
        use rad_core::types::Encode;
        self.entity_id.encode(buf)?;
        (24 + self.level.min(4)).encode(buf)
    }
}

struct ChangeGameMode(VarInt);
impl Packet for ChangeGameMode {
    const ID: i32 = 0x05;
    const STATE: ConnectionState = ConnectionState::Play;
    const DIRECTION: rad_core::packet::PacketDirection =
        rad_core::packet::PacketDirection::Serverbound;
    fn decode_body(buf: &mut impl bytes::Buf) -> rad_core::error::Result<Self> {
        use rad_core::types::Decode;
        Ok(Self(VarInt::decode(buf)?))
    }
    fn encode_body(&self, buf: &mut impl bytes::BufMut) -> rad_core::error::Result<()> {
        use rad_core::types::Encode;
        self.0.encode(buf)
    }
}
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{MissedTickBehavior, interval, timeout};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::{ServerConfig, ServerForwardingMode};
use crate::gameplay::entity::{EntityManager, MobKind, MobTickEvent};
use crate::gameplay::inventory::PlayerInventory;
use crate::radon::ServerRadonManager;
use crate::registries::{ArcPackageRegistryProvider, Version26_3Registries};
use crate::world::physics::{Aabb, MoveValidation, compute_fall_damage};
use crate::world::{WorldState, chunks_in_radius};

/// Connected player session tracked by [`ServerState`].
#[derive(Clone)]
pub struct PlayerSession {
    /// Server-assigned entity ID.
    pub entity_id: i32,
    /// Player UUID.
    pub uuid: Uuid,
    /// Player username.
    pub username: String,
    /// Profile properties (e.g. skin textures from forwarding).
    pub properties: Vec<GameProfileProperty>,
    /// Current X coordinate.
    pub x: f64,
    /// Current Y coordinate.
    pub y: f64,
    /// Current Z coordinate.
    pub z: f64,
    /// Current yaw in degrees.
    pub yaw: f32,
    /// Current pitch in degrees.
    pub pitch: f32,
    /// Current health (`0.0..=20.0`).
    pub hp: f32,
    /// Food level (`0..=20`).
    pub food: i32,
    /// Food saturation (`0.0..=5.0`).
    pub saturation: f32,
    /// Whether the player is currently on ground.
    pub on_ground: bool,
    /// Highest Y reached during the current airborne fall.
    pub fall_start_y: Option<f64>,
    /// Whether the player is waiting for respawn.
    pub dead: bool,
    /// Active game mode (`0` = survival, `1` = creative, etc.).
    pub game_mode: u8,
    /// Active dimension for this player session.
    pub dimension: crate::world::worldgen::DimensionId,
    /// Snapshot of inventory slots for live `save_all`.
    pub inventory: Vec<SavedSlot>,
    /// Selected hotbar index (`0..=8`).
    pub selected_hotbar: u8,
    /// Open chest container position (if any) for multi-viewer sync.
    pub open_chest: Option<(crate::world::worldgen::DimensionId, i32, i32, i32)>,
    /// Channel to request a per-viewer chest resync using that viewer's own `state_id` and inventory.
    pub chest_sync_tx:
        Option<mpsc::UnboundedSender<(crate::world::worldgen::DimensionId, i32, i32, i32)>>,
    /// Pending server-initiated teleport awaiting `ConfirmTeleportation`.
    pub pending_teleport_id: Option<i32>,
    /// Last keep-alive ID awaiting acknowledgement and when it was sent.
    pub pending_keep_alive: Option<(i64, Instant)>,
    /// Timestamp of the last non-keep-alive player activity (for `player-idle-timeout`).
    pub last_activity: Instant,
    /// Byte-budgeted outbound queue + unified session cancellation handle.
    pub tx: OutboundSender,
}

/// Default per-player outbound byte budget (64 MiB, accommodates high view-distance chunk bursts).
pub const MAX_PLAYER_OUTBOUND_BYTES: usize = 64 * 1024 * 1024;

/// Bounded, byte-budgeted outbound packet sender with unified session cancellation.
#[derive(Clone)]
pub struct OutboundSender {
    tx: mpsc::Sender<Bytes>,
    queued_bytes: Arc<AtomicUsize>,
    max_bytes: usize,
    cancel_tx: tokio::sync::watch::Sender<bool>,
    space_available: Arc<tokio::sync::Notify>,
}

/// Receiving half of [`outbound_channel`] that releases byte budget on dequeue.
pub struct OutboundReceiver {
    rx: mpsc::Receiver<Bytes>,
    queued_bytes: Arc<AtomicUsize>,
    space_available: Arc<tokio::sync::Notify>,
}

/// Creates a bounded, byte-budgeted outbound channel paired with a session cancellation token.
pub fn outbound_channel(
    max_messages: usize,
    max_bytes: usize,
) -> (
    OutboundSender,
    OutboundReceiver,
    tokio::sync::watch::Receiver<bool>,
) {
    let (tx, rx) = mpsc::channel(max_messages);
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    let space_available = Arc::new(tokio::sync::Notify::new());
    (
        OutboundSender {
            tx,
            queued_bytes: Arc::clone(&queued_bytes),
            max_bytes,
            cancel_tx,
            space_available: Arc::clone(&space_available),
        },
        OutboundReceiver {
            rx,
            queued_bytes,
            space_available,
        },
        cancel_rx,
    )
}

impl OutboundSender {
    /// Waits for outbound capacity without disconnecting a player during chunk streaming.
    pub async fn send_wait(&self, bytes: Bytes) -> bool {
        let len = bytes.len();
        if len > self.max_bytes {
            self.disconnect();
            return false;
        }
        let mut cancelled = self.cancel_tx.subscribe();
        loop {
            let notified = self.space_available.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if *cancelled.borrow() || self.tx.is_closed() {
                return false;
            }
            if self
                .queued_bytes
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                    used.checked_add(len)
                        .filter(|&total| total <= self.max_bytes)
                })
                .is_ok()
            {
                let result = tokio::select! {
                    result = self.tx.send(bytes) => result.is_ok(),
                    _ = cancelled.wait_for(|&v| v) => false,
                };
                if !result {
                    self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
                    self.space_available.notify_waiters();
                }
                return result;
            }
            tokio::select! {
                _ = &mut notified => {},
                _ = cancelled.wait_for(|&v| v) => return false,
            }
        }
    }
    /// Queues `bytes` if within both message and byte budgets; on overflow, disconnects the session.
    pub fn try_send(&self, bytes: Bytes) -> bool {
        if *self.cancel_tx.borrow() {
            return false;
        }
        let len = bytes.len();
        let prev = self.queued_bytes.fetch_add(len, Ordering::Relaxed);
        if prev.saturating_add(len) > self.max_bytes {
            self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
            tracing::warn!(
                queued_bytes = prev,
                incoming_bytes = len,
                max_bytes = self.max_bytes,
                "outbound byte budget exceeded; disconnecting player"
            );
            self.disconnect();
            return false;
        }
        match self.tx.try_send(bytes) {
            Ok(()) => true,
            Err(err) => {
                let is_full = matches!(err, mpsc::error::TrySendError::Full(_));
                let returned_len = match err {
                    mpsc::error::TrySendError::Full(b) | mpsc::error::TrySendError::Closed(b) => {
                        b.len()
                    }
                };
                self.queued_bytes.fetch_sub(returned_len, Ordering::Relaxed);
                if is_full {
                    tracing::warn!(
                        incoming_bytes = returned_len,
                        "outbound message queue full; disconnecting player"
                    );
                }
                self.disconnect();
                false
            }
        }
    }

    /// Signals unified cancellation for reader, writer, and session state.
    pub fn disconnect(&self) {
        let _ = self.cancel_tx.send(true);
    }

    /// Returns `true` if this session has been cancelled.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_cancelled(&self) -> bool {
        *self.cancel_tx.borrow()
    }

    /// Returns `true` if `self` and `other` target the same underlying player channel.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.tx.same_channel(&other.tx)
    }
}

impl OutboundReceiver {
    /// Receives the next queued frame and releases its byte budget.
    pub async fn recv(&mut self) -> Option<Bytes> {
        let bytes = self.rx.recv().await?;
        self.queued_bytes.fetch_sub(bytes.len(), Ordering::Relaxed);
        self.space_available.notify_waiters();
        Some(bytes)
    }
}

impl Drop for OutboundReceiver {
    fn drop(&mut self) {
        self.rx.close();
        self.space_available.notify_waiters();
    }
}

/// Shared multiplayer state for `RadServer`.
pub struct ServerState {
    /// Active configuration.
    pub config: ServerConfig,
    /// Per-boot server session UUID sent in `login_finished`.
    pub server_session_id: Uuid,
    /// Next unique entity ID.
    pub next_entity_id: AtomicI32,
    /// Total executed server ticks.
    pub tick_count: AtomicU64,
    /// Last `tick_count` persisted to `level_meta`.
    last_saved_tick: AtomicU64,
    /// Serializes concurrent `save_all` calls.
    save_mutex: tokio::sync::Mutex<()>,
    /// Last tick duration in microseconds.
    pub last_tick_us: AtomicU64,
    /// Active players keyed by `entity_id`.
    pub players: RwLock<HashMap<i32, PlayerSession>>,
    /// Per-player `(portal_ticks, portal_cooldown_ticks)` for Nether/End portal transitions.
    pub portal_state: RwLock<HashMap<i32, (u16, u16)>>,
    /// Chunks whose `block_entities`, `block_ticks`, and `entities` have already been lazily loaded from `.mca`.
    pub loaded_extra_chunks: RwLock<HashSet<(crate::world::worldgen::DimensionId, i32, i32)>>,
    /// Live player inventories keyed by `entity_id` for atomic item pickups.
    pub live_inventories:
        RwLock<HashMap<i32, Arc<std::sync::Mutex<crate::gameplay::inventory::PlayerInventory>>>>,
    /// Mutable world chunk store.
    pub world: RwLock<WorldState>,
    /// Persisted 27-slot chest container block entities.
    pub containers: RwLock<crate::world::containers::ContainerStore>,
    /// Server whitelist and ban lists (`world/access_control.json`).
    pub access: RwLock<AccessControl>,
    /// Vanilla-compatible `level.dat` metadata.
    pub level_meta: RwLock<crate::world::LevelMetadata>,
    /// Scheduled-tick world simulation engine (gravity, fluids, crops, fire).
    pub world_sim: RwLock<crate::world::sim::WorldSim>,
    /// Persistent furnace block entities (`world/furnaces.json`).
    pub furnaces: RwLock<crate::world::sim::FurnaceStore>,
    /// Active world entities and mobs.
    pub entities: RwLock<EntityManager>,
    /// Active player dimension overrides keyed by player UUID.
    pub player_dimensions: RwLock<HashMap<Uuid, crate::world::worldgen::DimensionId>>,
    /// Per-player gameplay statistics keyed by player UUID.
    pub player_stats: RwLock<HashMap<Uuid, PlayerStats>>,
    /// Registered native and WASM server radons (`RadAPI v3`).
    pub radons: RwLock<ServerRadonManager>,
    /// Loaded runtime crafting recipes (`<world_dir>/datapacks/recipes/*.json`).
    pub recipes: RwLock<Vec<crate::gameplay::inventory::DataPackRecipe>>,
    /// Validated runtime Minecraft version package (`versions/26.3/`).
    pub version_package: Arc<rad_core::version_package::LoadedVersionPackage>,
    /// Pre-resolved 26.3 registries backed by `version_package`.
    pub registries: Arc<Version26_3Registries>,
}

impl ServerState {
    /// Creates a new [`ServerState`] with optional Anvil `.mca` persistence directory
    /// and a validated runtime [`LoadedVersionPackage`](rad_core::version_package::LoadedVersionPackage).
    #[must_use]
    pub fn new_with_world_dir_and_package(
        config: ServerConfig,
        world_dir: Option<PathBuf>,
        version_package: Arc<rad_core::version_package::LoadedVersionPackage>,
    ) -> Self {
        let registries = Arc::new(
            Version26_3Registries::from_provider(
                Arc::new(ArcPackageRegistryProvider(Arc::clone(&version_package))),
                false,
            )
            .expect("validated version package must contain all required 26.3 registries"),
        );
        Self::new_with_world_dir_and_registries(config, world_dir, version_package, registries)
    }

    /// Creates a new [`ServerState`] with explicit `version_package` and resolved `registries`.
    #[must_use]
    pub fn new_with_world_dir_and_registries(
        config: ServerConfig,
        world_dir: Option<PathBuf>,
        version_package: Arc<rad_core::version_package::LoadedVersionPackage>,
        registries: Arc<Version26_3Registries>,
    ) -> Self {
        let boot_seed = format!(
            "radserver:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        if let Some(dir) = world_dir.as_deref()
            && dir.exists()
        {
            if dir.join("legacy_block_states.json").exists()
                && let Ok(report) =
                    crate::world::migrate::migrate_legacy_world_with(dir, false, &registries.items)
            {
                for q in &report.quarantined_records {
                    tracing::warn!(quarantine = %q, "legacy world migration quarantined ambiguous entry");
                }
            }
            for legacy_file in ["radbyte_world_profile.json", "schema_version.json"] {
                let p = dir.join(legacy_file);
                if p.exists() {
                    let _ = std::fs::remove_file(p);
                }
            }
            for legacy_empty_dir in ["playerdata", "backups"] {
                let d = dir.join(legacy_empty_dir);
                if d.is_dir() {
                    let _ = std::fs::remove_dir(d);
                }
            }
        }
        let mut containers = world_dir
            .as_deref()
            .map(crate::world::containers::ContainerStore::load_from_dir)
            .unwrap_or_default();
        let mut access = world_dir
            .as_deref()
            .map(AccessControl::load_from_dir)
            .unwrap_or_default();
        if let Some(wl) = config.white_list {
            access.whitelist_enabled = wl;
        }
        let mut level_meta = world_dir
            .as_deref()
            .map(crate::world::LevelMetadata::load_from_dir)
            .unwrap_or_default();
        level_meta.level_name = config.level_name.clone();
        if config.world_seed != 0 {
            level_meta.random_seed = config.world_seed;
        }
        level_meta.game_type = i32::from(config.game_mode);
        let mut furnaces = world_dir
            .as_deref()
            .map(crate::world::sim::FurnaceStore::load_from_dir)
            .unwrap_or_default();
        let mut world_sim = world_dir
            .as_deref()
            .map(crate::world::sim::WorldSim::load_from_dir)
            .unwrap_or_default();
        let mut entity_mgr = EntityManager::new();
        let mut loaded_extra_chunks = HashSet::new();
        let mut next_eid = 10_000_i32;
        if let Some(dir) = world_dir.as_deref() {
            let spawn_cx = level_meta.spawn_x.div_euclid(16);
            let spawn_cz = level_meta.spawn_z.div_euclid(16);
            let preload_r = config.simulation_distance.clamp(1, 6);
            for (cx, cz) in crate::world::chunks_in_radius(spawn_cx, spawn_cz, preload_r) {
                loaded_extra_chunks.insert((
                    crate::world::worldgen::DimensionId::Overworld,
                    cx,
                    cz,
                ));
                let (bes, ticks) =
                    crate::world::WorldState::load_chunk_block_entities_and_ticks_from_mca(
                        dir,
                        crate::world::worldgen::DimensionId::Overworld,
                        cx,
                        cz,
                    );
                apply_block_entities_to_stores(bes, &registries, &mut containers, &mut furnaces);
                apply_block_ticks_to_sim(ticks, &mut world_sim);
                let (items, mobs) = crate::gameplay::entity::load_entities_in_chunk_from_mca(
                    dir,
                    crate::world::worldgen::DimensionId::Overworld,
                    cx,
                    cz,
                    &mut next_eid,
                    &registries,
                );
                for item in items {
                    entity_mgr.insert_dropped_item(item);
                }
                for mob in mobs {
                    entity_mgr.insert_mob(mob);
                }
            }
        }
        let recipes = world_dir
            .as_deref()
            .map(crate::gameplay::inventory::load_custom_recipes_from_dir)
            .unwrap_or_default();
        let is_vanilla_3d_world = world_dir
            .as_deref()
            .is_some_and(|dir| dir.join("dimensions").is_dir());
        let generator = if config.world_type.eq_ignore_ascii_case("seeded") || is_vanilla_3d_world {
            let seed = if level_meta.random_seed != 0 {
                level_meta.random_seed
            } else if config.world_seed != 0 {
                config.world_seed
            } else {
                0x5EED_2603_i64
            };
            crate::world::worldgen::WorldGenerator::Seeded(seed)
        } else {
            crate::world::worldgen::WorldGenerator::Flat
        };
        Self {
            config,
            server_session_id: Uuid::new_v3(&Uuid::NAMESPACE_OID, boot_seed.as_bytes()),
            next_entity_id: AtomicI32::new(next_eid.max(1)),
            tick_count: AtomicU64::new(0),
            last_saved_tick: AtomicU64::new(0),
            save_mutex: tokio::sync::Mutex::new(()),
            last_tick_us: AtomicU64::new(0),
            players: RwLock::new(HashMap::new()),
            portal_state: RwLock::new(HashMap::new()),
            loaded_extra_chunks: RwLock::new(loaded_extra_chunks),
            live_inventories: RwLock::new(HashMap::new()),
            world: RwLock::new(
                WorldState::new_with_registries(world_dir, Arc::clone(&registries))
                    .with_generator(generator),
            ),
            containers: RwLock::new(containers),
            access: RwLock::new(access),
            level_meta: RwLock::new(level_meta),
            world_sim: RwLock::new(world_sim),
            furnaces: RwLock::new(furnaces),
            entities: RwLock::new(entity_mgr),
            player_dimensions: RwLock::new(HashMap::new()),
            player_stats: RwLock::new(HashMap::new()),
            radons: RwLock::new(ServerRadonManager::load_dir(std::path::Path::new("radons"))),
            recipes: RwLock::new(recipes),
            version_package,
            registries,
        }
    }

    /// Executes imperative [`ServerCommandAction`]s emitted by radons.
    pub async fn execute_radon_actions(&self, actions: Vec<ServerCommandAction>) {
        for action in actions {
            match action {
                ServerCommandAction::BroadcastChat(msg) => {
                    self.broadcast_chat(msg).await;
                }
                ServerCommandAction::SendMessage { entity_id, message } => {
                    if let Some(p) = self.players.read().await.get(&entity_id) {
                        send_chat_to(&p.tx, message);
                    }
                }
                ServerCommandAction::TeleportPlayer { entity_id, x, y, z } => {
                    if let Some(p) = self.players.write().await.get_mut(&entity_id) {
                        p.x = x;
                        p.y = y;
                        p.z = z;
                        p.pending_teleport_id = Some(77);
                        if let Ok(bytes) = encode_to_bytes(&SynchronizePlayerPosition {
                            teleport_id: VarInt(77),
                            x,
                            y,
                            z,
                            vx: 0.0,
                            vy: 0.0,
                            vz: 0.0,
                            yaw: p.yaw,
                            pitch: p.pitch,
                            flags: 0,
                        }) {
                            let _ = p.tx.try_send(bytes);
                        }
                    }
                }
                ServerCommandAction::SetBlock { x, y, z, state_id } => {
                    if self.world.write().await.set_block(x, y, z, state_id) {
                        self.broadcast_in_dimension_except(
                            crate::world::worldgen::DimensionId::Overworld,
                            &BlockUpdate {
                                pos: BlockPos::new(x, y, z),
                                block_state: VarInt(i32::from(state_id)),
                            },
                            None,
                        )
                        .await;
                    }
                }
                ServerCommandAction::KickPlayer { entity_id, reason } => {
                    if let Some(p) = self.players.read().await.get(&entity_id) {
                        if let Ok(bytes) = encode_to_bytes(&PlayDisconnect {
                            reason: NbtTag::String(reason),
                        }) {
                            let _ = p.tx.try_send(bytes);
                        }
                        p.tx.disconnect();
                    }
                }
            }
        }
    }

    /// Advances `level_meta` world age and day time by elapsed server ticks since last sync,
    /// returning `(game_time, day_time)`.
    pub async fn sync_level_time(&self) -> (i64, i64) {
        let mut meta = self.level_meta.write().await;
        let cur_tick = self.tick_count.load(Ordering::Relaxed);
        let prev_tick = self.last_saved_tick.swap(cur_tick, Ordering::Relaxed);
        let delta = cur_tick.saturating_sub(prev_tick) as i64;
        meta.time = meta.time.saturating_add(delta);
        meta.day_time = meta.day_time.saturating_add(delta);
        (meta.time, meta.day_time)
    }

    /// Sets `day_time` (clamped `>= 0`) after syncing elapsed ticks, returning `(game_time, day_time)`.
    pub async fn set_day_time(&self, day_time: i64) -> (i64, i64) {
        let mut meta = self.level_meta.write().await;
        let cur_tick = self.tick_count.load(Ordering::Relaxed);
        let prev_tick = self.last_saved_tick.swap(cur_tick, Ordering::Relaxed);
        let delta = cur_tick.saturating_sub(prev_tick) as i64;
        meta.time = meta.time.saturating_add(delta);
        meta.day_time = day_time.max(0);
        (meta.time, meta.day_time)
    }

    /// Adds `delta_day_time` to `day_time` after syncing elapsed ticks, returning `(game_time, day_time)`.
    pub async fn add_day_time(&self, delta_day_time: i64) -> (i64, i64) {
        let mut meta = self.level_meta.write().await;
        let cur_tick = self.tick_count.load(Ordering::Relaxed);
        let prev_tick = self.last_saved_tick.swap(cur_tick, Ordering::Relaxed);
        let delta = cur_tick.saturating_sub(prev_tick) as i64;
        meta.time = meta.time.saturating_add(delta);
        meta.day_time = meta
            .day_time
            .saturating_add(delta)
            .saturating_add(delta_day_time)
            .max(0);
        (meta.time, meta.day_time)
    }

    /// Broadcasts an encoded packet to all players except `exclude_entity_id`.
    pub async fn broadcast_except<P: Packet>(&self, pkt: &P, exclude_entity_id: Option<i32>) {
        let Ok(bytes) = encode_to_bytes(pkt) else {
            return;
        };
        let players = self.players.read().await;
        for (&eid, session) in players.iter() {
            if Some(eid) != exclude_entity_id {
                let _ = session.tx.try_send(bytes.clone());
            }
        }
    }

    /// Broadcasts an encoded packet to all players in `dim` except `exclude_entity_id`.
    pub async fn broadcast_in_dimension_except<P: Packet>(
        &self,
        dim: crate::world::worldgen::DimensionId,
        pkt: &P,
        exclude_entity_id: Option<i32>,
    ) {
        let Ok(bytes) = encode_to_bytes(pkt) else {
            return;
        };
        let players = self.players.read().await;
        for (&eid, session) in players.iter() {
            if Some(eid) != exclude_entity_id && session.dimension == dim {
                let _ = session.tx.try_send(bytes.clone());
            }
        }
    }

    /// Switches `entity_id` to `target_dim`, sending `Respawn`, new chunks, and updating visibility.
    /// Lazily loads `block_entities`, `block_ticks`, and `entities` for any unvisited chunks in `coords`.
    pub async fn ensure_chunks_extras_loaded(
        &self,
        dim: crate::world::worldgen::DimensionId,
        coords: &[(i32, i32)],
    ) {
        let Some(world_dir) = self.world.read().await.world_dir().map(PathBuf::from) else {
            return;
        };
        let to_load: Vec<(i32, i32)> = {
            let mut loaded = self.loaded_extra_chunks.write().await;
            coords
                .iter()
                .copied()
                .filter(|&(cx, cz)| loaded.insert((dim, cx, cz)))
                .collect()
        };
        if to_load.is_empty() {
            return;
        }
        let mut all_bes = Vec::new();
        let mut all_ticks = Vec::new();
        let mut all_items = Vec::new();
        let mut all_mobs = Vec::new();
        let mut tmp_eid = 1_i32;
        for (cx, cz) in to_load {
            let (bes, ticks) =
                crate::world::WorldState::load_chunk_block_entities_and_ticks_from_mca(
                    &world_dir, dim, cx, cz,
                );
            all_bes.extend(bes);
            all_ticks.extend(ticks);
            let (items, mobs) = crate::gameplay::entity::load_entities_in_chunk_from_mca(
                &world_dir,
                dim,
                cx,
                cz,
                &mut tmp_eid,
                &self.registries,
            );
            all_items.extend(items);
            all_mobs.extend(mobs);
        }
        if !all_bes.is_empty() {
            let mut containers = self.containers.write().await;
            let mut furnaces = self.furnaces.write().await;
            apply_block_entities_to_stores(
                all_bes,
                &self.registries,
                &mut containers,
                &mut furnaces,
            );
        }
        if !all_ticks.is_empty() {
            let mut sim = self.world_sim.write().await;
            apply_block_ticks_to_sim(all_ticks, &mut sim);
        }
        if !all_items.is_empty() || !all_mobs.is_empty() {
            let mut entities = self.entities.write().await;
            for mut item in all_items {
                item.entity_id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
                entities.insert_dropped_item(item);
            }
            for mut mob in all_mobs {
                mob.entity_id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);
                entities.insert_mob(mob);
            }
        }
    }

    /// Switches a player's active dimension at their current coordinates.
    pub async fn switch_player_dimension(
        &self,
        entity_id: i32,
        target_dim: crate::world::worldgen::DimensionId,
    ) {
        self.switch_player_dimension_at(entity_id, target_dim, None)
            .await;
    }

    /// Switches a player's active dimension, optionally moving them to `target_pos`.
    pub async fn switch_player_dimension_at(
        &self,
        entity_id: i32,
        target_dim: crate::world::worldgen::DimensionId,
        target_pos: Option<(f64, f64, f64)>,
    ) {
        let info = {
            let mut players = self.players.write().await;
            let Some(p) = players.get_mut(&entity_id) else {
                return;
            };
            let old_dim = p.dimension;
            if old_dim == target_dim {
                return;
            }
            p.dimension = target_dim;
            p.pending_teleport_id = Some(99);
            if let Some((tx, ty, tz)) = target_pos {
                p.x = tx;
                p.y = ty;
                p.z = tz;
                p.fall_start_y = None;
            }
            (
                p.uuid,
                p.username.clone(),
                p.properties.clone(),
                old_dim,
                p.x,
                p.y,
                p.z,
                p.yaw,
                p.pitch,
                p.game_mode,
                p.tx.clone(),
            )
        };
        let (uuid, username, properties, old_dim, px, py, pz, yaw, pitch, game_mode, tx) = info;
        self.player_dimensions
            .write()
            .await
            .insert(uuid, target_dim);

        let mut batch = BytesMut::with_capacity(4096);
        let _ = encode_packet(
            &Respawn {
                dimension_type: VarInt(target_dim.type_id()),
                dimension_name: target_dim.resource_name().into(),
                hashed_seed: 0,
                game_mode,
                previous_game_mode: -1,
                is_debug: false,
                is_flat: false,
                death_location: None,
                portal_cooldown: VarInt(0),
                sea_level: VarInt(64),
                data_kept: 0x03,
            },
            &mut batch,
        );

        let center_cx = (px.floor() as i32).div_euclid(16);
        let center_cz = (pz.floor() as i32).div_euclid(16);
        let _ = encode_packet(
            &SetChunkCacheCenter {
                chunk_x: VarInt(center_cx),
                chunk_z: VarInt(center_cz),
            },
            &mut batch,
        );
        let _ = encode_packet(
            &SetDefaultSpawnPosition {
                dimension_name: target_dim.resource_name().into(),
                location: BlockPos::new(px as i32, py as i32, pz as i32),
                yaw: 0.0,
                pitch: 0.0,
            },
            &mut batch,
        );
        let radius = self.config.view_distance.clamp(1, 3);
        let coords = crate::world::chunks_in_radius(center_cx, center_cz, radius);
        {
            let mut world = self.world.write().await;
            for (cx, cz) in coords {
                let pkt = world.chunk_packet_in(target_dim, cx, cz);
                let _ = encode_packet(&pkt, &mut batch);
            }
        }
        let _ = encode_packet(
            &SynchronizePlayerPosition {
                teleport_id: VarInt(99),
                x: px,
                y: py,
                z: pz,
                vx: 0.0,
                vy: 0.0,
                vz: 0.0,
                yaw,
                pitch,
                flags: 0,
            },
            &mut batch,
        );
        let _ = tx.try_send(batch.freeze());

        // Update player visibility across old_dim and target_dim
        let mut old_dim_eids = Vec::new();
        let players = self.players.read().await;
        if let Ok(remove_switcher) = encode_to_bytes(&RemoveEntities {
            entity_ids: vec![VarInt(entity_id)],
        }) && let Ok(spawn_switcher) = encode_to_bytes(&rad_core::packet::play::SpawnEntity {
            entity_id: VarInt(entity_id),
            uuid,
            entity_type: VarInt(self.registries.entities.player),
            x: px,
            y: py,
            z: pz,
            velocity: rad_core::types::LpVec3::ZERO,
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: VarInt(0),
        }) {
            let _ = username;
            let _ = properties;
            for (&other_eid, other) in players.iter() {
                if other_eid == entity_id {
                    continue;
                }
                if other.dimension == old_dim {
                    old_dim_eids.push(VarInt(other_eid));
                    let _ = other.tx.try_send(remove_switcher.clone());
                } else if other.dimension == target_dim {
                    let _ = other.tx.try_send(spawn_switcher.clone());
                    if let Ok(spawn_other) = encode_to_bytes(&rad_core::packet::play::SpawnEntity {
                        entity_id: VarInt(other.entity_id),
                        uuid: other.uuid,
                        entity_type: VarInt(self.registries.entities.player),
                        x: other.x,
                        y: other.y,
                        z: other.z,
                        velocity: rad_core::types::LpVec3::ZERO,
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: VarInt(0),
                    }) {
                        let _ = tx.try_send(spawn_other);
                    }
                }
            }
        }
        {
            let entities = self.entities.read().await;
            for mob in entities.mobs_in_dimension(old_dim) {
                old_dim_eids.push(VarInt(mob.entity_id));
            }
            for item in entities.dropped_items_in_dimension(old_dim) {
                old_dim_eids.push(VarInt(item.entity_id));
            }
            for mob in entities.mobs_in_dimension(target_dim) {
                if let Ok(bytes) =
                    encode_to_bytes(&mob.to_spawn_packet_with(&self.registries.entities))
                {
                    let _ = tx.try_send(bytes);
                }
            }
            for item in entities.dropped_items_in_dimension(target_dim) {
                if let Ok(bytes) =
                    encode_to_bytes(&item.to_spawn_packet_with(&self.registries.entities))
                {
                    let _ = tx.try_send(bytes);
                }
                if let Ok(bytes) = encode_to_bytes(&item.to_metadata_packet()) {
                    let _ = tx.try_send(bytes);
                }
            }
        }
        if !old_dim_eids.is_empty()
            && let Ok(bytes) = encode_to_bytes(&RemoveEntities {
                entity_ids: old_dim_eids,
            })
        {
            let _ = tx.try_send(bytes);
        }
    }

    /// Broadcasts a system chat line to all connected players.
    pub async fn broadcast_chat(&self, message: String) {
        self.broadcast_except(
            &SystemChatMessage {
                content: NbtTag::String(message),
                overlay: false,
            },
            None,
        )
        .await;
    }

    /// Loads persisted player data from `<world_dir>/players/<uuid>.dat` (Vanilla 26.3 gzipped NBT),
    /// falling back to legacy `playerdata/<uuid>.dat` or `playerdata/<uuid>.json` if present,
    /// and merges `players/stats/<uuid>.json` when available.
    pub async fn load_player_data(&self, uuid: Uuid) -> Option<SavedPlayerData> {
        let dir = self.world.read().await.world_dir()?.to_path_buf();
        let registries = Arc::clone(&self.registries);
        let players_data_dat = dir.join("players").join("data").join(format!("{uuid}.dat"));
        let players_dat = dir.join("players").join(format!("{uuid}.dat"));
        let legacy_dat = dir.join("playerdata").join(format!("{uuid}.dat"));
        let legacy_json = dir.join("playerdata").join(format!("{uuid}.json"));
        let stats_json = dir
            .join("players")
            .join("stats")
            .join(format!("{uuid}.json"));
        tokio::task::spawn_blocking(move || {
            let mut loaded = None;
            for dat_path in [&players_data_dat, &players_dat, &legacy_dat] {
                if let Ok(compressed) = std::fs::read(dat_path) {
                    use flate2::read::GzDecoder;
                    use std::io::Read;
                    let mut dec = GzDecoder::new(compressed.as_slice()).take(4 * 1024 * 1024);
                    let mut raw = Vec::new();
                    if dec.read_to_end(&mut raw).is_ok() {
                        let mut cursor = raw.as_slice();
                        if let Ok((_, root)) = rad_core::nbt::decode_named_nbt(&mut cursor)
                            && let Some(parsed) = SavedPlayerData::from_nbt(&root, &registries)
                        {
                            loaded = Some(parsed);
                            break;
                        }
                    }
                }
            }
            if loaded.is_none()
                && let Ok(raw) = std::fs::read_to_string(&legacy_json)
            {
                loaded = serde_json::from_str(&raw).ok();
            }
            if let Some(ref mut data) = loaded
                && let Ok(raw_stats) = std::fs::read_to_string(&stats_json)
                && let Ok(parsed_stats) = parse_vanilla_player_stats(&raw_stats)
            {
                data.stats = parsed_stats;
            }
            loaded
        })
        .await
        .ok()
        .flatten()
    }

    /// Atomically saves a single player's state to `<world_dir>/players/<uuid>.dat`
    /// and `<world_dir>/players/stats/<uuid>.json`, serialized via `save_mutex`.
    pub async fn save_player_data(
        &self,
        uuid: Uuid,
        data: &SavedPlayerData,
    ) -> std::io::Result<()> {
        let _save_guard = self.save_mutex.lock().await;
        self.save_player_data_unlocked(uuid, data).await
    }

    async fn save_player_data_unlocked(
        &self,
        uuid: Uuid,
        data: &SavedPlayerData,
    ) -> std::io::Result<()> {
        let Some(dir) = self
            .world
            .read()
            .await
            .world_dir()
            .map(std::path::Path::to_path_buf)
        else {
            return Ok(());
        };
        let nbt_tag = data.to_nbt(&self.registries);
        let mut nbt_bytes = BytesMut::new();
        rad_core::nbt::encode_named_nbt("", &nbt_tag, &mut nbt_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let data_version = rad_version::MC_26_3.world_data_version;
        let stats_json_bytes = format_vanilla_player_stats(&data.stats)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        tokio::task::spawn_blocking(move || {
            use flate2::Compression;
            use flate2::write::GzEncoder;
            use std::io::Write;
            let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
            enc.write_all(&nbt_bytes)?;
            let gz = enc.finish()?;

            let players_dir = dir.join("players");
            let dat_dir = if players_dir.join("data").exists()
                || dir.join("dimensions").join("minecraft").exists()
            {
                players_dir.join("data")
            } else {
                players_dir.clone()
            };
            std::fs::create_dir_all(&dat_dir)?;
            let tmp_path = dat_dir.join(format!("{uuid}.dat.{}", unique_tmp_suffix()));
            let final_path = dat_dir.join(format!("{uuid}.dat"));
            let mut file = std::fs::File::create(&tmp_path)?;
            file.write_all(&gz)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp_path, &final_path)?;

            let stats_dir = players_dir.join("stats");
            std::fs::create_dir_all(&stats_dir)?;
            let stats_tmp = stats_dir.join(format!("{uuid}.json.{}", unique_tmp_suffix()));
            let stats_final = stats_dir.join(format!("{uuid}.json"));
            std::fs::write(&stats_tmp, stats_json_bytes)?;
            std::fs::rename(&stats_tmp, &stats_final)?;

            let adv_dir = players_dir.join("advancements");
            std::fs::create_dir_all(&adv_dir)?;
            let adv_final = adv_dir.join(format!("{uuid}.json"));
            if !adv_final.exists() {
                let adv_tmp = adv_dir.join(format!("{uuid}.json.{}", unique_tmp_suffix()));
                let adv_json = format!("{{\"DataVersion\":{data_version}}}");
                std::fs::write(&adv_tmp, adv_json)?;
                std::fs::rename(&adv_tmp, &adv_final)?;
            }

            for legacy in [
                dir.join("playerdata").join(format!("{uuid}.dat")),
                dir.join("playerdata").join(format!("{uuid}.json")),
            ] {
                if legacy.exists() {
                    let _ = std::fs::remove_file(&legacy);
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| std::io::Error::other(format!("playerdata save task failed: {e}")))?
    }

    /// Disconnects all active player sessions cleanly (used on coordinated shutdown).
    pub async fn disconnect_all(&self, reason: &str) {
        let disconnect_bytes = encode_to_bytes(&PlayDisconnect {
            reason: NbtTag::String(reason.to_owned()),
        })
        .ok();
        let players = self.players.read().await;
        for p in players.values() {
            if let Some(ref bytes) = disconnect_bytes {
                let _ = p.tx.try_send(bytes.clone());
            }
            p.tx.disconnect();
        }
    }

    /// Persists all dirty chunks (async I/O with revision guard) and all online players' state.
    pub async fn save_all(&self) -> std::io::Result<usize> {
        let _save_guard = self.save_mutex.lock().await;
        let snapshots: Vec<(Uuid, SavedPlayerData)> = {
            let dims = self.player_dimensions.read().await;
            let stats_map = self.player_stats.read().await;
            let players = self.players.read().await;
            players
                .values()
                .map(|p| {
                    (
                        p.uuid,
                        SavedPlayerData {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                            yaw: p.yaw,
                            pitch: p.pitch,
                            hp: p.hp,
                            food: p.food,
                            saturation: p.saturation,
                            game_mode: p.game_mode,
                            inventory: p.inventory.clone(),
                            selected_hotbar: p.selected_hotbar,
                            dimension: dims.get(&p.uuid).copied().unwrap_or_default(),
                            stats: stats_map.get(&p.uuid).copied().unwrap_or_default(),
                        },
                    )
                })
                .collect()
        };
        for (uuid, data) in snapshots {
            self.save_player_data_unlocked(uuid, &data).await?;
        }

        let c_rev = {
            let containers = self.containers.read().await;
            containers.snapshot_if_dirty()?.map(|(rev, _)| rev)
        };
        let f_rev = {
            let furnaces = self.furnaces.read().await;
            furnaces.snapshot_if_dirty().map(|(rev, _)| rev)
        };
        let cur_tick = self.tick_count.load(Ordering::Relaxed);
        let s_rev = {
            let sim = self.world_sim.read().await;
            sim.snapshot_if_dirty(cur_tick).map(|(rev, _)| rev)
        };
        if c_rev.is_some() || f_rev.is_some() || s_rev.is_some() {
            let containers = self.containers.read().await;
            let furnaces = self.furnaces.read().await;
            let sim = self.world_sim.read().await;
            let mut world = self.world.write().await;
            if c_rev.is_some() || f_rev.is_some() {
                sync_block_entities_to_world(&mut world, &containers, &furnaces, &self.registries);
            }
            if s_rev.is_some() {
                sync_block_ticks_to_world(&mut world, &sim, &self.registries, cur_tick);
            }
        }

        let (world_dir, dirty) = {
            let world = self.world.read().await;
            let Some(dir) = world.world_dir().map(std::path::Path::to_path_buf) else {
                return Ok(0);
            };
            (dir, world.snapshot_dirty_chunks()?)
        };

        if let Some(rev) = c_rev {
            self.containers.write().await.mark_saved(rev);
            let legacy_containers = world_dir.join("containers.json");
            if legacy_containers.exists() {
                let _ = std::fs::remove_file(legacy_containers);
            }
        }

        let access_snap = {
            let access = self.access.read().await;
            access.snapshot_if_dirty()
        };
        if let Some((a_rev, access_snap)) = access_snap {
            let adir = world_dir.clone();
            tokio::task::spawn_blocking(move || {
                AccessControl::write_snapshot_atomic(&adir, &access_snap)
            })
            .await
            .map_err(|e| std::io::Error::other(format!("access save task failed: {e}")))??;
            self.access.write().await.mark_saved(a_rev);
        }

        let _ = self.sync_level_time().await;
        let level_snap = self.level_meta.read().await.clone();
        let ldir = world_dir.clone();
        tokio::task::spawn_blocking(move || level_snap.write_atomic(&ldir))
            .await
            .map_err(|e| std::io::Error::other(format!("level.dat save task failed: {e}")))??;

        if let Some(rev) = f_rev {
            self.furnaces.write().await.mark_saved(rev);
            let legacy_furnaces = world_dir.join("furnaces.json");
            if legacy_furnaces.exists() {
                let _ = std::fs::remove_file(legacy_furnaces);
            }
        }

        if let Some(rev) = s_rev {
            self.world_sim.write().await.mark_saved(rev);
            let legacy_sim = world_dir.join("sim_queue.json");
            if legacy_sim.exists() {
                let _ = std::fs::remove_file(legacy_sim);
            }
        }

        let (item_snap, mob_snap): (
            Vec<crate::gameplay::entity::DroppedItemEntity>,
            Vec<crate::gameplay::entity::MobEntity>,
        ) = {
            let entities = self.entities.read().await;
            (
                entities.all_dropped_items().cloned().collect(),
                entities.all_mobs().cloned().collect(),
            )
        };
        let idir = world_dir.clone();
        let regs = Arc::clone(&self.registries);
        tokio::task::spawn_blocking(move || {
            crate::gameplay::entity::write_entities_to_world_mca(
                &idir, &item_snap, &mob_snap, &regs,
            )
        })
        .await
        .map_err(|e| std::io::Error::other(format!("entities save task failed: {e}")))??;

        if dirty.is_empty() {
            return Ok(0);
        }

        let dir_clone = world_dir.clone();
        let saved_revs = tokio::task::spawn_blocking(
            move || -> std::io::Result<Vec<(crate::world::worldgen::DimensionId, i32, i32, u64)>> {
                let mut done = Vec::with_capacity(dirty.len());
                for (dim, cx, cz, rev, bytes) in dirty {
                    let dim_dir = crate::world::dimension_dir_for(&dir_clone, dim);
                    crate::world::anvil::AnvilRegion::write_chunk_payload(
                        &dim_dir, cx, cz, &bytes,
                    )?;
                    done.push((dim, cx, cz, rev));
                }
                Ok(done)
            },
        )
            .await
            .map_err(|e| std::io::Error::other(format!("chunk save task failed: {e}")))??;

        let count = saved_revs.len();
        self.world.write().await.mark_chunks_saved(&saved_revs);
        Ok(count)
    }

    /// Returns whether the server trusts forwarded UUIDs from an upstream proxy.
    #[must_use]
    pub const fn trusts_forwarded_identity(&self) -> bool {
        !matches!(self.config.forwarding_mode, ServerForwardingMode::None)
    }

    /// Returns the effective operator permission level (`0` = normal, `1..=4` = op) for a player.
    pub async fn permission_level(&self, username: &str, uuid: Option<Uuid>) -> u8 {
        let key = username.to_ascii_lowercase();
        if self.config.admins.contains(&key) {
            return 4;
        }
        let access = self.access.read().await;
        access.op_level_for_identity(username, uuid, self.trusts_forwarded_identity())
    }

    /// Returns `true` if `username` is a configured admin or runtime `/op` (`level >= 1`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn is_admin(&self, username: &str) -> bool {
        self.permission_level(username, None).await > 0
    }

    /// Disconnects any online non-op players not on the whitelist when `enforce_whitelist` is enabled.
    pub async fn enforce_whitelist_online(&self) -> usize {
        if !self.config.enforce_whitelist {
            return 0;
        }
        let trusted = self.trusts_forwarded_identity();
        let to_kick: Vec<(i32, OutboundSender)> = {
            let access = self.access.read().await;
            if !access.whitelist_enabled {
                return 0;
            }
            let players = self.players.read().await;
            players
                .iter()
                .filter_map(|(&eid, p)| {
                    let key = p.username.to_ascii_lowercase();
                    let is_op = self.config.admins.contains(&key)
                        || access.op_level_for_identity(&p.username, Some(p.uuid), trusted) > 0;
                    if !access.is_allowed_by_whitelist_identity(
                        &p.username,
                        Some(p.uuid),
                        is_op,
                        trusted,
                    ) {
                        Some((eid, p.tx.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };
        let count = to_kick.len();
        if count > 0 {
            let json_reason = TextComponent::text("You are not white-listed on this server!")
                .to_json()
                .unwrap_or_else(|_| {
                    r#"{"text":"You are not white-listed on this server!"}"#.to_owned()
                });
            let disconnect_bytes = encode_to_bytes(&PlayDisconnect {
                reason: NbtTag::String(json_reason),
            })
            .ok();
            let mut players = self.players.write().await;
            for (eid, tx) in to_kick {
                players.remove(&eid);
                if let Some(ref pkt) = disconnect_bytes {
                    let _ = tx.try_send(pkt.clone());
                }
                tx.disconnect();
            }
        }
        count
    }
}

static TMP_WRITE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Returns a collision-free temporary file suffix (`<pid>.<seq>.tmp`) for atomic writes.
pub(crate) fn unique_tmp_suffix() -> String {
    let seq = TMP_WRITE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}.{seq}.tmp", std::process::id())
}

/// Per-player tracked statistics (M10).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerStats {
    /// Total blocks broken.
    pub blocks_broken: u64,
    /// Total blocks placed.
    pub blocks_placed: u64,
    /// Total mobs killed.
    pub mobs_killed: u64,
}

/// Serializable inventory slot persisted in [`SavedPlayerData`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SavedSlot {
    /// Stack count (`0` for empty).
    pub count: i32,
    /// Protocol item ID (`0` for empty).
    pub item_id: i32,
    /// Optional item data components patch.
    #[serde(
        default,
        skip_serializing_if = "rad_core::packet::play::ItemComponentPatch::is_empty"
    )]
    pub components: rad_core::packet::play::ItemComponentPatch,
}

/// Serializable per-player state persisted under `world/playerdata/<uuid>.dat`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedPlayerData {
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Head yaw in degrees.
    pub yaw: f32,
    /// Head pitch in degrees.
    pub pitch: f32,
    /// Current health.
    pub hp: f32,
    /// Current food level.
    pub food: i32,
    /// Current food saturation.
    pub saturation: f32,
    /// Active game mode.
    pub game_mode: u8,
    /// Persisted inventory slots (`46` entries).
    #[serde(default)]
    pub inventory: Vec<SavedSlot>,
    /// Selected hotbar index (`0..=8`).
    #[serde(default)]
    pub selected_hotbar: u8,
    /// Active dimension for this player.
    #[serde(default)]
    pub dimension: crate::world::worldgen::DimensionId,
    /// Tracked gameplay statistics for this player.
    #[serde(default)]
    pub stats: PlayerStats,
}

impl SavedPlayerData {
    /// Encodes player state into Vanilla-compatible NBT (`playerdata/<uuid>.dat`).
    #[must_use]
    pub fn to_nbt(&self, registries: &crate::registries::Version26_3Registries) -> NbtTag {
        let mut root = std::collections::BTreeMap::new();
        root.insert("DataVersion".to_owned(), NbtTag::Int(4790));
        root.insert(
            "Pos".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(self.x),
                NbtTag::Double(self.y),
                NbtTag::Double(self.z),
            ]),
        );
        root.insert(
            "Rotation".to_owned(),
            NbtTag::List(vec![NbtTag::Float(self.yaw), NbtTag::Float(self.pitch)]),
        );
        root.insert("Health".to_owned(), NbtTag::Float(self.hp));
        root.insert("foodLevel".to_owned(), NbtTag::Int(self.food));
        root.insert(
            "foodSaturationLevel".to_owned(),
            NbtTag::Float(self.saturation),
        );
        root.insert(
            "playerGameType".to_owned(),
            NbtTag::Int(i32::from(self.game_mode)),
        );
        root.insert(
            "SelectedItemSlot".to_owned(),
            NbtTag::Int(i32::from(self.selected_hotbar)),
        );
        root.insert(
            "Dimension".to_owned(),
            NbtTag::String(self.dimension.resource_name().to_owned()),
        );

        let inv_list: Vec<NbtTag> = self
            .inventory
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.count > 0 && slot.item_id > 0)
            .filter_map(|(idx, slot)| {
                let vanilla_slot: i8 = match idx {
                    36..=44 => (idx - 36) as i8,
                    9..=35 => idx as i8,
                    5 => 103,
                    6 => 102,
                    7 => 101,
                    8 => 100,
                    45 => -106,
                    _ => return None,
                };
                let name = registries.item_name_by_id(slot.item_id)?;
                let mut item = std::collections::BTreeMap::new();
                item.insert("Slot".to_owned(), NbtTag::Byte(vanilla_slot));
                item.insert("id".to_owned(), NbtTag::String(name.to_owned()));
                item.insert("count".to_owned(), NbtTag::Int(slot.count.clamp(1, 64)));
                if let Some(comp_tag) = encode_item_components_nbt(&slot.components) {
                    item.insert("components".to_owned(), comp_tag);
                }
                Some(NbtTag::Compound(item))
            })
            .collect();
        root.insert("Inventory".to_owned(), NbtTag::List(inv_list));

        let mut stats_tag = std::collections::BTreeMap::new();
        stats_tag.insert(
            "blocks_broken".to_owned(),
            NbtTag::Long(self.stats.blocks_broken as i64),
        );
        stats_tag.insert(
            "blocks_placed".to_owned(),
            NbtTag::Long(self.stats.blocks_placed as i64),
        );
        stats_tag.insert(
            "mobs_killed".to_owned(),
            NbtTag::Long(self.stats.mobs_killed as i64),
        );
        root.insert("rad_stats".to_owned(), NbtTag::Compound(stats_tag));

        NbtTag::Compound(root)
    }

    /// Decodes player state from Vanilla-compatible NBT (`playerdata/<uuid>.dat`).
    #[must_use]
    pub fn from_nbt(
        tag: &NbtTag,
        registries: &crate::registries::Version26_3Registries,
    ) -> Option<Self> {
        let NbtTag::Compound(root) = tag else {
            return None;
        };
        let (x, y, z) = match root.get("Pos") {
            Some(NbtTag::List(vals)) if vals.len() == 3 => match (&vals[0], &vals[1], &vals[2]) {
                (NbtTag::Double(x), NbtTag::Double(y), NbtTag::Double(z)) => (*x, *y, *z),
                _ => return None,
            },
            _ => return None,
        };
        let (yaw, pitch) = match root.get("Rotation") {
            Some(NbtTag::List(vals)) if vals.len() == 2 => match (&vals[0], &vals[1]) {
                (NbtTag::Float(yaw), NbtTag::Float(pitch)) => (*yaw, *pitch),
                _ => (0.0, 0.0),
            },
            _ => (0.0, 0.0),
        };
        let hp = match root.get("Health") {
            Some(NbtTag::Float(v)) => *v,
            _ => 20.0,
        };
        let food = match root.get("foodLevel") {
            Some(NbtTag::Int(v)) => *v,
            _ => 20,
        };
        let saturation = match root.get("foodSaturationLevel") {
            Some(NbtTag::Float(v)) => *v,
            _ => 5.0,
        };
        let game_mode = match root.get("playerGameType") {
            Some(NbtTag::Int(v)) => (*v).clamp(0, 3) as u8,
            _ => 0,
        };
        let selected_hotbar = match root.get("SelectedItemSlot") {
            Some(NbtTag::Int(v)) => (*v).clamp(0, 8) as u8,
            _ => 0,
        };
        let dimension = match root.get("Dimension") {
            Some(NbtTag::String(s)) if s == "minecraft:the_nether" => {
                crate::world::worldgen::DimensionId::Nether
            }
            Some(NbtTag::String(s)) if s == "minecraft:the_end" => {
                crate::world::worldgen::DimensionId::End
            }
            _ => crate::world::worldgen::DimensionId::Overworld,
        };

        let mut inventory =
            vec![SavedSlot::default(); crate::gameplay::inventory::PLAYER_INVENTORY_SLOTS];
        if let Some(NbtTag::List(items)) = root.get("Inventory") {
            for entry in items {
                let NbtTag::Compound(map) = entry else {
                    continue;
                };
                let Some(NbtTag::Byte(slot_idx)) = map.get("Slot") else {
                    continue;
                };
                let idx = match *slot_idx {
                    0..=8 => 36 + (*slot_idx as usize),
                    9..=35 => *slot_idx as usize,
                    103 => 5,
                    102 => 6,
                    101 => 7,
                    100 => 8,
                    -106 => 45,
                    _ => continue,
                };
                if idx >= inventory.len() {
                    continue;
                }
                let Some(NbtTag::String(id_name)) = map.get("id") else {
                    continue;
                };
                let Some(item_id) = registries.item_by_name(id_name) else {
                    continue;
                };
                let count = match map.get("count").or_else(|| map.get("Count")) {
                    Some(NbtTag::Int(c)) => (*c).clamp(1, 64),
                    Some(NbtTag::Byte(c)) => i32::from(*c).clamp(1, 64),
                    _ => 1,
                };
                let components = decode_item_components_nbt(map);
                inventory[idx] = SavedSlot {
                    count,
                    item_id,
                    components,
                };
            }
        }

        let stats = match root.get("rad_stats") {
            Some(NbtTag::Compound(st)) => PlayerStats {
                blocks_broken: match st.get("blocks_broken") {
                    Some(NbtTag::Long(v)) => (*v).max(0) as u64,
                    _ => 0,
                },
                blocks_placed: match st.get("blocks_placed") {
                    Some(NbtTag::Long(v)) => (*v).max(0) as u64,
                    _ => 0,
                },
                mobs_killed: match st.get("mobs_killed") {
                    Some(NbtTag::Long(v)) => (*v).max(0) as u64,
                    _ => 0,
                },
            },
            _ => PlayerStats::default(),
        };

        Some(Self {
            x,
            y,
            z,
            yaw,
            pitch,
            hp,
            food,
            saturation,
            game_mode,
            inventory,
            selected_hotbar,
            dimension,
            stats,
        })
    }
}

fn format_vanilla_player_stats(stats: &PlayerStats) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "stats": {
            "minecraft:custom": {
                "radbyte:blocks_broken": stats.blocks_broken,
                "radbyte:blocks_placed": stats.blocks_placed,
                "radbyte:mobs_killed": stats.mobs_killed
            }
        },
        "DataVersion": 4790
    }))
}

fn parse_vanilla_player_stats(raw: &str) -> serde_json::Result<PlayerStats> {
    let val: serde_json::Value = serde_json::from_str(raw)?;
    let custom = val
        .get("stats")
        .and_then(|s| s.get("minecraft:custom"))
        .and_then(|c| c.as_object());
    Ok(PlayerStats {
        blocks_broken: custom
            .and_then(|c| c.get("radbyte:blocks_broken"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        blocks_placed: custom
            .and_then(|c| c.get("radbyte:blocks_placed"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        mobs_killed: custom
            .and_then(|c| c.get("radbyte:mobs_killed"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
    })
}

pub(super) fn decode_client_view_distance(payload: &[u8]) -> Option<i32> {
    use rad_core::types::Decode as _;
    let mut slice = payload;
    let _locale = String::decode(&mut slice).ok()?;
    let vd = u8::decode(&mut slice).ok()? as i8;
    Some(i32::from(vd).clamp(2, 32))
}

/// Handles a single accepted TCP connection to `RadServer`.
pub async fn handle_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    state: Arc<ServerState>,
) {
    let _ = stream.set_nodelay(true);
    let mut read_buf = BytesMut::with_capacity(4096);
    let max_packet = state.config.max_packet_size.max(DEFAULT_MAX_PACKET_SIZE);

    let hs = match timeout(
        Duration::from_secs(5),
        read_typed_packet::<Handshake>(&mut stream, &mut read_buf, max_packet),
    )
    .await
    {
        Ok(Some(hs)) => hs,
        _ => return,
    };

    match hs.next_state {
        ConnectionState::Status => handle_status(&mut stream, &mut read_buf, &state).await,
        ConnectionState::Login => {
            handle_login_and_play(stream, read_buf, peer_addr, &hs, state).await;
        }
        ConnectionState::Handshaking | ConnectionState::Configuration | ConnectionState::Play => {}
    }
}

async fn handle_status(stream: &mut TcpStream, read_buf: &mut BytesMut, state: &ServerState) {
    if !state.config.enable_status {
        return;
    }
    let max_packet = state.config.max_packet_size;
    if timeout(
        Duration::from_secs(5),
        read_typed_packet::<StatusRequest>(stream, read_buf, max_packet),
    )
    .await
    .ok()
    .flatten()
    .is_none()
    {
        return;
    }

    let online = if state.config.hide_online_players {
        0
    } else {
        state.players.read().await.len()
    };
    let payload = json!({
        "version": {
            "name": state.version_package.game_version(),
            "protocol": state.version_package.protocol_version()
        },
        "players": { "max": state.config.max_players, "online": online },
        "description": { "text": state.config.motd },
    })
    .to_string();

    if send_packet(stream, &StatusResponse { json: payload })
        .await
        .is_err()
    {
        return;
    }

    if let Ok(Some(ping)) = timeout(
        Duration::from_secs(5),
        read_typed_packet::<PingRequest>(stream, read_buf, max_packet),
    )
    .await
    {
        let _ = send_packet(
            stream,
            &PongResponse {
                timestamp: ping.timestamp,
            },
        )
        .await;
    }
}

pub(super) fn send_chat_to(tx: &OutboundSender, text: String) {
    if let Ok(bytes) = encode_to_bytes(&SystemChatMessage {
        content: NbtTag::String(text),
        overlay: false,
    }) {
        let _ = tx.try_send(bytes);
    }
}

/// Minecraft's network text components are NBT compounds, not JSON strings.
fn styled_text(text: impl Into<String>, color: &str) -> NbtTag {
    NbtTag::Compound(std::collections::BTreeMap::from([
        ("text".into(), NbtTag::String(text.into())),
        ("color".into(), NbtTag::String(color.into())),
    ]))
}

fn translated_text(key: &str, argument: NbtTag, color: &str) -> NbtTag {
    NbtTag::Compound(std::collections::BTreeMap::from([
        ("translate".into(), NbtTag::String(key.into())),
        ("with".into(), NbtTag::List(vec![argument])),
        ("color".into(), NbtTag::String(color.into())),
    ]))
}

fn gamemode_notice(mode: u8) -> Option<NbtTag> {
    let key = match mode {
        0 => "gameMode.survival",
        1 => "gameMode.creative",
        2 => "gameMode.adventure",
        3 => "gameMode.spectator",
        _ => return None,
    };
    let name = NbtTag::Compound(std::collections::BTreeMap::from([(
        "translate".into(),
        NbtTag::String(key.into()),
    )]));
    Some(translated_text("gameMode.changed", name, "gray"))
}

/// Sends a single outstanding `KeepAliveClientbound` per idle player and disconnects timed-out peers.
pub(super) async fn step_keep_alive(state: &ServerState, ka_seq: i64, timeout: Duration) {
    let now = Instant::now();
    let Ok(bytes) = encode_to_bytes(&KeepAliveClientbound { id: ka_seq }) else {
        return;
    };
    let mut players = state.players.write().await;
    let mut timed_out = Vec::new();
    for (&eid, p) in players.iter_mut() {
        if let Some((_, sent_at)) = p.pending_keep_alive {
            if now.duration_since(sent_at) > timeout {
                timed_out.push(eid);
            }
            continue;
        }
        p.pending_keep_alive = Some((ka_seq, now));
        let _ = p.tx.try_send(bytes.clone());
    }
    for eid in timed_out {
        if let Some(p) = players.remove(&eid) {
            info!(
                entity_id = eid,
                username = %p.username,
                reason = "keep_alive_timeout",
                "disconnecting timed-out player"
            );
            p.tx.disconnect();
        }
    }
}

/// Disconnects players whose `last_activity` exceeds `timeout`.
pub(super) async fn step_idle_timeouts(state: &ServerState, timeout: Duration) {
    let now = Instant::now();
    let mut players = state.players.write().await;
    let timed_out: Vec<i32> = players
        .iter()
        .filter(|(_, p)| now.duration_since(p.last_activity) >= timeout)
        .map(|(&eid, _)| eid)
        .collect();
    if timed_out.is_empty() {
        return;
    }
    let json_reason = TextComponent::text("You have been idle for too long!")
        .to_json()
        .unwrap_or_else(|_| r#"{"text":"You have been idle for too long!"}"#.to_owned());
    let disconnect_bytes = encode_to_bytes(&PlayDisconnect {
        reason: NbtTag::String(json_reason),
    })
    .ok();
    for eid in timed_out {
        if let Some(p) = players.remove(&eid) {
            info!(
                entity_id = eid,
                username = %p.username,
                reason = "idle_timeout",
                "disconnecting idle player"
            );
            if let Some(ref pkt) = disconnect_bytes {
                let _ = p.tx.try_send(pkt.clone());
            }
            p.tx.disconnect();
        }
    }
}

pub(super) fn offset_block_pos(pos: BlockPos, face: i32) -> BlockPos {
    match face {
        0 => BlockPos::new(pos.x, pos.y - 1, pos.z),
        1 => BlockPos::new(pos.x, pos.y + 1, pos.z),
        2 => BlockPos::new(pos.x, pos.y, pos.z - 1),
        3 => BlockPos::new(pos.x, pos.y, pos.z + 1),
        4 => BlockPos::new(pos.x - 1, pos.y, pos.z),
        5 => BlockPos::new(pos.x + 1, pos.y, pos.z),
        _ => pos,
    }
}

pub(super) fn deg_to_angle(deg: f32) -> i8 {
    ((deg.rem_euclid(360.0) / 360.0) * 256.0).round() as i32 as i8
}

pub(super) fn write_bungee_utf(buf: &mut BytesMut, s: &str) {
    let bytes = s.as_bytes();
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    buf.put_u16(len);
    buf.put_slice(&bytes[..usize::from(len)]);
}

pub(super) fn encode_to_bytes<P: Packet>(pkt: &P) -> rad_core::error::Result<Bytes> {
    let mut out = BytesMut::new();
    encode_packet(pkt, &mut out)?;
    Ok(out.freeze())
}

pub(super) async fn send_packet<P: Packet>(stream: &mut TcpStream, pkt: &P) -> std::io::Result<()> {
    let mut out = BytesMut::new();
    encode_packet(pkt, &mut out)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    stream.write_all(&out).await
}

pub(super) async fn read_raw_frame<R: AsyncReadExt + Unpin>(
    stream: &mut R,
    buf: &mut BytesMut,
    max_packet: usize,
) -> Option<rad_core::codec::RawPacket> {
    let max_buf = max_packet.saturating_add(5);
    loop {
        match decode_frame(buf, max_packet) {
            Ok(Some(frame)) => return Some(frame),
            Ok(None) => {}
            Err(_) => return None,
        }
        if buf.len() >= max_buf {
            return None;
        }
        match stream.read_buf(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

pub(super) async fn read_typed_packet<P: Packet>(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
    max_packet: usize,
) -> Option<P> {
    let frame = read_raw_frame(stream, buf, max_packet).await?;
    frame.decode_as::<P>().ok()
}

fn nbt_item_count(tag: &NbtTag) -> i32 {
    match tag {
        NbtTag::Int(v) => *v,
        NbtTag::Byte(v) => i32::from(*v),
        NbtTag::Short(v) => i32::from(*v),
        _ => 0,
    }
}

fn nbt_short_u16(tag: Option<&NbtTag>) -> u16 {
    match tag {
        Some(NbtTag::Short(v)) => (*v).max(0) as u16,
        Some(NbtTag::Int(v)) => (*v).clamp(0, i32::from(u16::MAX)) as u16,
        _ => 0,
    }
}

pub(crate) fn encode_item_components_nbt(
    comp: &rad_core::packet::play::ItemComponentPatch,
) -> Option<NbtTag> {
    if comp.is_empty() {
        return None;
    }
    let mut map = std::collections::BTreeMap::new();
    if let Some(ref name) = comp.custom_name {
        map.insert(
            "minecraft:custom_name".to_owned(),
            NbtTag::String(name.clone()),
        );
    }
    if let Some(dmg) = comp.damage {
        map.insert("minecraft:damage".to_owned(), NbtTag::Int(dmg));
    }
    if let Some(max_dmg) = comp.max_damage {
        map.insert("minecraft:max_damage".to_owned(), NbtTag::Int(max_dmg));
    }
    if let Some(cost) = comp.repair_cost {
        map.insert("minecraft:repair_cost".to_owned(), NbtTag::Int(cost));
    }
    if comp.unbreakable {
        map.insert(
            "minecraft:unbreakable".to_owned(),
            NbtTag::Compound(std::collections::BTreeMap::new()),
        );
    }
    if !comp.removed.is_empty() {
        map.insert(
            "rad_removed".to_owned(),
            NbtTag::List(comp.removed.iter().map(|id| NbtTag::Int(*id)).collect()),
        );
    }
    Some(NbtTag::Compound(map))
}

pub(crate) fn decode_item_components_nbt(
    item_map: &std::collections::BTreeMap<String, NbtTag>,
) -> rad_core::packet::play::ItemComponentPatch {
    if let Some(NbtTag::Compound(comp_map)) = item_map.get("components") {
        let mut patch = rad_core::packet::play::ItemComponentPatch::default();
        if let Some(NbtTag::String(name)) = comp_map.get("minecraft:custom_name") {
            patch.custom_name = Some(name.clone());
        }
        if let Some(NbtTag::Int(dmg)) = comp_map.get("minecraft:damage") {
            patch.damage = Some(*dmg);
        }
        if let Some(NbtTag::Int(max_dmg)) = comp_map.get("minecraft:max_damage") {
            patch.max_damage = Some(*max_dmg);
        }
        if let Some(NbtTag::Int(cost)) = comp_map.get("minecraft:repair_cost") {
            patch.repair_cost = Some(*cost);
        }
        if comp_map.contains_key("minecraft:unbreakable") {
            patch.unbreakable = true;
        }
        if let Some(NbtTag::List(removed)) = comp_map.get("rad_removed") {
            for tag in removed {
                if let NbtTag::Int(id) = tag {
                    patch.removed.push(*id);
                }
            }
        }
        return patch;
    }
    match item_map.get("rad_components") {
        Some(NbtTag::String(raw)) => serde_json::from_str(raw).unwrap_or_default(),
        _ => Default::default(),
    }
}

fn encode_nbt_slot_item(
    slot_idx: u8,
    item_id: i32,
    count: i32,
    components: Option<&rad_core::packet::play::ItemComponentPatch>,
    regs: &Version26_3Registries,
) -> Option<NbtTag> {
    if count <= 0 || item_id <= 0 {
        return None;
    }
    let name = regs.item_name_by_id(item_id)?;
    let mut item = std::collections::BTreeMap::new();
    item.insert("Slot".to_string(), NbtTag::Byte(slot_idx as i8));
    item.insert("id".to_string(), NbtTag::String(name.to_string()));
    item.insert("count".to_string(), NbtTag::Int(count.clamp(1, 64)));
    if let Some(comp) = components
        && let Some(comp_tag) = encode_item_components_nbt(comp)
    {
        item.insert("components".to_string(), comp_tag);
    }
    Some(NbtTag::Compound(item))
}

#[cfg(test)]
fn load_block_entities_into_stores(
    world_dir: &std::path::Path,
    regs: &Version26_3Registries,
    containers: &mut crate::world::containers::ContainerStore,
    furnaces: &mut crate::world::sim::FurnaceStore,
) {
    let loaded = crate::world::WorldState::load_all_block_entities_from_mca(
        world_dir,
        regs.provider().as_ref(),
    );
    apply_block_entities_to_stores(loaded, regs, containers, furnaces);
}

fn apply_block_entities_to_stores(
    loaded: Vec<(
        crate::world::worldgen::DimensionId,
        std::collections::BTreeMap<String, NbtTag>,
    )>,
    regs: &Version26_3Registries,
    containers: &mut crate::world::containers::ContainerStore,
    furnaces: &mut crate::world::sim::FurnaceStore,
) {
    for (dim, map) in loaded {
        let Some(NbtTag::String(id)) = map.get("id") else {
            continue;
        };
        let (Some(NbtTag::Int(x)), Some(NbtTag::Int(y)), Some(NbtTag::Int(z))) =
            (map.get("x"), map.get("y"), map.get("z"))
        else {
            continue;
        };
        if id == "minecraft:chest" {
            let mut slots = vec![
                rad_core::packet::play::SimpleSlot::default();
                crate::world::containers::CHEST_SLOTS
            ];
            let mut comps = vec![
                rad_core::packet::play::ItemComponentPatch::default();
                crate::world::containers::CHEST_SLOTS
            ];
            if let Some(NbtTag::List(items)) = map.get("Items") {
                for entry in items {
                    let NbtTag::Compound(item) = entry else {
                        continue;
                    };
                    let Some(NbtTag::Byte(slot_byte)) = item.get("Slot") else {
                        continue;
                    };
                    let Ok(slot_idx) = usize::try_from(*slot_byte) else {
                        continue;
                    };
                    if slot_idx >= crate::world::containers::CHEST_SLOTS {
                        continue;
                    }
                    let Some(NbtTag::String(item_name)) = item.get("id") else {
                        continue;
                    };
                    let Some(item_id) = regs.item_by_name(item_name) else {
                        continue;
                    };
                    let count = item.get("count").map_or(0, nbt_item_count).clamp(0, 64);
                    if count > 0 {
                        slots[slot_idx] = rad_core::packet::play::SimpleSlot { count, item_id };
                        comps[slot_idx] = decode_item_components_nbt(item);
                    }
                }
            }
            containers.set_chest_slots_in(dim, *x, *y, *z, slots, comps);
        } else if id == "minecraft:furnace" {
            let mut furnace = crate::world::sim::FurnaceEntity {
                burn_time: nbt_short_u16(map.get("BurnTime")),
                cook_progress: nbt_short_u16(map.get("CookTime")),
                ..Default::default()
            };
            if let Some(NbtTag::List(items)) = map.get("Items") {
                for entry in items {
                    let NbtTag::Compound(item) = entry else {
                        continue;
                    };
                    let Some(NbtTag::Byte(slot_byte)) = item.get("Slot") else {
                        continue;
                    };
                    let Some(NbtTag::String(item_name)) = item.get("id") else {
                        continue;
                    };
                    let Some(item_id) = regs.item_by_name(item_name) else {
                        continue;
                    };
                    let count = item.get("count").map_or(0, nbt_item_count).clamp(0, 64);
                    if count <= 0 {
                        continue;
                    }
                    match *slot_byte {
                        0 => {
                            furnace.input_item = item_id;
                            furnace.input_count = count;
                        }
                        1 => {
                            furnace.fuel_item = item_id;
                            furnace.fuel_count = count;
                        }
                        2 => {
                            furnace.output_item = item_id;
                            furnace.output_count = count;
                        }
                        _ => {}
                    }
                }
            }
            furnaces.insert_in(dim, *x, *y, *z, furnace);
        }
    }
}

fn sync_block_entities_to_world(
    world: &mut crate::world::WorldState,
    containers: &crate::world::containers::ContainerStore,
    furnaces: &crate::world::sim::FurnaceStore,
    regs: &Version26_3Registries,
) {
    let mut by_chunk: HashMap<
        (crate::world::worldgen::DimensionId, i32, i32),
        Vec<std::collections::BTreeMap<String, NbtTag>>,
    > = HashMap::new();

    for (dim, x, y, z, slots, comps) in containers.all_chests() {
        let mut be = std::collections::BTreeMap::new();
        be.insert(
            "id".to_string(),
            NbtTag::String("minecraft:chest".to_string()),
        );
        be.insert("x".to_string(), NbtTag::Int(x));
        be.insert("y".to_string(), NbtTag::Int(y));
        be.insert("z".to_string(), NbtTag::Int(z));
        let mut items = Vec::new();
        for (i, slot) in slots.iter().enumerate() {
            if let Ok(idx) = u8::try_from(i)
                && let Some(tag) =
                    encode_nbt_slot_item(idx, slot.item_id, slot.count, comps.get(i), regs)
            {
                items.push(tag);
            }
        }
        be.insert("Items".to_string(), NbtTag::List(items));
        by_chunk
            .entry((dim, x.div_euclid(16), z.div_euclid(16)))
            .or_default()
            .push(be);
    }

    for (dim, x, y, z, f) in furnaces.all_furnaces() {
        let mut be = std::collections::BTreeMap::new();
        be.insert(
            "id".to_string(),
            NbtTag::String("minecraft:furnace".to_string()),
        );
        be.insert("x".to_string(), NbtTag::Int(x));
        be.insert("y".to_string(), NbtTag::Int(y));
        be.insert("z".to_string(), NbtTag::Int(z));
        be.insert("BurnTime".to_string(), NbtTag::Short(f.burn_time as i16));
        be.insert(
            "CookTime".to_string(),
            NbtTag::Short(f.cook_progress as i16),
        );
        be.insert("CookTimeTotal".to_string(), NbtTag::Short(200));
        let mut items = Vec::new();
        if let Some(tag) = encode_nbt_slot_item(0, f.input_item, f.input_count, None, regs) {
            items.push(tag);
        }
        if let Some(tag) = encode_nbt_slot_item(1, f.fuel_item, f.fuel_count, None, regs) {
            items.push(tag);
        }
        if let Some(tag) = encode_nbt_slot_item(2, f.output_item, f.output_count, None, regs) {
            items.push(tag);
        }
        be.insert("Items".to_string(), NbtTag::List(items));
        by_chunk
            .entry((dim, x.div_euclid(16), z.div_euclid(16)))
            .or_default()
            .push(be);
    }

    for ((dim, cx, cz), block_entities) in by_chunk {
        world.set_chunk_block_entities(dim, cx, cz, block_entities);
    }
}

#[cfg(test)]
fn load_block_ticks_into_sim(
    world_dir: &std::path::Path,
    regs: &Version26_3Registries,
    sim: &mut crate::world::sim::WorldSim,
) {
    let loaded = crate::world::WorldState::load_all_block_ticks_from_mca(
        world_dir,
        regs.provider().as_ref(),
    );
    apply_block_ticks_to_sim(loaded, sim);
}

fn apply_block_ticks_to_sim(
    loaded: Vec<(
        crate::world::worldgen::DimensionId,
        std::collections::BTreeMap<String, NbtTag>,
    )>,
    sim: &mut crate::world::sim::WorldSim,
) {
    for (dim, map) in loaded {
        let (Some(NbtTag::Int(x)), Some(NbtTag::Int(y)), Some(NbtTag::Int(z))) =
            (map.get("x"), map.get("y"), map.get("z"))
        else {
            continue;
        };
        let delay = match map.get("t") {
            Some(NbtTag::Int(t)) => (*t).max(1) as u64,
            _ => 1,
        };
        let action = match map.get("rad_action") {
            Some(NbtTag::String(tag)) => crate::world::sim::SimAction::decode_tag(tag),
            _ => None,
        }
        .or_else(|| {
            let Some(NbtTag::String(block_name)) = map.get("i") else {
                return None;
            };
            let short = block_name.strip_prefix("minecraft:").unwrap_or(block_name);
            if matches!(short, "water" | "lava") {
                Some(crate::world::sim::SimAction::FluidSpread { depth: 0 })
            } else if matches!(short, "sand" | "red_sand" | "gravel")
                || short.ends_with("_concrete_powder")
                || short.ends_with("anvil")
            {
                Some(crate::world::sim::SimAction::GravityFall)
            } else if short == "repeater" {
                Some(crate::world::sim::SimAction::RepeaterTick)
            } else if short == "fire" {
                Some(crate::world::sim::SimAction::FireExtinguish)
            } else {
                None
            }
        });
        if let Some(action) = action {
            sim.schedule_loaded_tick(dim, *x, *y, *z, delay, action);
        }
    }
}

fn sync_block_ticks_to_world(
    world: &mut crate::world::WorldState,
    sim: &crate::world::sim::WorldSim,
    regs: &Version26_3Registries,
    cur_tick: u64,
) {
    let mut by_chunk: HashMap<
        (crate::world::worldgen::DimensionId, i32, i32),
        Vec<std::collections::BTreeMap<String, NbtTag>>,
    > = HashMap::new();
    for (dim, x, y, z, delay, action) in sim.all_pending_ticks(cur_tick) {
        let state_id = world.get_block_in(dim, x, y, z);
        let block_name = regs
            .block_by_state(state_id)
            .map_or_else(|| "minecraft:air".to_owned(), |b| b.name);
        let mut tick = std::collections::BTreeMap::new();
        tick.insert("i".to_owned(), NbtTag::String(block_name));
        tick.insert("p".to_owned(), NbtTag::Int(0));
        tick.insert(
            "t".to_owned(),
            NbtTag::Int(delay.clamp(1, i32::MAX as u64) as i32),
        );
        tick.insert("x".to_owned(), NbtTag::Int(x));
        tick.insert("y".to_owned(), NbtTag::Int(y));
        tick.insert("z".to_owned(), NbtTag::Int(z));
        tick.insert("rad_action".to_owned(), NbtTag::String(action.encode_tag()));
        by_chunk
            .entry((dim, x.div_euclid(16), z.div_euclid(16)))
            .or_default()
            .push(tick);
    }
    for ((dim, cx, cz), block_ticks) in by_chunk {
        world.set_chunk_block_ticks(dim, cx, cz, block_ticks);
    }
}

#[cfg(test)]
mod playerdata_nbt_tests {
    use super::*;

    #[test]
    fn playerdata_nbt_roundtrip_preserves_state_and_inventory() {
        let provider: Arc<dyn rad_core::registry_provider::RegistryProvider> =
            Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = crate::registries::Version26_3Registries::from_provider(provider, true)
            .expect("synthetic registries");
        let item_id = regs.item_by_name("minecraft:cobblestone").unwrap_or(1);

        let mut inv =
            vec![SavedSlot::default(); crate::gameplay::inventory::PLAYER_INVENTORY_SLOTS];
        inv[36] = SavedSlot {
            count: 16,
            item_id,
            components: Default::default(),
        };

        let original = SavedPlayerData {
            x: 12.5,
            y: 65.0,
            z: -3.25,
            yaw: 90.0,
            pitch: -15.0,
            hp: 18.5,
            food: 17,
            saturation: 4.5,
            game_mode: 1,
            inventory: inv,
            selected_hotbar: 2,
            dimension: crate::world::worldgen::DimensionId::Nether,
            stats: PlayerStats {
                blocks_broken: 42,
                blocks_placed: 7,
                mobs_killed: 3,
            },
        };

        let nbt = original.to_nbt(&regs);
        let decoded = SavedPlayerData::from_nbt(&nbt, &regs).expect("decode nbt");
        assert!((decoded.x - original.x).abs() < f64::EPSILON);
        assert!((decoded.y - original.y).abs() < f64::EPSILON);
        assert!((decoded.z - original.z).abs() < f64::EPSILON);
        assert_eq!(decoded.game_mode, 1);
        assert_eq!(decoded.selected_hotbar, 2);
        assert_eq!(
            decoded.dimension,
            crate::world::worldgen::DimensionId::Nether
        );
        assert_eq!(decoded.inventory[36].count, 16);
        assert_eq!(decoded.inventory[36].item_id, item_id);
        assert_eq!(decoded.stats.blocks_broken, 42);
    }

    #[test]
    fn block_entities_mca_roundtrip_preserves_chests_and_furnaces() {
        let provider: Arc<dyn rad_core::registry_provider::RegistryProvider> =
            Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = crate::registries::Version26_3Registries::from_provider(provider.clone(), true)
            .expect("synthetic registries");
        let item_id = regs.item_by_name("minecraft:cobblestone").unwrap_or(1);

        let dir = std::env::temp_dir().join(format!(
            "rad_block_entities_mca_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp world dir");

        let mut world = crate::world::WorldState::new_with_registries(
            Some(dir.clone()),
            Arc::new(regs.clone()),
        );
        let mut containers = crate::world::containers::ContainerStore::default();
        let mut slots = vec![
            rad_core::packet::play::SimpleSlot::default();
            crate::world::containers::CHEST_SLOTS
        ];
        slots[3] = rad_core::packet::play::SimpleSlot { count: 12, item_id };
        let comps = vec![
            rad_core::packet::play::ItemComponentPatch::default();
            crate::world::containers::CHEST_SLOTS
        ];
        containers.set_chest_slots_in(
            crate::world::worldgen::DimensionId::Overworld,
            5,
            64,
            7,
            slots,
            comps,
        );

        let mut furnaces = crate::world::sim::FurnaceStore::default();
        furnaces.insert_in(
            crate::world::worldgen::DimensionId::Overworld,
            6,
            64,
            7,
            crate::world::sim::FurnaceEntity {
                input_item: item_id,
                input_count: 8,
                fuel_item: item_id,
                fuel_count: 2,
                output_item: item_id,
                output_count: 1,
                burn_time: 150,
                cook_progress: 45,
            },
        );

        world.set_block(5, 64, 7, 1);
        sync_block_entities_to_world(&mut world, &containers, &furnaces, &regs);
        world.save_dirty_chunks().expect("save dirty chunks");

        let mut loaded_containers = crate::world::containers::ContainerStore::default();
        let mut loaded_furnaces = crate::world::sim::FurnaceStore::default();
        load_block_entities_into_stores(&dir, &regs, &mut loaded_containers, &mut loaded_furnaces);

        let chests = loaded_containers.all_chests();
        assert_eq!(chests.len(), 1);
        assert_eq!(chests[0].0, crate::world::worldgen::DimensionId::Overworld);
        assert_eq!((chests[0].1, chests[0].2, chests[0].3), (5, 64, 7));
        assert_eq!(chests[0].4[3].count, 12);
        assert_eq!(chests[0].4[3].item_id, item_id);

        let f_list = loaded_furnaces.all_furnaces();
        assert_eq!(f_list.len(), 1);
        assert_eq!((f_list[0].1, f_list[0].2, f_list[0].3), (6, 64, 7));
        assert_eq!(f_list[0].4.burn_time, 150);
        assert_eq!(f_list[0].4.cook_progress, 45);
        assert_eq!(f_list[0].4.input_count, 8);
        assert_eq!(f_list[0].4.fuel_count, 2);
        assert_eq!(f_list[0].4.output_count, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_ticks_mca_roundtrip_preserves_sim_queue() {
        let provider: Arc<dyn rad_core::registry_provider::RegistryProvider> =
            Arc::new(crate::registries::synthetic_26_3_test_provider());
        let regs = crate::registries::Version26_3Registries::from_provider(provider, true)
            .expect("synthetic registries");

        let dir =
            std::env::temp_dir().join(format!("rad_block_ticks_mca_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp world dir");

        let mut world = crate::world::WorldState::new_with_registries(
            Some(dir.clone()),
            Arc::new(regs.clone()),
        );
        world.set_block(4, 65, 8, 1);

        let mut sim = crate::world::sim::WorldSim::default();
        sim.schedule_loaded_tick(
            crate::world::worldgen::DimensionId::Overworld,
            4,
            65,
            8,
            12,
            crate::world::sim::SimAction::FluidSpread { depth: 2 },
        );

        sync_block_ticks_to_world(&mut world, &sim, &regs, 0);
        world.save_dirty_chunks().expect("save dirty chunks");

        let mut reloaded_sim = crate::world::sim::WorldSim::default();
        load_block_ticks_into_sim(&dir, &regs, &mut reloaded_sim);
        let ticks = reloaded_sim.all_pending_ticks(0);
        assert_eq!(ticks.len(), 1);
        assert_eq!(
            ticks[0],
            (
                crate::world::worldgen::DimensionId::Overworld,
                4,
                65,
                8,
                12,
                crate::world::sim::SimAction::FluidSpread { depth: 2 }
            )
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
