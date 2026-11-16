//! Tracks a player's active block-breaking attempt.
use rad_core::types::BlockPos;
use rad_core::version_package::BlockMiningProperties;
use std::time::{Duration, Instant};

fn effective_tool(data: &BlockMiningProperties, held: Option<&str>) -> Option<(f64, u8)> {
    held.and_then(|item| {
        let item = item.strip_prefix("minecraft:")?;
        let (tier, tool) = item.split_once('_')?;
        if Some(tool) != data.effective_tool.as_deref() {
            return None;
        }
        match tier {
            "wooden" => Some((2.0, 0)),
            "stone" => Some((4.0, 1)),
            "iron" => Some((6.0, 2)),
            "diamond" => Some((8.0, 3)),
            "netherite" => Some((9.0, 3)),
            "golden" => Some((12.0, 0)),
            _ => None,
        }
    })
}

pub(super) fn can_harvest(data: &BlockMiningProperties, held: Option<&str>) -> Option<bool> {
    if !data.requires_tool {
        return Some(true);
    }
    let minimum = data.min_tool_tier?;
    Some(effective_tool(data, held).is_some_and(|(_, tier)| tier >= minimum))
}

/// A timing estimate based on explicit data. Required-tool blocks are left
/// unsupported until the package can describe the correct tool tiers.
pub(super) fn packaged_duration(
    data: &BlockMiningProperties,
    held: Option<&str>,
) -> Option<Duration> {
    if data.hardness < 0.0 || (data.requires_tool && data.min_tool_tier.is_none()) {
        return None;
    }
    if data.hardness == 0.0 {
        return Some(Duration::ZERO);
    }
    let tool = effective_tool(data, held);
    let speed = tool.map_or(1.0, |(speed, _)| speed);
    let can_harvest = can_harvest(data, held)?;
    let ticks = (data.hardness * if can_harvest { 30.0 } else { 100.0 } / speed).ceil();
    if !ticks.is_finite() || ticks > 1_000_000.0 {
        return None;
    }
    Some(Duration::from_millis((ticks as u64).saturating_sub(1) * 50))
}

/// Blocks that cannot be harvested in survival mode.
pub(super) fn survival_unbreakable(name: &str) -> bool {
    matches!(
        name,
        "minecraft:bedrock"
            | "minecraft:barrier"
            | "minecraft:end_portal"
            | "minecraft:end_portal_frame"
            | "minecraft:end_gateway"
            | "minecraft:command_block"
            | "minecraft:chain_command_block"
            | "minecraft:repeating_command_block"
            | "minecraft:structure_block"
            | "minecraft:jigsaw"
    )
}

fn tool_tier(held: Option<&str>, expected_tool: &str) -> Option<u8> {
    let item = held?.strip_prefix("minecraft:")?;
    let (tier, tool) = item.split_once('_')?;
    if tool != expected_tool {
        return None;
    }
    match tier {
        "wooden" | "golden" => Some(0),
        "stone" => Some(1),
        "iron" => Some(2),
        "diamond" | "netherite" => Some(3),
        _ => None,
    }
}

