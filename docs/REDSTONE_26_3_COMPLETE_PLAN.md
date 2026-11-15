# Minecraft Java 26.3: Redstone Tab, Mechanics, and RadByte Plan

## Source and Scope of the List

During preparation, a local Minecraft Java 26.3 client was reviewed: `net.minecraft.world.item.CreativeModeTabs`, specifically the `REDSTONE_BLOCKS` tab creation method, via field names in bytecode. This document outlines the **key elements and groups** that it populates, alongside functional requirements for RadByte. The table is not a copy of Mojang code nor a claim that the mechanics are already operational. Collections of colors, oxidation stages, waxed states, and wood types must be expanded and verified in a separate report using a real client: a simple list of field references does not prove the total number of variants. The creative inventory tab and **all blocks interacting with redstone signals** are not identical sets: both requirement types are accounted for below where noted.

## 1. Tab Contents and Required Features

### Signal and Conductors

| Element or group | Mechanics to implement and verify |
|---|---|
| Redstone dust | Placement, support, dot/line/corner/cross branching shapes, `none/side/up` sides, 0–15 signal, attenuation, multiple sources, recalculation on neighbor change, client states, and drops. |
| Redstone torch and wall variant | Proper support and orientation, inversion, delay, neighbor updates, lockout checks, and burnout per 26.3 rules. |
| Block of redstone | Constant power source, interaction with dust and mechanisms, piston movement. |
| Repeater | Direction, input/output, 4 delay settings, side-signal locking, scheduled ticks. |
| Comparator | Direction, compare/subtract modes, front and side inputs, analog signal, scheduled updates, and container output. |
| Target | Signal strength based on hit position, duration, and various projectiles. |
| Copper bulb, variants | State toggle on pulse, powering, light, oxidation/waxing, and persistence. |

### Sources and Sensors

| Element or group | Mechanics |
|---|---|
| Lever | Orientation, support, toggling, signal, and neighbor updates. |
| Oak/stone button and corresponding variants | Support/direction, timed pulse, material differences, and projectile interactions if applicable. |
| Oak/stone pressure plate | Mob/player and other entities, recheck delay, support. |
| Light/heavy weighted pressure plate | Analog signal strength based on entity count and plate type. |
| Sculk sensor and calibrated sculk sensor | Vibrations, filtering/frequency, cooldown, signal transmission, and interaction with wool. |
| Sculk shrieker | Response to sensor signal, warnings/consequences, and summon conditions — verify against 26.3. |
| Amethyst block | Resonance and vibration interaction per vanilla; normal block physics. |
| Wool, color collection | Vibration occlusion/dampening based on placement; all colors. |
| Tripwire hook and string | Direction and support, string line connection, entities, powering both ends, breaking, and tools. |
| Lectern | Book, page, redstone pulse, and analog comparator output, inventory. |
| Daylight detector | Light, time of day, inverted mode, periodic updates, and signal. |
| Lightning rod, state collection | Lightning strike/signal timing, direction, oxidation, and waxing if applicable. |
| Observer | Observation direction, state change detection, pulse, and delay. |

### Movement and Item Interaction

| Element or group | Mechanics |
|---|---|
| Piston, sticky piston | Facing/extended, signal, delay, head/moving state, push limit, immovable/fragile blocks, entity movement, sticky pull, persistence, and container safety. |
| Slime block, honey block | Sticky rules and exceptions, interaction with pistons and entities; movement dependencies/branches. |
| Dispenser, dropper | Facing, signal and edge trigger, delay, random slot, difference between using and dropping items, all supported item behaviors. |
| Crafter | Grid, disabled slots, recipe, pulse, item output, comparator, concurrency, and persistence. |
| Hopper | Input/output, direction, transfer cooldown, power lockout, item entities, two containers, minecart, comparator. |

### Containers and Measurable Blocks

Chest, copper chest (including state collection), barrel, chiseled bookshelf, oak shelf, furnace, trapped chest, jukebox, decorated pot, composter, and cauldron are present in the tab. For each, verify placement, block entity, opening/closing, inventory or persistent state, hopper interaction, comparator output, client updates, and restart. For trapped chest, additionally verify signal strength based on player count; for shelf — analog values for its contents according to vanilla 26.3. Containers that lack such interactions in vanilla should not gain them by assumption.

### Transport and Mechanisms

