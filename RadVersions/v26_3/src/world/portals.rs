//! Vanilla-compatible Nether and End portal construction, validation, and linking.

use std::collections::{HashSet, VecDeque};

use crate::registries::state_in_entry;
use crate::world::WorldState;
use crate::world::anvil::{MAX_WORLD_Y, MIN_WORLD_Y};
use crate::world::worldgen::DimensionId;

const MIN_PORTAL_WIDTH: i32 = 2;
const MAX_PORTAL_WIDTH: i32 = 21;
const MIN_PORTAL_HEIGHT: i32 = 3;
const MAX_PORTAL_HEIGHT: i32 = 21;

/// List of `(x, y, z, block_state)` updates produced by portal operations.
pub type PortalBlockUpdates = Vec<(i32, i32, i32, u16)>;

/// Attempts to light a Nether portal containing `(x, y, z)` along the X or Z axis.
pub fn try_light_nether_portal(
    world: &mut WorldState,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<(i32, i32, i32, u16)> {
    if !matches!(dim, DimensionId::Overworld | DimensionId::Nether) {
        return Vec::new();
    }
    let regs = std::sync::Arc::clone(world.registries());
    let Some(obsidian) = regs.block_by_name("minecraft:obsidian") else {
        return Vec::new();
    };
    let Some(portal) = regs.block_by_name("minecraft:nether_portal") else {
        return Vec::new();
    };

    for is_x_axis in [true, false] {
        if let Some(updates) = try_light_axis(world, dim, x, y, z, is_x_axis, &obsidian, &portal) {
            return updates;
        }
    }
    Vec::new()
}

#[allow(clippy::too_many_arguments)]
fn try_light_axis(
    world: &mut WorldState,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    is_x_axis: bool,
    obsidian: &rad_core::registry_provider::BlockRegistryEntry,
    portal: &rad_core::registry_provider::BlockRegistryEntry,
) -> Option<Vec<(i32, i32, i32, u16)>> {
    let regs = std::sync::Arc::clone(world.registries());
    let is_interior_replaceable = |st: u16| -> bool {
        regs.is_air_state(st) || st == regs.blocks.fire.default_state || state_in_entry(portal, st)
    };
    let is_obs = |st: u16| state_in_entry(obsidian, st);

    let get_at = |w: &mut WorldState, u: i32, py: i32| -> u16 {
        if is_x_axis {
            w.get_block_in(dim, u, py, z)
        } else {
            w.get_block_in(dim, x, py, u)
        }
    };

    let origin_u = if is_x_axis { x } else { z };
    if !is_interior_replaceable(get_at(world, origin_u, y)) {
        return None;
    }

    // Find bottom obsidian.
    let mut bottom_y = y;
    while bottom_y > MIN_WORLD_Y && is_interior_replaceable(get_at(world, origin_u, bottom_y - 1)) {
        if y - (bottom_y - 1) >= MAX_PORTAL_HEIGHT {
            return None;
        }
        bottom_y -= 1;
    }
    if bottom_y <= MIN_WORLD_Y || !is_obs(get_at(world, origin_u, bottom_y - 1)) {
        return None;
    }

    // Find left and right bounds along U at bottom_y.
    let mut min_u = origin_u;
    while is_interior_replaceable(get_at(world, min_u - 1, bottom_y)) {
        if origin_u - (min_u - 1) >= MAX_PORTAL_WIDTH {
            return None;
        }
        min_u -= 1;
    }
    if !is_obs(get_at(world, min_u - 1, bottom_y)) {
        return None;
    }

    let mut max_u = origin_u;
    while is_interior_replaceable(get_at(world, max_u + 1, bottom_y)) {
        if (max_u + 1) - min_u + 1 > MAX_PORTAL_WIDTH {
            return None;
        }
        max_u += 1;
    }
    if !is_obs(get_at(world, max_u + 1, bottom_y)) {
        return None;
    }

    let width = max_u - min_u + 1;
    if !(MIN_PORTAL_WIDTH..=MAX_PORTAL_WIDTH).contains(&width) {
        return None;
    }

    // Find top obsidian.
    let mut top_y = bottom_y;
    while top_y < MAX_WORLD_Y && is_interior_replaceable(get_at(world, min_u, top_y + 1)) {
        if (top_y + 1) - bottom_y + 1 > MAX_PORTAL_HEIGHT {
            return None;
        }
        top_y += 1;
    }
    let height = top_y - bottom_y + 1;
    if !(MIN_PORTAL_HEIGHT..=MAX_PORTAL_HEIGHT).contains(&height) {
        return None;
    }

    // Verify full obsidian frame (bottom, top, left, right) and replaceable interior.
    for u in min_u..=max_u {
        if !is_obs(get_at(world, u, bottom_y - 1)) || !is_obs(get_at(world, u, top_y + 1)) {
            return None;
        }
        for py in bottom_y..=top_y {
            if !is_interior_replaceable(get_at(world, u, py)) {
                return None;
            }
        }
    }
    for py in bottom_y..=top_y {
        if !is_obs(get_at(world, min_u - 1, py)) || !is_obs(get_at(world, max_u + 1, py)) {
            return None;
        }
    }

    let portal_state = if is_x_axis {
        portal.min_state
    } else {
        (portal.min_state + 1).min(portal.max_state)
    };
    let mut updates = Vec::with_capacity((width * height) as usize);
    for u in min_u..=max_u {
        for py in bottom_y..=top_y {
            let (wx, wz) = if is_x_axis { (u, z) } else { (x, u) };
            if world.set_block_in(dim, wx, py, wz, portal_state) {
                updates.push((wx, py, wz, portal_state));
            }
        }
    }
    Some(updates)
}

/// Clears all connected `minecraft:nether_portal` blocks when a portal block or adjacent obsidian breaks.
pub fn invalidate_broken_nether_portal(
    world: &mut WorldState,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<(i32, i32, i32, u16)> {
    let regs = std::sync::Arc::clone(world.registries());
    let Some(portal) = regs.block_by_name("minecraft:nether_portal") else {
        return Vec::new();
    };
    let air = regs.blocks.air.default_state;

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    for (nx, ny, nz) in [
        (x + 1, y, z),
        (x - 1, y, z),
        (x, y + 1, z),
        (x, y - 1, z),
        (x, y, z + 1),
        (x, y, z - 1),
    ] {
        if state_in_entry(&portal, world.get_block_in(dim, nx, ny, nz))
            && visited.insert((nx, ny, nz))
        {
            queue.push_back((nx, ny, nz));
        }
    }

    let mut updates = Vec::new();
    while let Some((px, py, pz)) = queue.pop_front() {
        if !state_in_entry(&portal, world.get_block_in(dim, px, py, pz)) {
            continue;
        }
        if world.set_block_in(dim, px, py, pz, air) {
            updates.push((px, py, pz, air));
        }
        for (nx, ny, nz) in [
            (px + 1, py, pz),
            (px - 1, py, pz),
            (px, py + 1, pz),
            (px, py - 1, pz),
            (px, py, pz + 1),
            (px, py, pz - 1),
        ] {
            if state_in_entry(&portal, world.get_block_in(dim, nx, ny, nz))
                && visited.insert((nx, ny, nz))
            {
                queue.push_back((nx, ny, nz));
            }
        }
    }
    updates
}

/// Checks if placing an Eye of Ender in `(frame_x, frame_y, frame_z)` completes a 3x3 End Portal ring.
pub fn try_activate_end_portal(
    world: &mut WorldState,
    dim: DimensionId,
    frame_x: i32,
    frame_y: i32,
    frame_z: i32,
) -> Vec<(i32, i32, i32, u16)> {
    let regs = std::sync::Arc::clone(world.registries());
    let Some(frame) = regs.block_by_name("minecraft:end_portal_frame") else {
        return Vec::new();
    };
    let Some(end_portal) = regs.block_by_name("minecraft:end_portal") else {
        return Vec::new();
    };

    let is_eye_frame =
        |st: u16| -> bool { state_in_entry(&frame, st) && st < frame.min_state.saturating_add(4) };

    // Test candidate 3x3 centers around the newly eyed frame block.
    for cx in (frame_x - 2)..=(frame_x + 2) {
        for cz in (frame_z - 2)..=(frame_z + 2) {
            let mut ring_ok = true;
            for offset in -1..=1 {
                let coords = [
                    (cx + offset, frame_y, cz - 2),
                    (cx + offset, frame_y, cz + 2),
                    (cx - 2, frame_y, cz + offset),
                    (cx + 2, frame_y, cz + offset),
                ];
                for (fx, fy, fz) in coords {
                    if !is_eye_frame(world.get_block_in(dim, fx, fy, fz)) {
                        ring_ok = false;
                        break;
                    }
                }
                if !ring_ok {
                    break;
                }
            }
            if ring_ok {
                let mut updates = Vec::with_capacity(9);
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        if world.set_block_in(
                            dim,
                            cx + dx,
                            frame_y,
                            cz + dz,
                            end_portal.default_state,
                        ) {
                            updates.push((cx + dx, frame_y, cz + dz, end_portal.default_state));
                        }
                    }
                }
                return updates;
            }
        }
    }
    Vec::new()
}