/// Returns `true` when `block` breaks instantaneously (`hardness == 0.0` or instant with `held_item`) in survival.
pub(super) fn is_instant_break_block(block: &str, held_item: Option<&str>) -> bool {
    if block == "minecraft:bamboo" && held_item.is_some_and(|i| i.ends_with("_sword")) {
        return true;
    }
    if block.ends_with("_leaves") && held_item == Some("minecraft:shears") {
        return true;
    }
    matches!(
        block,
        "minecraft:redstone_wire"
            | "minecraft:repeater"
            | "minecraft:comparator"
            | "minecraft:redstone_torch"
            | "minecraft:redstone_wall_torch"
            | "minecraft:torch"
            | "minecraft:wall_torch"
            | "minecraft:soul_torch"
            | "minecraft:soul_wall_torch"
            | "minecraft:copper_torch"
            | "minecraft:copper_wall_torch"
            | "minecraft:tnt"
            | "minecraft:slime_block"
            | "minecraft:honey_block"
            | "minecraft:scaffolding"
            | "minecraft:lily_pad"
            | "minecraft:tripwire"
            | "minecraft:tripwire_hook"
            | "minecraft:flower_pot"
            | "minecraft:azalea"
            | "minecraft:flowering_azalea"
            | "minecraft:spore_blossom"
            | "minecraft:short_grass"
            | "minecraft:tall_grass"
            | "minecraft:fern"
            | "minecraft:large_fern"
            | "minecraft:dead_bush"
            | "minecraft:dandelion"
            | "minecraft:poppy"
            | "minecraft:blue_orchid"
            | "minecraft:allium"
            | "minecraft:azure_bluet"
            | "minecraft:red_tulip"
            | "minecraft:orange_tulip"
            | "minecraft:white_tulip"
            | "minecraft:pink_tulip"
            | "minecraft:oxeye_daisy"
            | "minecraft:cornflower"
            | "minecraft:lily_of_the_valley"
            | "minecraft:wither_rose"
            | "minecraft:torchflower"
            | "minecraft:closed_eyeblossom"
            | "minecraft:open_eyeblossom"
            | "minecraft:sunflower"
            | "minecraft:lilac"
            | "minecraft:rose_bush"
            | "minecraft:peony"
            | "minecraft:pitcher_plant"
            | "minecraft:wheat"
            | "minecraft:carrots"
            | "minecraft:potatoes"
            | "minecraft:beetroots"
            | "minecraft:nether_wart"
            | "minecraft:melon_stem"
            | "minecraft:pumpkin_stem"
            | "minecraft:attached_melon_stem"
            | "minecraft:attached_pumpkin_stem"
            | "minecraft:sweet_berry_bush"
            | "minecraft:sugar_cane"
            | "minecraft:bamboo_sapling"
            | "minecraft:kelp"
            | "minecraft:kelp_plant"
            | "minecraft:seagrass"
            | "minecraft:tall_seagrass"
            | "minecraft:fire"
            | "minecraft:soul_fire"
            | "minecraft:nether_sprouts"
            | "minecraft:crimson_fungus"
            | "minecraft:warped_fungus"
            | "minecraft:crimson_roots"
            | "minecraft:warped_roots"
            | "minecraft:weeping_vines"
            | "minecraft:weeping_vines_plant"
            | "minecraft:twisting_vines"
            | "minecraft:twisting_vines_plant"
            | "minecraft:cave_vines"
            | "minecraft:cave_vines_plant"
            | "minecraft:small_dripleaf"
            | "minecraft:big_dripleaf"
            | "minecraft:big_dripleaf_stem"
            | "minecraft:decorated_pot"
            | "minecraft:pink_petals"
            | "minecraft:wildflowers"
            | "minecraft:leaf_litter"
            | "minecraft:bush"
            | "minecraft:firefly_bush"
            | "minecraft:cactus_flower"
            | "minecraft:short_dry_grass"
            | "minecraft:tall_dry_grass"
            | "minecraft:resin_clump"
    ) || block.ends_with("_sapling")
}

/// Vanilla fallback harvest check when `BlockMiningProperties` is absent from the version package.
pub(super) fn fallback_can_harvest(block: &str, held_item: Option<&str>) -> bool {
    let min_pickaxe_tier = match block {
        "minecraft:obsidian"
        | "minecraft:crying_obsidian"
        | "minecraft:netherite_block"
        | "minecraft:ancient_debris"
        | "minecraft:respawn_anchor" => Some(3),
        "minecraft:diamond_ore"
        | "minecraft:deepslate_diamond_ore"
        | "minecraft:diamond_block"
        | "minecraft:emerald_ore"
        | "minecraft:deepslate_emerald_ore"
        | "minecraft:emerald_block"
        | "minecraft:gold_ore"
        | "minecraft:deepslate_gold_ore"
        | "minecraft:gold_block"
        | "minecraft:raw_gold_block"
        | "minecraft:redstone_ore"
        | "minecraft:deepslate_redstone_ore" => Some(2),
        "minecraft:iron_ore"
        | "minecraft:deepslate_iron_ore"
        | "minecraft:iron_block"
        | "minecraft:raw_iron_block"
        | "minecraft:copper_ore"
        | "minecraft:deepslate_copper_ore"
        | "minecraft:copper_block"
        | "minecraft:raw_copper_block"
        | "minecraft:lapis_ore"
        | "minecraft:deepslate_lapis_ore"
        | "minecraft:lapis_block" => Some(1),
        "minecraft:stone"
        | "minecraft:cobblestone"
        | "minecraft:mossy_cobblestone"
        | "minecraft:deepslate"
        | "minecraft:cobbled_deepslate"
        | "minecraft:granite"
        | "minecraft:diorite"
        | "minecraft:andesite"
        | "minecraft:tuff"
        | "minecraft:calcite"
        | "minecraft:basalt"
        | "minecraft:blackstone"
        | "minecraft:netherrack"
        | "minecraft:nether_bricks"
        | "minecraft:end_stone"
        | "minecraft:coal_ore"
        | "minecraft:deepslate_coal_ore"
        | "minecraft:nether_quartz_ore"
        | "minecraft:nether_gold_ore"
        | "minecraft:coal_block"
        | "minecraft:quartz_block"
        | "minecraft:bricks"
        | "minecraft:stone_bricks"
        | "minecraft:furnace"
        | "minecraft:blast_furnace"
        | "minecraft:smoker"
        | "minecraft:dispenser"
        | "minecraft:dropper"
        | "minecraft:hopper"
        | "minecraft:anvil"
        | "minecraft:chipped_anvil"
        | "minecraft:damaged_anvil"
        | "minecraft:enchanting_table"
        | "minecraft:ender_chest"
        | "minecraft:stonecutter"
        | "minecraft:grindstone"
        | "minecraft:iron_door"
        | "minecraft:iron_trapdoor"
        | "minecraft:iron_bars"
        | "minecraft:cauldron"
        | "minecraft:bell"
        | "minecraft:lantern"
        | "minecraft:soul_lantern"
        | "minecraft:prismarine"
        | "minecraft:prismarine_bricks"
        | "minecraft:dark_prismarine"
        | "minecraft:amethyst_block" => Some(0),
        _ => None,
    };
    if let Some(req) = min_pickaxe_tier {
        return tool_tier(held_item, "pickaxe").is_some_and(|t| t >= req);
    }
    if matches!(block, "minecraft:snow" | "minecraft:snow_block") {
        return tool_tier(held_item, "shovel").is_some();
    }
    true
}

