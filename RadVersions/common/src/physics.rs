//! Shared voxel AABB collision, movement validation, and fall damage math.
//!
//! Runtime- and storage-agnostic via [`BlockCollisionLookup`], usable by both
//! `RadVersions/v26_3` server validation and future `RadClient` prediction.

use rad_core::registry_provider::RegistryProvider;

use crate::shapes::{BlockBox, VoxelShape, block_collision_shape_with};

/// Read-only block state lookup for collision queries.
pub trait BlockCollisionLookup {
    /// Returns the block state ID at world coordinates `(x, y, z)`.
    fn get_block_state(&mut self, x: i32, y: i32, z: i32) -> u16;
}

/// Axis-aligned bounding box in world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Minimum X coordinate.
    pub min_x: f64,
    /// Minimum Y coordinate.
    pub min_y: f64,
    /// Minimum Z coordinate.
    pub min_z: f64,
    /// Maximum X coordinate.
    pub max_x: f64,
    /// Maximum Y coordinate.
    pub max_y: f64,
    /// Maximum Z coordinate.
    pub max_z: f64,
}

impl Aabb {
    /// Standard standing player bounding box (`0.6 x 1.8` blocks).
    #[must_use]
    pub const fn from_player(x: f64, y: f64, z: f64) -> Self {
        Self::from_entity(x, y, z, 0.3, 1.8)
    }

    /// Player bounding box with explicit height (e.g. `1.8` standing, `1.5` sneaking, `0.6` swimming).
    #[must_use]
    pub const fn from_player_height(x: f64, y: f64, z: f64, height: f64) -> Self {
        Self::from_entity(x, y, z, 0.3, height)
    }

    /// Generic entity bounding box centered on `(x, z)` with base at `y`.
    #[must_use]
    pub const fn from_entity(x: f64, y: f64, z: f64, half_width: f64, height: f64) -> Self {
        Self {
            min_x: x - half_width,
            min_y: y,
            min_z: z - half_width,
            max_x: x + half_width,
            max_y: y + height,
            max_z: z + half_width,
        }
    }

