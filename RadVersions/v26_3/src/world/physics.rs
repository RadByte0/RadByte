//! Voxel AABB collision, movement validation, and fall damage adapter for `RadServer`.
//!
//! Core shapes and movement math live in [`rad_version::shapes`] and [`rad_version::physics`].

use rad_core::registry_provider::RegistryProvider;
pub use rad_version::physics::{
    Aabb, BlockCollisionLookup, MoveValidation, block_collision_boxes_with, compute_fall_damage,
    is_solid_block_with, validate_player_move_with_pose_lookup_with,
};
pub use rad_version::shapes::{
    BlockBox, VoxelShape, block_collision_shape_with, block_interaction_shape_with,
    block_supports_top_face_with,
};

use crate::world::WorldState;
use crate::world::anvil::MIN_WORLD_Y;
use crate::world::worldgen::DimensionId;

struct WorldDimensionLookup<'a> {
    world: &'a mut WorldState,
    dim: DimensionId,
}

impl BlockCollisionLookup for WorldDimensionLookup<'_> {
    fn get_block_state(&mut self, x: i32, y: i32, z: i32) -> u16 {
        self.world.get_block_in(self.dim, x, y, z)
    }
}

/// Extension trait adapting [`Aabb`] to [`WorldState`] queries.
pub trait WorldAabbExt {
    /// Returns `true` if this AABB overlaps any solid block in `world` in `dim` using `provider`.
    fn intersects_solid_in_with(
        &self,
        world: &mut WorldState,
        dim: DimensionId,
        provider: &dyn RegistryProvider,
    ) -> bool;
}

impl WorldAabbExt for Aabb {
    fn intersects_solid_in_with(
        &self,
        world: &mut WorldState,
        dim: DimensionId,
        provider: &dyn RegistryProvider,
    ) -> bool {
        let mut lookup = WorldDimensionLookup { world, dim };
        self.intersects_solid_lookup_with(&mut lookup, MIN_WORLD_Y, provider)
    }
}

/// Validates a player move with explicit `noclip`, `player_height`, and `provider`.
#[allow(clippy::too_many_arguments)]
pub fn validate_player_move_with_pose_in_with(
    world: &mut WorldState,
    dim: DimensionId,
    from: (f64, f64, f64),
    to: (f64, f64, f64),
    elapsed_secs: f64,
    allow_flight: bool,
    noclip: bool,
    player_height: f64,
    provider: &dyn RegistryProvider,
) -> MoveValidation {
    let mut lookup = WorldDimensionLookup { world, dim };
    validate_player_move_with_pose_lookup_with(
        &mut lookup,
        MIN_WORLD_Y,
        from,
        to,
        elapsed_secs,
        allow_flight,
        noclip,
        player_height,
        provider,
    )
}

/// Computes authoritative ground contact directly below the player's feet in `dim` using `provider`.
#[must_use]
pub fn is_player_on_ground_in_with(
    world: &mut WorldState,
    dim: DimensionId,
    x: f64,
    y: f64,
    z: f64,
    provider: &dyn RegistryProvider,
) -> bool {
    Aabb::from_player(x, y - 0.05, z).intersects_solid_in_with(world, dim, provider)
}