/// Computes the unmodified Java survival mining duration for soil whose
/// hardness is known. Unknown blocks have no inferred hardness.
pub(super) fn minimum_duration(block: &str, held_item: Option<&str>) -> Duration {
    if is_instant_break_block(block, held_item) {
        return Duration::ZERO;
    }
    let hardness: f64 = match block {
        "minecraft:dirt"
        | "minecraft:coarse_dirt"
        | "minecraft:rooted_dirt"
        | "minecraft:podzol" => 0.5,
        "minecraft:grass_block" | "minecraft:mycelium" => 0.6,
        _ => return Duration::from_millis(40),
    };
    let speed: f64 = match held_item {
        Some("minecraft:wooden_shovel") => 2.0,
        Some("minecraft:stone_shovel") => 4.0,
        Some("minecraft:iron_shovel") => 6.0,
        Some("minecraft:diamond_shovel") => 8.0,
        Some("minecraft:netherite_shovel") => 9.0,
        Some("minecraft:golden_shovel") => 12.0,
        _ => 1.0,
    };
    // Soil can be harvested by hand: progress per tick is speed / hardness / 30.
    // Allow one tick of scheduling tolerance when comparing network timestamps.
    let ticks = (hardness * 30.0 / speed).ceil() as u64;
    Duration::from_millis(ticks.saturating_sub(1) * 50)
}

#[derive(Default)]
pub(super) struct MiningAttempt(Option<(BlockPos, u16, u16, Instant)>);

