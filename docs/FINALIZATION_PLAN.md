# RadByte — Overall Finalization Plan from Current State

Cutoff date: November 14, 2026, 14:56 Europe/London. Baseline: current codebase inspection, 112 unit/integration + 3 doc-tests PASS under RUST_LOG=warn.

This plan refines the priorities of the detailed C00–C12: do not repeat what is already done, do not close incomplete gates based on a single unit test. Target — a playable survival server suitable for real gameplay with reliable persistence, followed by vanilla parity. There is no promise to complete all of Minecraft in two hours.

## 1. P0 Correctness: Complete Dimensions and State Ownership

**Closes N01/N02. Crates: RadServer, RadCore.**

- Introduce WorldKey/DimensionKey for containers, furnace, entities, simulation queues, physics, and addon actions; remove the Overworld default from gameplay paths.
- UseItemOn/get_block/line-of-sight/collision/ground read the current session's dimension. SetBlock and damage use the same context.
- All simulation/entity/damage packets go only to observers of the corresponding dimension. WorldSim updates include dimension.
- Replace `155` in transfer visibility with generated PLAYER; scan for all new numeric IDs in production paths.
- Dimension transfer follows a single session-control path: menu closure, proper cache/tickets reset, Respawn, chunk streaming, teleport confirmation, observer rebuild.

**Acceptance:** Two clients in different dimensions at identical coordinates have distinct chest/furnace contents and collision. They do not see each other's blocks/entities/damage. After transfer and restart, the state remains correct; a new observer sees a player, not another entity.

## 2. P0 Data: Persistence, Shutdown, Migration

**Closes N05/N06/N08. Crates: RadServer, data tooling/scripts.**

- Combine cancellation/tick/session tasks into a managed lifecycle: halt input, await cleanup, snapshots, final save, flush, exit. Do not use map emptiness or fixed delays as proof of I/O completion.
- Coordinate direct player writes, autosave, /save, and quit; unique temp files and revisions/serial writer. Race tests must block disk I/O and intentionally inject mutations.
- Vanilla import reads block Name+Properties; preserves or rejects biomes, block entities, and unknown fields prior to write. Clearly separate RAD1 native storage from vanilla-compatible formats.
- Real migration CLI: dry-run, schema/source-registry detection, full backup of regions/DIM/player/container/furnace/metadata, semantic diff, and rollback. Do not resolve ambiguous legacy IDs by guessing; do not launch a quarantined world for writing without an explicit decision.
- Dedicated recovery rules for corrupt files and interrupted writes; snapshot boundary for block/container/inventory transactions.

**Acceptance:** world/player/container/entity state survives reconnect, new process launch, autosave, concurrent save, and managed shutdown. A known legacy fixture is actually scanned/migrated; collision in region payload is detected without an auxiliary marker file. Vanilla roundtrip does not alter orientation/waterlogging and does not lose metadata according to the declared contract. Backup is restored in a different directory with semantic read-back.

## 3. P1 Gameplay: Genuine Items, Menus, and Recipes

**Closes N04; completes C04/C09. Crates: RadCore, RadServer, RadAPI.**

- Implement domain ItemStack with component patch/defaults: names, durability, enchantments, effects/potions, stack size, and persistence. Wire decoder must not arbitrarily drop or discard standard supported items.
- Move recipes into runtime RecipeRegistry with namespaced IDs; connect generated vanilla data and supported datapack schemas to startup/reload/crafting.
- Single authoritative transaction for ingredients/output/cursor/container. An open menu has session/dimension/distance/state_id ownership.
- Chest multi-viewer synchronization; furnace input/fuel/output/progress/XP; all click modes and close/disconnect with item retention.
- Invalid/custom recipe yields an error/resync; invalid reload rolls back atomically.

**Acceptance:** A real client crafts tools and a furnace, smelts a resource, moves component-based items; two viewers concurrently modify a chest without dupes/loss. A datapack recipe actually alters runtime crafting results. Inventory/components/cursor policy survive restart.

## 4. P1 Simulation: Movement, Light, Projectiles, and Entities

**Closes N03/N07; completes C05–C07. Crates: RadServer, RadCore.**

- Server-tick movement budget with time accumulation, bounded tolerance, and packet-rate checks; shape collisions based on state/pose, fluids, and vehicles.
- Dynamic sky/block light, opacity/emission/neighbor propagation, matching heightmap predicates, and incremental packets.
- UseItem creates projectile in the server manager; manager is responsible for dimension, trajectory, collision/hit/damage/despawn. Bow charge/ammo/durability/cooldown and snowball consumption are verified.
- Persist/tracking metadata/attributes/equipment/loot for mobs/item entities; dimension-aware AI/navigation/spawning/despawning.
- Scheduled/random ticks and timers are persisted; queues have deduplication/limits. Simulation does not stall on chunk edges and does not generate unbounded world chunks via network input.

