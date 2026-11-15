# RadByte: Feature Catalog and 26.3 Completion Checklist

Snapshot: November 15, 2026. This is a work catalog, not a claim that every item is missing or present. The previous CURRENT_STATUS.md is dated November 14, contains outdated paths, and requires re-verification. The presence of an ID, name, test, or generated table does not prove behavioral implementation.

## Researched Projects

| Project | Public Claims | Useful Aspects to Study |
|---|---|---|
| [SteelMC](https://github.com/Steel-Foundation/SteelMC) | Rust server for 26.3, pre-alpha; authors explicitly report incomplete survival. | Implementation status registry, boundaries of synchronous ticks, and asynchronous chunk servicing. |
| [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) | Minecraft Rust server in development. | Organization of block subsystems, test scenarios, and development structure; verify specific implementation and version before drawing conclusions. |
| [FerrumC](https://github.com/ferrumc-rs/ferrumc) | Rust server focused on performance; authors describe a clean-room approach and limited generation. | Server architecture, thread management, and honest differentiation between performance and exact compatibility. |
| [Valence](https://github.com/valence-rs/valence) | Server framework using ECS; vanilla mechanics are not automatically complete. | Modular APIs, network entities, and block properties, rather than ready-made vanilla survival. |
| [Azalea](https://github.com/azalea-rs/azalea) | Client and bot libraries, not a server; current README indicates 26.2. | Automated client, movement, and interaction ideas; 26.3 compatibility must be verified separately. |

Conclusions are based on public descriptions, not on an exhaustive audit of their blocks or compatibility levels. Copying code, tables, or resources is permissible only after a separate verification of license, provenance, terms, and compatibility, preserving attribution; the existence of an open repository does not make all data automatically usable. For clean-room development, use independent specifications and tests in accordance with [PROVENANCE_CLEAN_ROOM.md](PROVENANCE_CLEAN_ROOM.md); do not import questionable sources.

## Methodology and Tracking

The items below represent requirements; `[ ]` signifies "not confirmed for this document", not necessarily "unimplemented". At the start of execution, record for each item: `ID | version | status | code/source | tests | GUI | restart | limitations`. Statuses: `NOT_ASSESSED`, `MISSING`, `PARTIAL`, `IMPLEMENTED_UNVERIFIED`, `VERIFIED`, `BLOCKED`, `NOT_APPLICABLE`. Check off an item only according to the acceptance criteria; do not carry over results from an older build to a new one. For blockers, record the cause and next step.

Generate the matrix of all registry IDs, states, and variants from a verified snapshot of 26.3 specifically, without inventing a manual "complete" inventory. For each block, separately evaluate state, item mapping, placement, interaction, collision, support, update, drops, tool, fluid, ticks, redstone, block entity, network, and persistence where applicable. The families below form a behavioral map rather than an exhaustive registry ID inventory.

## A. Networking, Access, and Server Lifecycle

- [ ] A01 Status, ping, handshake, protocol mismatch, correct rejection messages.
- [ ] A02 Login, online/offline identity, encryption, compression, configuration registry/tags, login finish.
- [ ] A03 Play bootstrap, initial chunks/light/position, teleport confirmation, keepalive, clean disconnect.
- [ ] A04 Compression, frame/NBT/string/packet limits, invalid packets, timeout, backpressure, rate limits.
- [ ] A05 Max players, whitelist, ops, ban/IP ban, permissions, and available commands synchronization.
- [ ] A06 Correct tick, connect/disconnect lifecycle, task shutdown, final save, and bounded shutdown.
- [ ] A07 Multiplayer visibility, dimension isolation, player list, chat, and reconnect without desync.

## B. World, Chunks, and Data Reliability

- [ ] B01 Chunk sections, palettes/block states, heights, signed coordinates, chunk boundaries, view/simulation distance.
- [ ] B02 Safe loading/unloading, chunk tickets, broadcasting and updates sent only to relevant players.
- [ ] B03 Time of day, weather, difficulty, gamerules, spawn, world border, and their persistence.
- [ ] B04 Atomic/consistent writes, autosave, save-all, shutdown barrier, concurrency, and write failure recovery.
- [ ] B05 Playerdata: UUID, position, dimension, state, inventory/components, health, experience, spawn, options.
- [ ] B06 Containers/block entities/entities, scheduled ticks, and runtime state: define and test persistence policy.
- [ ] B07 level.dat, region/Anvil, DataVersion; document actual format and unknown data handling. Do not claim lossless vanilla roundtrip without verification.
- [ ] B08 Backup/restore, corrupt data, power-loss recovery, unknown fields, migrations/rollback, disallow unsupported downgrade.
- [ ] B09 Overworld/Nether/End: total state isolation, respawn/transfer, portals, safe transit, and persistence.
- [ ] B10 Seed, flat/default generation, biomes, structures, ores, caves, and resources; seed parity separately from custom generation.
- [ ] B11 Sky/block light: sources, opacity/transparency, updates, chunk borders, client data, and persistence.

## C. Basic Block Actions and Physics

- [ ] C01 All registry IDs/state properties/default states, correct serialization and validation.
- [ ] C02 Placement: face, player direction, replacement, reach/line-of-sight, bounds, collision, permission, and consumption.
- [ ] C03 Breaking: gamemodes, hardness, progress, tool/effects, cancellation, drops, XP, block entity.
- [ ] C04 Pick block in creative, hotbar/held items, special data where supported.
- [ ] C05 Outline/selection, interaction, collision, and occlusion shapes per state; do not conflate shape types.
- [ ] C06 Slabs/stairs, fences/walls/gates, doors/trapdoors, carpets/snow layers, ladders/vines, plants, fluids, partial blocks.
- [ ] C07 Movement: standing, step/jump against walls, falling, pose, swimming/climbing, sneaking, push, and stuck recovery.
- [ ] C08 Server-authoritative movement budget, collision/reach validation, teleport, and packet spam protection.
- [ ] C09 Neighbor updates in deterministic order: support change/removal, cross-chunk, deduplication, and queue bounds.
- [ ] C10 Scheduled/random ticks: chunk loading, work budget limits, restart, and proper dimension.
- [ ] C11 Sand/red sand/gravel/concrete powder/anvils: falling, client entity, landing, drops, contacts, and restart.
- [ ] C12 Water/lava: sources/flow, waterlogging, interactions, neighbor updates, flow forces, safe boundaries, and persistence.

## D. Block Families and Behaviors

- [ ] D01 Building blocks: logs/pillars orientation, directional blocks, slabs/stairs, connections of fences/walls/panes.
- [ ] D02 Two-part doors, trapdoors, gates: facing, halves, open/powered, locking, and neighbor interactions.
- [ ] D03 Torches, lanterns, buttons, levers, signs, paintings, and other attached blocks: proper support, face, and drop.
- [ ] D04 Beds: both halves, orientation, sleeping, spawnpoint, obstructions, dimension-rule explosions.
- [ ] D05 Plants, saplings, leaves, crops, farmland: support, age, random ticks, lighting, growth, drops.
- [ ] D06 Mushrooms, cactus, sugar cane, bamboo, vines, moss, coral, and aquatic plants: specific placement/growth rules.
- [ ] D07 Dirt, grass, mud, snow/ice: spreading, melting/freezing, and conditional interactions.
- [ ] D08 Fire/campfires/soul fire: spread, extinguishment, support, damage, and water interaction.
- [ ] D09 Chests/barrels/shulker: inventory, block entity, opening, state, persistence, breaking, and viewers.
- [ ] D10 Crafting table/furnace/blast furnace/smoker: menu, input/fuel/output, progress, recipes, XP, and persistence.
- [ ] D11 Signs/hanging signs, bookshelves/lecterns, jukebox, note block, bells: interaction, data, sound/events.
- [ ] D12 Beds, banners, skulls, pots, decorated pots, item frames, and other data-bearing objects: states and data.
- [ ] D13 Beacon, conduit, cauldron, composter, beehive, spawner: conditions, inventory/state, timers, and effects.
- [ ] D14 Nether/End portals, end gateway, dragon egg, respawn anchor: versioned rules and transit safety.
- [ ] D15 Block tags, loot tables, sounds/particles, blast resistance, tools, support, and properties of each family.

## E. Redstone and Mechanisms

- [ ] E01 Sources/receivers, weak/strong power, directions, 0–15 levels, attenuation, and update order.
- [ ] E02 Redstone dust: shapes, slopes, connections, powering, support removal, chunk borders, and loops.
- [ ] E03 Torch, lever, button, pressure plates, tripwire, target, daylight sensor, and their timings/directions.
- [ ] E04 Repeater: delay, lock, state, orientation, and scheduled updates.
- [ ] E05 Comparator: compare/subtract, side/rear input, container signal, reading, and state updates.
- [ ] E06 Piston/sticky piston: facing, extension/retraction, push limit, immovable blocks, adhesive rules, and obstruction handling.
- [ ] E07 Piston head/moving piston: animation, collisions, entities, chunk border, block entity/persistence, and interrupter cases.
- [ ] E08 Observer: direction, change sources, pulse, delay, and loop without infinite hang.
- [ ] E09 Hopper, dispenser, dropper: inventory transfer, cooldown, power, interactions, viewers, and restart.
- [ ] E10 Redstone lamp, door, trapdoor, gate, note block, TNT, rail, and other receivers: edge/pulse semantics.
- [ ] E11 TNT: ignition, fuse entity, explosion resistance, damage, knockback, drops, chain reactions.
- [ ] E12 Common contraptions: line, vertical transmission, repeater clock, comparator container, piston door, observer farm.

## F. Items, Menus, and Survival

- [ ] F01 Registry IDs, stacks, maximum stack size, components/defaults, enchantments, durability, custom data, and wire format.
- [ ] F02 Inventory/hotbar/offhand/armor/cursor, state ID synchronization, resync, and reconnect.
- [ ] F03 Clicks: left/right, shift, number key, drag, double-click, creative actions, Q/Ctrl+Q, item pickup, and death.
- [ ] F04 Atomic transactions for two players/menus, absence of dupes and losses on disconnect, save, and restart.
- [ ] F05 Crafting 2×2/3×3, recipe book, shaped/shapeless, remainders, data packs, vanilla recipes per snapshot.
- [ ] F06 Furnaces and containers: concurrent viewers, fuel, recipes, progress, XP, persistence, and automation.
- [ ] F07 Tools, tiers, mining speed, durability, correct drops, bucket, shears, hoe, shovel, axe, flint and steel.
- [ ] F08 Food, hunger, saturation, exhaustion, healing, starvation, status effects, and their persistence.
- [ ] F09 Damage sources, armor, knockback, fall, fire, drowning, suffocation, death, drops, and respawn.
- [ ] F10 Enchanting, anvil, smithing, grindstone, brewing/potions: UI, recipes, costs, components, and persistence.
- [ ] F11 Fishing, farming, breeding, taming, trading, and full survival progression from resources to the End.

## G. Entities and Players

- [ ] G01 Spawn/despawn, tracking, ID/UUID, metadata, attributes, equipment, dimension, and persistence.
- [ ] G02 Item entities: spawn, movement, pickup delay, merging, inventory, despawn, network updates.
- [ ] G03 Mobs by family: spawning, navigation, AI behavior, attack, damage, loot, despawn, persistence.
- [ ] G04 Combat system: cooldown, reach, attack, critical hits, shield, effects, armor, death, and multiplayer consistency.
- [ ] G05 Projectiles: ammo, spawning in server simulation, trajectory, collision, hit/damage, removal, and persistence policy.
- [ ] G06 Vehicles/mounts, boats/minecarts, passengers, movement, and sync.
- [ ] G07 Bosses, villagers, villagers' trades, raids, and special entity mechanics per specific scenarios.

## H. Commands, Extensions, and Platform

- [ ] H01 DeclareCommands/Brigadier tree, permissions, suggestions, parsing, selector/coordinates/errors.
- [ ] H02 Administration: op, whitelist, ban, kick, save, stop, gamemode, teleport, time/weather, gamerules.
- [ ] H03 World commands: give, clear, setblock, fill, clone, summon, execute, data, and others — per command with tests.
- [ ] H04 Scoreboards, teams, bossbars, advancements, statistics, functions, and datapacks/reload.
- [ ] H05 RadAPI v1: capability/permission model, WASM lifecycle, limits/fuel, isolation, error handling, reload.
- [ ] H06 Events/hooks: player, block, inventory, world, entities, commands; explicit ordering and safety model.
- [ ] H07 RadProxy: Status, login, forwarding modes/trusted peers, backend switching, fallback, permissions, protocol limits.
- [ ] H08 RadLimbo: login/configuration/play, real-client compatibility, keepalive, and clean termination.
- [ ] H09 Configurations, logs, metrics, network security, documentation, and operational deployment examples.
- [ ] H10 Version packages, manifest/ABI/data validation, version-specific rules, and safe world compatibility.
- [ ] H11 Performance/load/soak, memory/queue bounds, p95/p99 tick and save latency, benchmarks on documented hardware.
- [ ] H12 Windows/Linux/macOS CI separated, release packaging, checksum, backup/restore, provenance and license verification.

## Mandatory Acceptance Template for Each Feature

```text
ID / name / Minecraft version:
Status and date:
Behavior and scope (including gamemode and dimension):
Specification source / data provenance:
Code:
Unit/property tests:
Network integration and multiplayer:
World/player after restart, chunk border, and dimension (if applicable):
Reference comparison / expected values:
Real client: PASS / FAIL / NOT_RUN and version:
Known limitations / next step:
```

Especially for blocks: test placement on every face, state/rotation, interaction, support, shapes, neighbor changes, drops, chunk borders, multiplayer, and reload where applicable. For redstone, include temporal ordering, power levels, delay, cycles, and counterexamples. Do not check off an item solely for having a type stub or placeholder unit test.