impl MiningAttempt {
    pub(super) fn cancel(&mut self) {
        self.0 = None;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn start(&mut self, pos: BlockPos, state: u16, now: Instant) {
        self.0 = (state != 0).then_some((pos, state, state, now));
    }

    pub(super) fn start_block(&mut self, pos: BlockPos, block_id: u16, state: u16, now: Instant) {
        self.0 = (state != 0).then_some((pos, block_id, state, now));
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn finish(
        &mut self,
        pos: BlockPos,
        state: u16,
        now: Instant,
        minimum: Duration,
    ) -> bool {
        self.0
            .take()
            .is_some_and(|(started_pos, _, started_state, started)| {
                started_pos == pos
                    && started_state == state
                    && state != 0
                    && now.saturating_duration_since(started) >= minimum
            })
    }

    pub(super) fn finish_block(
        &mut self,
        pos: BlockPos,
        block_id: u16,
        state: u16,
        now: Instant,
        minimum: Duration,
    ) -> bool {
        self.0
            .take()
            .is_some_and(|(started_pos, started_block_id, _, started)| {
                started_pos == pos
                    && started_block_id == block_id
                    && state != 0
                    && now.saturating_duration_since(started) >= minimum
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_requires_matching_unchanged_block_and_consumes_attempt() {
        let p = BlockPos::new(1, 2, 3);
        let now = Instant::now();
        let mut attempt = MiningAttempt::default();
        let minimum = Duration::from_millis(40);
        assert!(!attempt.finish(p, 1, now, minimum));
        attempt.start(p, 1, now);
        assert!(!attempt.finish(p, 2, now + Duration::from_secs(1), minimum));
        assert!(!attempt.finish(p, 1, now + Duration::from_secs(1), minimum));
        attempt.start(p, 1, now);
        assert!(!attempt.finish(p, 1, now, minimum));
        attempt.start(p, 1, now);
        assert!(attempt.finish(p, 1, now + Duration::from_millis(50), minimum));
        attempt.start(p, 1, now);
        attempt.cancel();
        assert!(!attempt.finish(p, 1, now + Duration::from_secs(1), minimum));
    }

    #[test]
    fn soil_hardness_and_shovel_speed_determine_duration() {
        assert_eq!(
            minimum_duration("minecraft:dirt", None),
            Duration::from_millis(700)
        );
        assert_eq!(
            minimum_duration("minecraft:dirt", Some("minecraft:diamond_shovel")),
            Duration::from_millis(50)
        );
        assert_eq!(
            minimum_duration("minecraft:grass_block", None),
            Duration::from_millis(850)
        );
        assert_eq!(
            minimum_duration("minecraft:stone", None),
            Duration::from_millis(40)
        );
    }

    #[test]
    fn unbreakable_blocks_are_distinct_from_normal_blocks() {
        assert!(survival_unbreakable("minecraft:bedrock"));
        assert!(!survival_unbreakable("minecraft:dirt"));
    }

    #[test]
    fn package_hardness_and_tool_are_used_only_when_complete() {
        let mut data = BlockMiningProperties {
            hardness: 0.5,
            effective_tool: Some("shovel".into()),
            requires_tool: false,
            min_tool_tier: None,
        };
        assert_eq!(
            packaged_duration(&data, None),
            Some(Duration::from_millis(700))
        );
        assert_eq!(
            packaged_duration(&data, Some("minecraft:stone_shovel")),
            Some(Duration::from_millis(150))
        );
        assert_eq!(
            packaged_duration(&data, Some("minecraft:stone_pickaxe")),
            Some(Duration::from_millis(700))
        );
        data.requires_tool = true;
        assert_eq!(packaged_duration(&data, None), None);
        data.effective_tool = Some("pickaxe".into());
        data.min_tool_tier = Some(2);
        data.hardness = 1.5;
        assert_eq!(
            packaged_duration(&data, None),
            Some(Duration::from_millis(7450))
        );
        assert_eq!(
            packaged_duration(&data, Some("minecraft:wooden_pickaxe")),
            Some(Duration::from_millis(3700))
        );
        assert_eq!(
            packaged_duration(&data, Some("minecraft:iron_pickaxe")),
            Some(Duration::from_millis(350))
        );
    }

    #[test]
    fn instant_break_and_block_id_matching_work() {
        assert_eq!(
            minimum_duration("minecraft:redstone_wire", None),
            Duration::ZERO
        );
        assert_eq!(minimum_duration("minecraft:repeater", None), Duration::ZERO);
        assert_eq!(minimum_duration("minecraft:torch", None), Duration::ZERO);
        assert_eq!(minimum_duration("minecraft:tnt", None), Duration::ZERO);
        assert_eq!(
            minimum_duration("minecraft:oak_sapling", None),
            Duration::ZERO
        );
        assert_eq!(
            minimum_duration("minecraft:bamboo", Some("minecraft:iron_sword")),
            Duration::ZERO
        );

        let p = BlockPos::new(4, 64, 4);
        let now = Instant::now();
        let mut attempt = MiningAttempt::default();
        attempt.start_block(p, 42, 100, now);
        // State changes mid-mine (e.g. redstone_ore lit=true), same block_id 42
        assert!(attempt.finish_block(
            p,
            42,
            101,
            now + Duration::from_millis(50),
            Duration::from_millis(40)
        ));
    }

    #[test]
    fn fallback_harvest_checks_tool_tiers() {
        assert!(!fallback_can_harvest("minecraft:stone", None));
        assert!(fallback_can_harvest(
            "minecraft:stone",
            Some("minecraft:wooden_pickaxe")
        ));
        assert!(!fallback_can_harvest(
            "minecraft:iron_ore",
            Some("minecraft:wooden_pickaxe")
        ));
        assert!(fallback_can_harvest(
            "minecraft:iron_ore",
            Some("minecraft:stone_pickaxe")
        ));
        assert!(!fallback_can_harvest(
            "minecraft:diamond_ore",
            Some("minecraft:stone_pickaxe")
        ));
        assert!(fallback_can_harvest(
            "minecraft:diamond_ore",
            Some("minecraft:iron_pickaxe")
        ));
        assert!(!fallback_can_harvest(
            "minecraft:obsidian",
            Some("minecraft:iron_pickaxe")
        ));
        assert!(fallback_can_harvest(
            "minecraft:obsidian",
            Some("minecraft:diamond_pickaxe")
        ));
        assert!(fallback_can_harvest("minecraft:dirt", None));
        assert!(fallback_can_harvest("minecraft:oak_log", None));
    }
}
