# General Neighbor Block Updates Plan in RadByte

## Goal and Current State

Implement the block update system for Minecraft Java 26.3, protocol 777:
a block change triggers corresponding neighbor checks, physics, scheduled ticks,
state updates, and client synchronization. The system must account for distinct
rules per block, persist data safely, and avoid freezing on loops/cycles.

In `RadVersion26_3/src/world/sim.rs`, handling of six direct neighbors, a
scheduler, gravity, redstone, and support checks for dust and certain floor
blocks already exists. This is a partial implementation: wall attachments,
complex support shapes, plants, mechanisms, and other cases cannot be considered
fully vanilla. Prior to working, inspect current code and tests.

## 1. Audit and Independent Verification

1. Read AGENTS.md, check third-party edits, running processes, existing tests, and
   the packaged DLL version. Test only in an isolated world and directory.
2. Trace all block change code paths: placement, breaking, commands, pistons,
   explosions, fluids, falling, ticks, generation, and API. Identify update bypasses.
3. Verify `WorldSim` queues, drops, their limits, persistence, chunk boundaries,
   simulation-distance, and lock ordering.
4. For each block family, compare 26.3 behavior against available independent
   sources or a real vanilla server. Block data and state IDs do not prove
   behavioral implementation. Document fixture provenance and licensing.

## 2. Unified Change and Update System

1. Create a unified path for applying block changes and queuing updates:
   cause, dimension, coordinates, old and new state, revision, and action source.
2. Define update rules for the six neighbors, distant dependencies, and
   scheduled ticks for specific blocks. Do not apply a single collision check
   to all blocks as a replacement for vanilla rules.
3. Create a bounded iterative queue with deduplication, correct cycle and order
   handling, per-tick work budgets, and starvation protection.
4. Do not drop events silently when reaching limits; provide safe continuation,
   diagnostics, and memory/load monitoring.
5. Do not hold the world lock while waiting on network/disk; synchronize all
   changed blocks to relevant players and return correct acknowledgments.

## 3. Support Properties and Shapes

1. Use a verified table of 26.3 block state properties: facing, face, half,
   shape, waterlogged, powered, and neighbor dependencies.
   Do not calculate IDs arithmetically without independently confirmed mapping.
2. Introduce support rules based on specific shapes/faces and special
   exceptions. Distinguish full block, top/bottom slab, stairs, and other shapes.
3. Floor and wall torches, redstone torches, buttons, levers, pressure plates,
   rails, paintings/other hanging entities — each must check the correct attachment
   face rather than assuming the block underneath.
4. Plants, mushrooms, cactus, sugar cane, vines, aquatic plants, and multi-block
   plants receive soil, water, height, light, and neighbor update rules
   according to verified behavior.
5. Doors, beds, fence gates, fences, walls, glass, and other multi-part
   or connectable blocks update both parts/neighbors without duplication or data loss.

## 4. Redstone and Mechanisms

1. Verify dust and its support, `north/east/south/west`, vertical transitions,
   levels `0..15`, multiple sources, and recalculation upon break/place.
2. Sources and consumers: torches, buttons, levers, pressure plates, redstone block, lamps,
   sensors, repeaters, comparators; direction, delays, loops, and restart.
3. Pistons and sticky pistons: power, states, block movement and retraction,
   obstructions, limits, animation, chunk boundaries, block entities, and safe
   rollback. Do not duplicate items or containers.
4. Hopper, dropper, dispenser, rails, doors, trapdoors, TNT, and other mechanisms
   should be added by family with verified interactions and network packets.

## 5. Gravity, Fluids, and Events

1. Sand, gravel, concrete powder, anvils: support, falling entity, water, landing,
   drops, damage, and behavior at world/chunk borders.
2. Fluids, fire, explosions, snow, ice, and scheduled/random ticks should be
   implemented according to separate rules; changes to any of them must notify neighbors.
3. Verify concurrent player actions, redstone, and physics without duplication,
   loss of blocks, items, or container contents.

## 6. Persistence and Networking

1. Queues of pending events, falling entities, block entities, and drops must be
   persisted and restored with clear semantics; do not lose them on restart.
2. Atomic saving, revision checks, and disk error handling; safe migration of
   existing worlds with backups. Do not claim vanilla Anvil parity without
   independent verification.
3. Synchronize blocks and entities to both clients, including on chunk reloads,
   teleports, dimension changes, and through a proxy.

## 7. Automated Tests and Acceptance

- For each family: placement, support, support removal, drops, adjacent
  blocks, invalid face, replaceable and partial shapes.
- Cascading updates, chunk boundaries and negative coordinates, multiple
  dimensions, two players, missing/inactive chunks, restart, snapshot, and block entities.
- Redstone power 0–15, multiple sources, loops, sensors, and pistons; high
  loads, filled queue, memory limits, TPS, and deadlock prevention.
- Comparative tests with vanilla 26.3 and a real client when available;
  headless/unit tests must be noted separately and not passed off as GUI verification.
- `scripts/verify.ps1`, release DLL, isolated packaging, and state verification
  before and after restart. Note that the script may update dist and stop the
  server; preserve data, avoid forceful termination, leave server startup to the user.

Format the final result as a table by block family: supported / partial /
unsupported, existing tests, and differences from vanilla. Complete vanilla
behavior is not considered finished without verification of each required case.
