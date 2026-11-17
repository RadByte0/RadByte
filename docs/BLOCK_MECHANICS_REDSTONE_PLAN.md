# Plan: Block States, Redstone, and Mechanisms in Minecraft 26.3

## Goal

Fix block appearance and behavior, particularly redstone dust connections, and progressively implement verified mechanics for Minecraft Java 26.3 / protocol 777. The success criterion is that a block has the correct state visible to all clients, exhibits correct interaction, and persists across disconnects and restarts. The presence of an ID in the registry does not imply implementation of the mechanic.

## Current Code Findings

- In the screenshot example, dust appears as disconnected dots. Before treating this as a bug, reproduce the layout and check adjacency: vanilla redstone does not connect diagonal blocks automatically.
- `RadVersion26_3/src/gameplay/inventory.rs::item_id_to_block_state` returns the default state for the corresponding block; this is insufficient for states that depend on face, player orientation, or neighbors.
- `RadVersion26_3/src/world/sim.rs::toggle_lever_in` currently traverses neighbors and distinguishes between two wire states and lamp states. It is necessary to verify all states, signal strength, connections, multiple sources, and updates following neighbor changes.
- `REDSTONE_WIRE_RANGE` contains many state IDs; one cannot infer that the first and last ID cover all necessary combinations based on power alone.
- These are observations from the codebase, not assertions of full mechanical completeness. Before implementation, check recent changes and associated tests.

## Execution Rules

Work against the current repository state and AGENTS.md without modifying external edits. Use a separate test world and port. Do not overwrite live worlds, playerdata, configs, or secrets without a backup. Verify behavior and block-state properties against vanilla 26.3; respect data licensing constraints. Do not declare feature support merely because an ID exists; do not introduce placeholder/fictitious states.

## 1. Reproduction and Independent Baseline

1. In a temporary world, reproduce a straight line of dust, a corner, a T-junction, a diagonal line, and connections to a redstone block, lever, and lamp. Record actual block-state IDs and client rendering before and after neighbor changes.
2. Replicate identical configurations on a vanilla 26.3 server where possible; utilize registry and state data as a distinct source, without inferring behavior purely from ID ordering.
3. Verify that `block update`, chunk data, and persisted states match; distinguish server-side state errors from client desynchronization.
4. Add independent fixtures for `north/east/south/west`, power, and `none`, `side`, `up` states, including chunk boundaries and negative coordinates.

**Done criteria:** Visibly divergent cases from vanilla are clearly identified; tests reproduce them prior to fixing.

## 2. General State System and Neighbor Updates

1. Construct typed access to registry block states: name and properties → exact state ID and vice versa. Data and generators must be versioned and reproducible; do not use ID arithmetic without confirmed lookup tables.
2. Separate raw block writing, state calculation upon placement, neighbor updates, scheduled/random ticks, and network packets. Define clear transaction boundaries.
3. The update queue must operate during placement, breaking, use, signal changes, chunk loading, and other mutations. Do not lose events, execute unbounded recursion, or hold a world lock across network I/O.
4. Ensure deterministic ordering for critical scenarios and bounded per-tick work; persist deferred updates and restore them after a restart.
5. Test placement across chunk boundaries, across dimensions, after unload/reload, with missing neighbor chunks, and under concurrent two-player interaction.

**Done criteria:** Block modifications reliably update dependent states and persist without infinite loops, silent drops, or unnecessary world loading.

## 3. Redstone Dust — First Functional Milestone

1. Establish connection rules with dust, sources, receivers, repeaters, and comparators, specifically horizontal, vertical, and diagonal boundary constraints.
2. Calculate connection geometry and exact state after placing or removing dust/neighboring blocks; support dot/cross shapes and player interaction in accordance with 26.3 behavior.
3. Implement `0..15` signal levels with correct attenuation, constant and variable power sources, rules for conducting through solid blocks, and lamp updates.
4. Multiple power sources: turning off one source must not depower a line powered by another; verify loops, long lines, branching, source toggling, and absence of phantom power via tests.
5. Correctly display and transmit all modified states via block updates and chunk packets; upon restart, layout and signal strength must be restored.

**Done criteria:** Dust and lamp function correctly across straight lines, turns, branches, neighbor updates, dual power sources, and restarts; behavior verified using a real client.

## 4. Signal Sources and Receivers

Sequentially implement and test redstone blocks, levers, buttons, pressure plates, torches, lamps, daylight sensors, observers, tripwires, and other relevant blocks. For each, determine placement rules, orientation, signal strength, scheduled ticks, interaction, and persistence. Document support by family and explicitly flag missing features.

## 5. Repeaters, Comparators, and Mechanisms

1. Repeaters: direction, signal, delay, locking, and reaction to network changes.
2. Comparators: direction, modes, side inputs, container readings, persistence, and correct order of scheduled ticks.
3. Pistons/sticky pistons: direction, push limit, movement of blocks and block entities, sticky mechanics, conflicts, and neighbor updates; prevent inventory loss.
4. Doors, trapdoors, fence gates, note blocks, dispensers/droppers, hoppers, rails, and other mechanisms — driven by dedicated scenarios, resource limits, and persistence.
5. TNT: power, ignition, fuse, explosion, damage, drops, networking, and restart; do not substitute simple block removal for a full vanilla explosion.

**Done criteria for each mechanism:** Behavior, state, client synchronization, load limits, persistence, and vanilla parity are verified.

## 6. Other Block Mechanics

In separate families, verify placement/orientation/shape for stairs, slabs, fences, walls, glass, waterlogging, doors, beds, and signs; support for plants and attachments; fluids and falling blocks; block entities, containers, furnaces, crafting, and breaking with loot. Unsupported actions must fail safely without damaging items or the world. A registry ID on its own does not constitute an implementation.

## 7. Persistence, Networking, and Load

- Persist states, block entities, and the tick queue in a well-defined format with atomic writes; test autosave, graceful shutdown, and restart.
- Two clients must see identical states within their render distance; verify chunk reload, teleports, multiple dimensions, proxy setups, and absence of stale updates.
- Bound update network depth/volume, task counts, and memory usage; benchmark TPS on long transmission lines and clock loops. Verify the effect of simulation-distance.
- Verify backward compatibility for older worlds and version packages, taking backups prior to migration. Do not claim unverified vanilla Anvil compatibility.

## 8. Coverage and Release Criteria

1. Maintain a family-by-family matrix: registry, placement, interaction, power/ticks, networking, persistence, vanilla fixtures, real client.
2. Tests: screenshot recreation, straight/corner/diagonal connections, multiple sources, 0..15 signal, lamp, repeaters, chunk boundaries, restart, load testing, two players, security, and malformed packets.
3. Run ``cargo test --workspace --locked`` after reviewing its side effects, followed by release build and isolated smoke test; test with a real 26.3 client where feasible.
4. Assemble synchronized executable/DLL/data in `dist`; do not overwrite user data, shut down the server cleanly, and leave it stopped.
5. Honestly enumerate implemented and unsupported mechanics; do not claim full vanilla parity based solely on registries or unit tests.

The implementer proceeds through the phases sequentially, not stopping after a single lamp or test, while remaining transparent about incomplete work and external constraints.