/// Finds an existing Nether portal near `(target_x, target_y, target_z)` or creates a 2x3 portal frame.
pub fn find_or_create_nether_portal(
    world: &mut WorldState,
    target_dim: DimensionId,
    target_x: i32,
    target_y: i32,
    target_z: i32,
) -> ((f64, f64, f64), PortalBlockUpdates) {
    let regs = std::sync::Arc::clone(world.registries());
    let Some(portal) = regs.block_by_name("minecraft:nether_portal") else {
        return (
            (
                f64::from(target_x) + 0.5,
                f64::from(target_y),
                f64::from(target_z) + 0.5,
            ),
            Vec::new(),
        );
    };

    // Search nearby for an existing portal.
    let mut best: Option<(i64, i32, i32, i32)> = None;
    for dx in -16..=16 {
        for dz in -16..=16 {
            let x = target_x + dx;
            let z = target_z + dz;
            for y in (MIN_WORLD_Y + 4)..=120 {
                if state_in_entry(&portal, world.get_block_in(target_dim, x, y, z)) {
                    let dist_sq = i64::from(dx) * i64::from(dx)
                        + i64::from(dz) * i64::from(dz)
                        + i64::from(y - target_y) * i64::from(y - target_y);
                    if best.is_none_or(|(bd, _, _, _)| dist_sq < bd) {
                        best = Some((dist_sq, x, y, z));
                    }
                }
            }
        }
    }
    if let Some((_, px, mut py, pz)) = best {
        while py > MIN_WORLD_Y + 1
            && state_in_entry(&portal, world.get_block_in(target_dim, px, py - 1, pz))
        {
            py -= 1;
        }
        return (
            (f64::from(px) + 0.5, f64::from(py), f64::from(pz) + 0.5),
            Vec::new(),
        );
    }

    let Some(obsidian) = regs.block_by_name("minecraft:obsidian") else {
        return (
            (
                f64::from(target_x) + 0.5,
                f64::from(target_y),
                f64::from(target_z) + 0.5,
            ),
            Vec::new(),
        );
    };
    let air = regs.blocks.air.default_state;
    let base_y = target_y.clamp(MIN_WORLD_Y + 8, 110);
    let mut updates = Vec::new();

    // Clear 3x4x3 interior around the new portal and ensure obsidian floor/frame.
    for dx in -1..=2 {
        for dz in -1..=1 {
            if world.set_block_in(
                target_dim,
                target_x + dx,
                base_y - 1,
                target_z + dz,
                obsidian.default_state,
            ) {
                updates.push((
                    target_x + dx,
                    base_y - 1,
                    target_z + dz,
                    obsidian.default_state,
                ));
            }
            for dy in 0..=3 {
                if world.set_block_in(target_dim, target_x + dx, base_y + dy, target_z + dz, air) {
                    updates.push((target_x + dx, base_y + dy, target_z + dz, air));
                }
            }
        }
    }
    for dx in -1..=2 {
        for dy in -1..=3 {
            let is_border = dx == -1 || dx == 2 || dy == -1 || dy == 3;
            let st = if is_border {
                obsidian.default_state
            } else {
                portal.min_state
            };
            if world.set_block_in(target_dim, target_x + dx, base_y + dy, target_z, st) {
                updates.push((target_x + dx, base_y + dy, target_z, st));
            }
        }
    }

    (
        (
            f64::from(target_x) + 0.5,
            f64::from(base_y),
            f64::from(target_z) + 0.5,
        ),
        updates,
    )
}

