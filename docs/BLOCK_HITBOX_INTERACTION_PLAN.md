# Plan: Hitboxes, Collisions, and Block Activation in Minecraft 26.3

## Goal

Eliminate player snagging/stuck states, false movement rollbacks/rubber-banding, and the inability to click/activate blocks. For each Minecraft Java 26.3 (protocol 777) block state, ensure verified collision and interaction shapes, player and entity behavior, synchronization, and persistence.

"Hitbox" in this plan refers to several distinct concepts: collision shape for physics, outline/selection shape for targeting, interaction shape for clicking, support shape for resting, and occlusion/lighting shape. These can differ. The client also performs its own targeting; server refusal to activate a block may arise from packet ID, reach, state, gamemode, or missing mechanics, rather than shape alone.

## What Is Confirmed by Current Code

- `RadVersion26_3/src/world/physics.rs::block_collision_boxes` contains several special shapes and returns a full cube as a fallback. Stairs are explicitly approximated by their bottom half; fences/walls by a single central box. Such approximations do not demonstrate vanilla compliance across all states.
- `Aabb::intersects_solid_in` and `validate_player_move_with_budget_in` participate in movement validation. Verify setback corrections, body position, epsilon, step height, and post-collision recovery.
- `UseItemOn` in `RadVersion26_3/src/net/play.rs` performs separate reach checks and branches into mechanics. The presence of a collision shape does not guarantee block activation.
- `RadCore/data/mc777/PROVENANCE.json` lists 1,286 block entries in the current dataset. This is the count of entries, not the count of verified hitboxes; a single block can have many states. Data provenance requires an independent legal audit (`docs/PROVENANCE_CLEAN_ROOM.md`).
- At the time of preparation, no complete independent mapping of "every block state → vanilla shape → verified result" was found. Therefore, a reliable percentage of correctly working blocks is currently undetermined.

## 1. Safe Reproduction and Measurement

1. Read AGENTS.md, inspect the latest code, third-party changes, test baseline, and running servers. Use a separate world/port; do not modify the user's saves, addons, configs, or secrets.
2. Reproduce the screenshot issues on a real client: record name, block-state properties, position, clicked face, gamemode, world/dimension, action, outcome, and server/client logs. Do not assume the mechanism failure is caused solely by collision shape.
3. Assemble an inventory of all block types and **all of their states** from the version registry and a verified independent source. Build a machine-readable table: block, properties, state ID, collision, outline/interaction, support, status, provenance, test, client verification.
4. Classify each state as `verified`, `approximate`, `missing`, `not-applicable`, or `unknown`. Do not count the `FULL` fallback as confirmed merely because some blocks are indeed full blocks.
5. Define metrics: percentage of verified block-state collisions, interaction/activation separately, real client separately. By block type, consider a type fully verified only when all its relevant states have been verified. Publish numerator, denominator, and unknown cases.

## 2. Source Data and Shape Architecture

1. Reconcile Minecraft 26.3 rules against acceptable sources and establish the provenance/distribution rights of fixtures and geometry. Explicitly discuss independent data collection; do not label extracted data as clean-room.
2. Create a typed VoxelShape model as a collection of AABBs supporting empty, full, and compound shapes, values exceeding block boundaries, and state-dependent orientation. Validate bounds, finite values, and handle unknown states safely.
3. Separate collision shapes, targeting/interaction shapes, support shapes, and lighting shapes. For entity context, account for players, items, mobs, vehicles, projectiles, posture, and applicable vanilla behavior rules.
4. Build a reproducible generator/lookup table of `block state → shape` with versioning, hash, provenance, and validation. Avoid unverified state numbering formulas and silent replacement of unknown states with a full block.
5. Bound memory consumption and lookup time (shape deduplication and indices), minimize redundant world locks, and control chunk loading.

## 3. Priority Blocks Causing Player Snagging

1. Slabs: top, bottom, double, waterlogged, every material and state.
2. Stairs: directions, top/bottom, straight/inner/outer, neighbors, water, step-up.
3. Fence/wall/gate: each connection side, height, and open state; railings, panes, doors, and trapdoors in open/closed states.
4. Carpet, snow layers, trapdoor, pressure plate, button, lever, redstone dust/repeater/comparator, chest, hopper, cauldron, lectern, bell, and other partial and compound shapes. Distinguish selection shapes from collision.
5. Water/lava, plants, cobweb, climbable blocks, powder snow, honey/slime, scaffolding, moving pistons, and falling blocks — distinct entity context, friction/speed/special effects according to confirmed behavior.
6. Verify that the user does not get stuck in an existing saved world; do not move or delete blocks or players automatically in ways that appear safe but are destructive. Develop recovery mechanisms for illegal/stale positions without introducing wall-phasing exploits.

## 4. Server Physics and Movement Validation

1. Perform collision, axis resolution, step height, jump, landing, head clearance, sneaking, swimming/crawling, flight/spectator, knockback, and moving blocks according to player gamemode and dimensions.
2. Reconcile client prediction with server corrections: accurate positions, velocities, teleport confirmation, and prevention of infinite rubber-banding.
3. Verify collisions with blocks extending beyond their voxel (walls, fences, etc.), at negative coordinates, and across chunk/dimension boundaries.
4. Do not weaken safeguards: test against noclip movement, excessive speed, NaN/Infinity, out-of-bounds coordinates, mid-movement state changes, and network latency.
5. Falling blocks, items, mobs, and projectiles must use shapes appropriate to them rather than an arbitrary player shape. Maintain correct state across restart and simulation-distance without losing or duplicating blocks.

## 5. Activation and Interaction

1. Reconcile `UseItemOn`, `PlayerAction`, hit vector, hand, face, sequence, and acknowledgement with protocol 26.3; separately verify creative/survival, crouching, permission/spawn protection, range, line of sight, and dimension.
2. For redstone dust, repeaters, comparators, lever/button, doors, trapdoors, chests, furnaces, crafting tables, lecterns, and other blocks, add or fix distinct server-side interactions, block-state updates, scheduling, and UI.
3. Ensure that on rejection, the client receives correct resynchronization, while items and block entities are not lost. Two clients must observe identical state.
4. Do not claim that all mechanisms work solely after fixing geometry: each requires verified redstone, inventory, tick, and persistence rules.

## 6. Tests, Coverage, and Acceptance

- Automated shape tests for all states against an independent reference table, rather than tests echoing values of the same implementation.
- Player movement routes through corners, stairs, slabs, fences, doors, pistons, torches, and redstone; point-blank jumps, chunk transitions, two dimensions, different gamemodes, and network latency.
- Interaction tests: clicks on all shape faces/boundaries, reach, crouch, permissions, signal changes, two players, remote client, and visible outcome.
- Save/restart of world and player; atomicity, entity/item persistence, testing legacy worlds only on copies, backup/restore.
- Performance tests for high-volume state lookups and long-run TPS, parser limits, fuzz/property testing as needed, and deadlock freedom.
- Real 26.3 client and, where feasible, comparison with vanilla; if GUI or reference source is unavailable, mark as `not verified` and proceed with independent tasks.
- Update the report with numerators/denominators and methodology, run `scripts/verify.ps1` after reviewing side effects, and assemble synchronized executable/version DLL/data package in `dist`. Cleanly save and stop the production server only when necessary; leave launching to the user.

## Execution Order

M0 coverage and reproduction → M1 exact shape model → M2 critical stuck states → M3 interaction/activation → M4 remaining families → M5 parity and scale testing. The implementer does not stop after a single block, but transparently reports any incomplete states and does not label approximations as full vanilla implementation.