| Element or group | Mechanics |
|---|---|
| Rail, powered rail, detector rail, activator rail | Connection/shape/slopes, support, signal, neighbor updates, minecart interaction. |
| Minecart, hopper/chest/furnace/TNT minecart | Rail physics, acceleration, contents, transfer, fuel/explosion, entity sync, and persistence. |
| Oak chest boat, bamboo chest raft | Movement/steering, inventory, hopper interaction, and persistence; verify material list in the creative tab via the client. |
| Oak/iron door, oak fence gate, oak/iron trapdoor | Facing, half/hinge, signal, manual opening per material, multi-block integrity, and neighbor updates. |
| TNT | Ignition from various sources, fuse entity, explosion, blocks/entities, drops, and chain reactions. |
| Redstone lamp | Toggling on/off, delay, lighting, neighbors, client updates. |
| Bell | Ringing/direction, signal, sound and effects, interactions. |
| Big dripleaf | Tilt state, entities, timings, and holding state under signal according to vanilla rules. |
| Armor stand | Entity, interaction/pose, redstone/mechanism interaction per vanilla rules. |
| Redstone ore | Ore states and interactions, light, and corresponding updates; do not assume it is a regular power source. |

**Important:** The list of names above is an overview of fields in the tab method, not proof that the listed features are implemented in RadByte. Properties, delays, signal shapes, and nuances must be verified against Java 26.3 rather than assumed by name.

## 2. Initial RadByte Verification

1. Read AGENTS.md and current changes, build and verify the baseline.
2. Trace `WorldSim`, `RedstoneWireState`, `update_redstone_around_in`, block placement/break, item↔block mapping, `simulation-distance`, `world/physics`, container data, network packets, and persistence.
3. Confirm actual behavior; do not rely on function names and old documents. In particular, separately verify 0–15, queues, and support logic already present in code.
4. Create a matrix across all base elements and variants: `missing / partial / tested / verified against vanilla / GUI verified`, with files and tests.
5. Reproduce the screenshot: straight and diagonal wires, support presence, redstone block, and updates after support removal. Do not treat diagonal wires as connected without vanilla confirmation.

## 3. Shared Architecture

1. A single verified block modification pipeline: placement, break, command, fluid, redstone, piston, explosion, event/API, chunk load, and recovery.
2. Versioned mapping of block name + properties ↔ state ID, schemas for all variants, without assumptions about arithmetic order of numeric IDs.
3. Bounded iterative queue of neighbor updates/scheduled ticks: verified order, dedup, no starvation, no silent drops, with diagnostics.
4. Semantics of active/inactive chunks, dimension boundaries, and task persistence; do not load arbitrary areas via an uncontrolled update graph.
5. Atomic block/inventory/entity transitions, revision snapshots, without blocking on network or disk while holding a world lock. Two clients receive consistent states, and a slow client does not stall the tick loop.

## 4. Execution Order

**M0 — Baseline and coverage.** Compile an exact machine-readable list of the tab with all variants, compare with the vanilla client; produce an implementation matrix and tests.

**M1 — Dust and support.** Proper state properties, dot/cross, straight and vertical connections, 0–15, two sources, support breaking/drop, cycle, and network changes. Fix both visualization and server-side results.

**M2 — Sources, lamp, and sensors.** Lever, button, plates, redstone torch, redstone block, target, lamp, daylight detector, observer; timing and support.

**M3 — Delays and measurements.** Repeater, comparator, and analog container outputs, lectern, composter, cauldron, and other verified blocks.

**M4 — Pistons.** Facing, power, and head, push/pull, limits, slime/honey, entity collisions, chunk boundaries, moving-block persistence, and rollback without duplication. No block entity removal without safe transfer.

**M5 — Item mechanisms.** Dispenser/dropper/crafter/hopper, containers, item behavior interactions, concurrent openings, signals, and persistence.

**M6 — Transport and moving entities.** All rails, minecarts, and chest boats; power signals, movement, inventory, and restart.

**M7 — Remainder of the tab.** Sculk/vibrations, copper states, jukebox, bell, dripleaf, armor stand, TNT, doors/trapdoors/gates, ore, and other variants.

**M8 — Vanilla parity verification.** For each family, document discrepancies against Java 26.3, resolve them, and record headless and real-client results.

## 5. Criteria for Each Component

- Correct placement, facing, support, item/drop, state shapes, and client ID.
- All inputs/outputs and ranges, neighbor changes, reactivation, timing delays, cycles, and concurrent sources conform to verified rules.
- State, block entity, inventory, pending ticks, and moving entities survive a clean restart; errors neither duplicate nor lose the world or items.
- Tests cover partial blocks, adjacent chunks, negative coordinates, two dimensions and players, high load, slow clients, and invalid packets.
- Missing/unverified features are visible in coverage rather than represented as complete.

## 6. Integration and Release

Protect working worlds, configs, and secrets. Work in temporary worlds; before running `scripts/verify.ps1`, read its side effects. Run fmt, clippy, unit/E2E tests, documentation, release build, and isolated smoke tests on `dist`. Check 2 clients on 26.3, direct/proxy, chunks `view=12/simulation=8`, restart, and backup/restore. If a real client is unavailable, state this explicitly. Update executable, version DLL, and data package together, leaving the server stopped for the user. The complete tab is a large scope: do not consider it done after a single passing test, do not halt independent work after an intermediate milestone, and honestly state remaining items at the end.