/// Ensures the Vanilla 5x5 obsidian spawn platform at `(100, 48, 0)` exists in The End.
pub fn ensure_end_spawn_platform(world: &mut WorldState) -> Vec<(i32, i32, i32, u16)> {
    let regs = std::sync::Arc::clone(world.registries());
    let Some(obsidian) = regs.block_by_name("minecraft:obsidian") else {
        return Vec::new();
    };
    let air = regs.blocks.air.default_state;
    let mut updates = Vec::new();
    for x in 98..=102 {
        for z in -2..=2 {
            if world.set_block_in(DimensionId::End, x, 48, z, obsidian.default_state) {
                updates.push((x, 48, z, obsidian.default_state));
            }
            for y in 49..=51 {
                if world.set_block_in(DimensionId::End, x, y, z, air) {
                    updates.push((x, y, z, air));
                }
            }
        }
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::entity::MobKind;

    #[test]
    fn vanilla_mob_roster_parses_all_common_entities() {
        for (name, expected, hostile) in [
            ("minecraft:zombie", MobKind::Zombie, true),
            ("minecraft:skeleton", MobKind::Skeleton, true),
            ("minecraft:creeper", MobKind::Creeper, true),
            ("minecraft:spider", MobKind::Spider, true),
            ("minecraft:enderman", MobKind::Enderman, false),
            ("minecraft:pig", MobKind::Pig, false),
            ("minecraft:cow", MobKind::Cow, false),
            ("minecraft:sheep", MobKind::Sheep, false),
            ("minecraft:chicken", MobKind::Chicken, false),
            ("minecraft:villager", MobKind::Villager, false),
            ("minecraft:iron_golem", MobKind::IronGolem, false),
        ] {
            let parsed = MobKind::from_resource_name(name).expect(name);
            assert_eq!(parsed, expected);
            assert_eq!(parsed.is_hostile(), hostile);
            assert_eq!(parsed.resource_name(), name);
        }
    }

    #[test]
    fn nether_and_end_portals_work_with_real_registries() {
        let pkg_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/package-check/versions/26.3");
        if !pkg_dir.is_dir() {
            return;
        }
        let Ok(pkg) =
            rad_core::version_package::LoadedVersionPackage::load_from_package_dir(&pkg_dir)
        else {
            return;
        };
        let provider: std::sync::Arc<dyn rad_core::registry_provider::RegistryProvider> =
            std::sync::Arc::new(crate::registries::ArcPackageRegistryProvider(
                std::sync::Arc::new(pkg),
            ));
        let regs = std::sync::Arc::new(
            crate::registries::Version26_3Registries::from_provider(provider, true)
                .expect("load real 26.3 registries"),
        );
        let mut world = WorldState::new_with_registries(None, regs.clone());

        let obsidian = regs
            .block_by_name("minecraft:obsidian")
            .unwrap()
            .default_state;
        let portal = regs.block_by_name("minecraft:nether_portal").unwrap();

        // Build 4x5 obsidian frame along X axis at z = 0, x in 0..=3, y in 64..=68.
        for x in 0..=3 {
            for y in 64..=68 {
                if x == 0 || x == 3 || y == 64 || y == 68 {
                    assert!(world.set_block_in(DimensionId::Overworld, x, y, 0, obsidian));
                }
            }
        }
        let lit = try_light_nether_portal(&mut world, DimensionId::Overworld, 1, 65, 0);
        assert_eq!(lit.len(), 6);
        assert!(state_in_entry(
            &portal,
            world.get_block_in(DimensionId::Overworld, 2, 67, 0)
        ));

        // Breaking a frame block clears all 6 portal blocks.
        assert!(world.set_block_in(DimensionId::Overworld, 0, 65, 0, 0));
        let cleared = invalidate_broken_nether_portal(&mut world, DimensionId::Overworld, 0, 65, 0);
        assert_eq!(cleared.len(), 6);
        assert_eq!(world.get_block_in(DimensionId::Overworld, 2, 67, 0), 0);

        // End spawn platform check.
        let platform = ensure_end_spawn_platform(&mut world);
        assert!(!platform.is_empty());
        assert_eq!(world.get_block_in(DimensionId::End, 100, 48, 0), obsidian);
        assert_eq!(world.get_block_in(DimensionId::End, 100, 49, 0), 0);
    }

    #[test]
    fn real_vanilla_chunk_decodes() {
        let dim_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../dist/RadServer/world/dimensions/minecraft/overworld");
        let pkg_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/package-check/versions/26.3");
        if !dim_dir.is_dir() || !pkg_dir.is_dir() {
            return;
        }
        let pkg = rad_core::version_package::LoadedVersionPackage::load_from_package_dir(&pkg_dir)
            .expect("load pkg");
        let provider = crate::registries::ArcPackageRegistryProvider(std::sync::Arc::new(pkg));
        for cx in -2..=2 {
            for cz in -2..=2 {
                if let Some(payload) =
                    crate::world::AnvilRegion::read_chunk_payload_checked(&dim_dir, cx, cz)
                        .expect("read payload")
                {
                    crate::world::anvil::ChunkColumn::decode_storage_bytes_checked_with(
                        &payload, &provider,
                    )
                    .unwrap_or_else(|e| panic!("decode real vanilla chunk ({cx}, {cz}): {e:?}"));
                }
            }
        }
    }
}