**Acceptance:** Packet spam does not speed up the character; stairs/slabs/plants/doors have expected shapes; torch/roof changes lighting; an arrow actually hits, deals damage, and despawns rather than merely displaying on the client. Simulation continuation after reload matches save policy.

## 5. P1 End-to-End Operational Proof for RadByte

**Closes N09; RadProxy/RadLimbo/RadCore/RadAPI/scripts. Also executed as the regression axis for Stages 1–4.**

- Fresh release binaries run from generated dist, source/data/binary fingerprints recorded in report.
- Actual external client 26.3: direct Server, Proxy→Server, Proxy→Limbo, switch in both directions, backend kick/TCP fallback, reconnect with preserved state.
- Matrix of none/zlib/various thresholds, Modern/Legacy/BungeeGuard according to the supported deployment scope; trusted peers/access/identity verified.
- Independent decoder verifies all required bootstrap/chunk/menu/entity/Respawn packets, not just IDs. GUI confirms actual appearance and interaction.
- Online auth when a legitimate account is available; otherwise NOT_RUN. Actual compiled WASM guests verify stateful/stateless contract, capabilities, limits, reload, and host actions across different dimensions.
- Reports do not carry over stale PASS results to modified source. External NOT_RUN remain visible; passing smoke does not equal G10/G12 full scope.

**Acceptance:** User executes documented Start/Test/Stop/Backup/Restore on a clean package and obtains identical results. No orphan tasks/processes or mutations after disconnect.

## 6. P2 Complete Baseline Survival

Following correctness gates: natural cycle of wood→tools→ore→furnace→food→shelter, tool tiers/durability/loot, hunger/exhaustion/armor/effects, death/respawn/spawnpoint/gamerules, world time/weather. Resources available in generated world without /give.

Expand biome/features/structures, navigation/mob behaviors, and item entities. Nether/End have gameplay transfer, resources, spawn safety, persistence, and progression, rather than merely visual terrain.

**Acceptance:** Two players conduct an extended survival session without administrative workarounds for mechanics; restart restores complete state. All constraints are recorded prior to the test, not added post-hoc.

## 7. P2/P3 Full Content and Vanilla Parity

Maintain inventory coverage against verified registry snapshot: each family of blocks/items/entities/recipes/menus/effects and corresponding behavior.

- Redstone: levels/connections/update order, repeaters/comparators/torches, pistons/observers, hoppers/dispenser/dropper, and their block entities.
- TNT/explosions: fuse/primed entity, resistance/drops, damage/knockback, packets, chain reactions.
- Enchanting/anvil/smithing/grindstone/brewing, potions/effects, fishing, farming/breeding/taming/trading, and remaining survival systems.
- Complete mobs/attributes/AI/families/vehicles/bosses and end progression.
- Datapacks/functions/loot/tags/predicates, advancements/stats, scoreboards/teams/bossbars, typed commands/selectors, and permission context.
- Vanilla-compatible world format and generation target; plan exact seed parity separately from proprietary deterministic generator.

**Acceptance:** Reference/differential suites for each group; unsupported items are not marked VERIFIED. Full vanilla parity must not be claimed based solely on file counts, enum variants, or milestone names.

## 8. P1/P2 Release Reliability

After and during each preceding stage: bounded resource budgets, malformed/fuzz/property tests for critical invariants, save fault injection, crash recovery/restore drill, soak/load with measurement of hardware/config and p50/p95/p99 tick latency/RSS/queue/save latency. Windows/Linux gates are separate.

Release target — 20 TPS under documented load; a unit test of 150 entities or status RPS does not prove capacity for real survival. Security deployment profiles/auth/trust/backend exposure must correspond to actual guarantees.

## Mandatory Milestones Instead of a Generic "100%"

| Milestone | Completed When |
|---|---|
| A — Correct multiplayer | Stage 1 and foundation of 4: dimensions/physics/visibility/menus isolated, new player is visible correctly |
| B — Safe persistence | Stage 2: backup/migration/save/restart/recovery with semantic evidence |
| C — Playable survival beta | Stages 3–6: real client can play without administrative substitution of gameplay |
| D — Network release candidate | Stages 5/8: packaged network, auth profile, WASM, load/restore, documented limits |
| E — Vanilla parity | Stage 7 and full reference coverage, including declared worldgen/storage target |

Work cycle: specific gap → implementation → regression → actual client/decoder → save/restart → evidence → next gap. Do not stop at a green verify alone. Do not create yet another basic implementation just for a "Done" mark; finish existing paths. Do not modify a valuable user world without backup/preview and explicit agreement on ambiguous migration.

Expected artifacts: updated FEATURE_READINESS with evidence and exact scope, tasks log for N01–N09, fresh tests/logs/manifests, reproducible generated data/migrations, up-to-date README/STORAGE_FORMAT, and dist. Current readiness assessments are in CURRENT_STATUS.md.