    /// Returns `true` if this AABB overlaps any solid block via `lookup` using `provider` (clamped at `min_world_y`).
    pub fn intersects_solid_lookup_with<L: BlockCollisionLookup + ?Sized>(
        &self,
        lookup: &mut L,
        min_world_y: i32,
        provider: &dyn RegistryProvider,
    ) -> bool {
        // Small epsilon avoids false positives when standing flush on top of a block.
        // Start one block lower on Y so 1.5-tall blocks (fences/walls/gates) are checked.
        let eps = 1e-4;
        let x0 = (self.min_x + eps).floor() as i32;
        let x1 = (self.max_x - eps).floor() as i32;
        let y0 = ((self.min_y + eps).floor() as i32 - 1).max(min_world_y);
        let y1 = (self.max_y - eps).floor() as i32;
        let z0 = (self.min_z + eps).floor() as i32;
        let z1 = (self.max_z - eps).floor() as i32;

        for by in y0..=y1 {
            for bz in z0..=z1 {
                for bx in x0..=x1 {
                    let state_id = lookup.get_block_state(bx, by, bz);
                    let boxes =
                        block_collision_boxes_with(state_id, provider).unwrap_or(&[BlockBox::FULL]);
                    for shape in boxes {
                        let smin_x = f64::from(bx) + shape.min_x;
                        let smax_x = f64::from(bx) + shape.max_x;
                        let smin_y = f64::from(by) + shape.min_y;
                        let smax_y = f64::from(by) + shape.max_y;
                        let smin_z = f64::from(bz) + shape.min_z;
                        let smax_z = f64::from(bz) + shape.max_z;
                        if (self.min_x + eps) < smax_x
                            && (self.max_x - eps) > smin_x
                            && (self.min_y + eps) < smax_y
                            && (self.max_y - eps) > smin_y
                            && (self.min_z + eps) < smax_z
                            && (self.max_z - eps) > smin_z
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Returns local collision boxes for `state_id` using a [`RegistryProvider`].
#[must_use]
pub fn block_collision_boxes_with(
    state_id: u16,
    provider: &dyn RegistryProvider,
) -> Option<&'static [BlockBox]> {
    Some(block_collision_shape_with(state_id, provider)?.as_boxes())
}

/// Returns `true` if `state_id` represents a solid collidable block using a [`RegistryProvider`].
#[must_use]
pub fn is_solid_block_with(state_id: u16, provider: &dyn RegistryProvider) -> Option<bool> {
    Some(!matches!(
        block_collision_shape_with(state_id, provider)?,
        VoxelShape::Empty
    ))
}

/// Result of validating a client-reported player movement step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveValidation {
    /// Movement is physically plausible.
    Accepted,
    /// Movement exceeded max single-packet speed/distance or contained non-finite coordinates.
    TooFast,
    /// Destination AABB intersects solid world geometry.
    CollidedWithBlock,
}

fn sweep_clear<L: BlockCollisionLookup + ?Sized>(
    lookup: &mut L,
    min_world_y: i32,
    from: (f64, f64, f64),
    to: (f64, f64, f64),
    steps: usize,
    height: f64,
    provider: &dyn RegistryProvider,
) -> bool {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let dz = to.2 - from.2;
    (1..=steps).all(|step| {
        let t = step as f64 / steps as f64;
        !Aabb::from_player_height(from.0 + dx * t, from.1 + dy * t, from.2 + dz * t, height)
            .intersects_solid_lookup_with(lookup, min_world_y, provider)
    })
}

/// Validates a player move with explicit `noclip`, `player_height`, and `provider`.
#[allow(clippy::too_many_arguments)]
pub fn validate_player_move_with_pose_lookup_with<L: BlockCollisionLookup + ?Sized>(
    lookup: &mut L,
    min_world_y: i32,
    from: (f64, f64, f64),
    to: (f64, f64, f64),
    elapsed_secs: f64,
    allow_flight: bool,
    noclip: bool,
    player_height: f64,
    provider: &dyn RegistryProvider,
) -> MoveValidation {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let dz = to.2 - from.2;
    let dist_sq = dx * dx + dy * dy + dz * dz;

    let max_speed = if noclip {
        120.0
    } else if allow_flight {
        22.0
    } else {
        12.0
    };
    let max_step = max_speed * elapsed_secs.clamp(0.05, 0.5) + 0.5;
    if !dist_sq.is_finite()
        || !to.0.is_finite()
        || !to.1.is_finite()
        || !to.2.is_finite()
        || dist_sq > max_step * max_step
    {
        return MoveValidation::TooFast;
    }

    if noclip {
        return MoveValidation::Accepted;
    }

    let current_box = Aabb::from_player_height(from.0, from.1, from.2, player_height);
    let dest_box = Aabb::from_player_height(to.0, to.1, to.2, player_height);
    if current_box.intersects_solid_lookup_with(lookup, min_world_y, provider) {
        if !dest_box.intersects_solid_lookup_with(lookup, min_world_y, provider) {
            return MoveValidation::Accepted;
        }
        if dist_sq <= 0.65 * 0.65 {
            return MoveValidation::Accepted;
        }
        return MoveValidation::CollidedWithBlock;
    }

    if dest_box.intersects_solid_lookup_with(lookup, min_world_y, provider) {
        return MoveValidation::CollidedWithBlock;
    }

    let dist = dist_sq.sqrt();
    let steps = (dist / 0.25).ceil().clamp(1.0, 64.0) as usize;
    if sweep_clear(
        lookup,
        min_world_y,
        from,
        to,
        steps,
        player_height,
        provider,
    ) {
        return MoveValidation::Accepted;
    }

    if dy > 0.0
        && sweep_clear(
            lookup,
            min_world_y,
            from,
            (from.0, to.1, from.2),
            steps,
            player_height,
            provider,
        )
        && sweep_clear(
            lookup,
            min_world_y,
            (from.0, to.1, from.2),
            to,
            steps,
            player_height,
            provider,
        )
    {
        return MoveValidation::Accepted;
    }

    let xzy_clear = sweep_clear(
        lookup,
        min_world_y,
        from,
        (to.0, from.1, from.2),
        steps,
        player_height,
        provider,
    ) && sweep_clear(
        lookup,
        min_world_y,
        (to.0, from.1, from.2),
        (to.0, from.1, to.2),
        steps,
        player_height,
        provider,
    ) && sweep_clear(
        lookup,
        min_world_y,
        (to.0, from.1, to.2),
        to,
        steps,
        player_height,
        provider,
    );
    if xzy_clear {
        return MoveValidation::Accepted;
    }

    let zxy_clear = sweep_clear(
        lookup,
        min_world_y,
        from,
        (from.0, from.1, to.2),
        steps,
        player_height,
        provider,
    ) && sweep_clear(
        lookup,
        min_world_y,
        (from.0, from.1, to.2),
        (to.0, from.1, to.2),
        steps,
        player_height,
        provider,
    ) && sweep_clear(
        lookup,
        min_world_y,
        (to.0, from.1, to.2),
        to,
        steps,
        player_height,
        provider,
    );
    if zxy_clear {
        return MoveValidation::Accepted;
    }

    MoveValidation::CollidedWithBlock
}

/// Computes vanilla-style fall damage for a completed fall distance in blocks.
#[must_use]
pub fn compute_fall_damage(fall_distance: f64) -> f32 {
    if fall_distance <= 3.0 || !fall_distance.is_finite() {
        return 0.0;
    }
    ((fall_distance - 3.0).floor() as f32).clamp(0.0, 100.0)
}
