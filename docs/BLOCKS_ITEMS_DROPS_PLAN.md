# Plan: All Blocks and Items in Minecraft 26.3 and Item Dropping

## Goal

Bring support for blocks, items, and in-world item entities to verified
Minecraft Java 26.3 (protocol 777) behavior: registries, rendering,
placement, breaking, correct drops, inventory, dropping, pickup,
simulation, and persistence. The presence of an ID in the registry does not in itself
imply that the block or item is implemented in the game.

First priority is preventing inventory loss on `Q` and `Ctrl+Q`, followed by
implementing the complete lifecycle of in-world item entities and progressively finishing
support for all block and item types. Do not claim vanilla parity
until actual behavior, client, and persistence have been verified.

## Current State to Verify Before Work

- `RadCore/data/mc777/PROVENANCE.json` describes tables with 1,286 block entries
  and 1,658 item entries. These are data from the current source, not proof of completeness
  or functionality of each entry. Reconcile them with vanilla 26.3 and
  the actually utilized version package.
- `RadVersion26_3/src/gameplay/inventory.rs` contains conversions
  `item_id_to_block_state`, `block_state_to_item_id`, and the method
  `drop_selected_hotbar`.
- The current `PlayerAction` branch in `RadVersion26_3/src/net/play.rs` on Q or
  Ctrl+Q calls `drop_selected_hotbar` and synchronizes the inventory; in this branch
  there is no in-world item creation. This poses a risk of item loss and is the first defect.
- Determine the current status of other work, applicable AGENTS.md, existing tests,
  processes, and other developers' changes; do not overwrite others' work.

## Principles of Execution

- Use test worlds and ports; do not modify working `world`,
  playerdata, `ops.json`, `whitelist.json`, configs, or secrets without a backup.
- First reproduce the loss risk and protect the player. Do not consider mere
  disappearance of an item from the hotbar a successful drop.
- Track coverage by states: ID known → item renders → action
  implemented → synchronized to client → persisted → verified against vanilla.
- Do not copy decompiled code or data without checking provenance and rights.
  Preserve provenance, versions, reproducible generators, and hashes.
- Break down implementation by behavioral families; do not add 1,286 manual
  branches and do not simulate support via default state or stubs.
- Each completed stage: substantive tests, regression analysis, `fmt`/clippy, and
  applicable workspace checks. Clearly state unavailable verifications.

## Stage 1. Immediately Eliminate Item Loss

1. Create a regression test for current Q and Ctrl+Q behavior: selected slot,
   count, item components, server/client synchronization, and item
   counts before and after the action.
2. As long as the world item entity cannot be reliably spawned, do not decrement inventory:
   safe failure with resynchronization. Test empty slot and full
   queue/spawn error; the operation must neither lose nor duplicate items.
3. Check analogous drop paths from open containers, outside windows,
   upon death, and during block breaking; identify and address other item loss vectors.
4. Add logging for failure reasons without secrets or personal data.

**Done:** the item does not disappear without creating a valid world entity, including
on error or disconnect during the action.

## Stage 2. Item Entity and Network Protocol

1. Reconcile `minecraft:item` entity type ID, SpawnEntity, metadata item stack,
   components, and corresponding movement/destruction packets with protocol 26.3.
   Independent wire fixtures and a real client must confirm rendering.
2. Implement `DroppedItem` as a server entity with entity ID, UUID, dimension,
   position, velocity, full item stack and component patch, count,
   pickup delay, age, and visibility state for players.
3. Q spawns a single unit, Ctrl+Q — the existing stack; preserve exact components,
   custom names, and metadata. Do not modify the stack before the world entity is ready;
   coordinate rollback and synchronization order on errors.
4. Transmit spawn/metadata and updates only to players in the relevant dimension
   and chunks; late-joining players must receive existing items. Correctly
   remove from visibility upon unload, dimension change, disconnect, and despawn.
5. Test two clients, chunk boundary, slow client, queue overflow,
   disconnect during a transaction, and unacknowledged/duplicate packets.

**Done:** the dropped item is visible to other players and precisely matches
the item stack taken from the slot.

## Stage 3. Physics, Pickup, and Persistence

1. Add gravity, block collisions, resting on surfaces, buoyancy/water,
   and interactions conforming to the verified 26.3 model. Limit
   entity counts and per-tick workload; do not load arbitrary chunks.
2. Pickup: post-drop delay, reach limit, available inventory space,
   correct partial pickup, absence of duplication, synchronization, and
   corresponding client effect. A full inventory leaves the remaining item.
3. Merging of compatible stacks taking into account ID, components, max stack size,
   dimension, and delays; incompatible items are not merged.
4. Reconcile age, despawn, and influence of active chunks/simulation-distance against vanilla.
   An item outside the active region must not be lost due to simple unloading.
5. Save items alongside chunks or via a documented persistence
   format; atomic snapshot and correct restoration following autosave/restart.
   If Anvil/vanilla compatibility is claimed, verify independently.
6. Integration scenario: two players, multiple stacks with components, full
   inventory, disconnect/reconnect, chunk transitions, clean restart, and pickup.

**Done:** after dropping, moving across chunks, and restarting, the
total count and components are preserved; pickup works for both clients.

## Stage 4. Complete and Verified Registry for 26.3

1. Construct an independent comparison table of all block IDs, block states, and
   item IDs based on an authorized and verified Minecraft 26.3 source.
   Verify missing, extra, duplicates, default state, valid properties,
   data components, stack size, and network ID alignment.
2. Anchor generation of Rust tables, version package, manifests, and client
   registries to a single authoritative source. Verify hashes, consistency, and
   reproducibility; do not rely on name matching for mapping state → item.
3. Generate a machine-readable coverage report for each block/item covering
   categories: registry, rendering, placement, breaking, loot, interaction,
   physics, crafting, persistence, client verified. No fabricated 100% metrics.
4. Ensure correct rendering of each item in creative inventory and
   via `/give`, including valid stack size and components. Check required permissions,
   config, and network format separately.

**Done:** all registry entries present in vanilla 26.3 are reconciled,
mapping errors feature automated diagnostics, and unsupported actions are visible.

## Stage 5. Placement and Block States

1. Check target location, reach, collision, gamemode, protection, selected item,
   consumption, and player state before mutating the world. Invalid actions must not consume the item.
2. Implement data-driven rules for solid blocks, transparent blocks,
   slabs, stairs, fences, walls, glass panes, waterlogging, doors,
   trapdoors, beds, plants, lanterns, rails, signs, and other 26.3 groups.
3. Context-dependent properties: facing, face, half, neighbors,
   support, water, gravity, light, and multi-block structures. Generated
   properties must match active protocol block-state IDs.
4. Update neighbors and clients, handle latency and duplicate packets;
   cancellation/error must not leave inconsistent inventory and world state.
5. Representative tests for each family and automated verification
   of coverage for all IDs; verification with two real clients and restart.

## Stage 6. Breaking, Tools, and Loot

1. Vanilla breaking times, tool efficiency, materials, enchantments,
   gamemode, unbreakable blocks, and other conditions confirmed for 26.3.
2. Loot tables with proper provenance/license, seeded RNG, context,
   tool, predicates, functions, and fallback behavior without silent item loss.
3. Block entities, container contents, and multi-block structures upon breaking;
   correct drops, XP, broadcasting changes, and persistence.
4. Verify prevention of duplicate drops, out-of-range packets, race conditions
   between two players, insufficient inventory space, and restarting mid-operation.

## Stage 7. Interactive Blocks and Items

1. Containers, furnaces, crafting table, signs, doors, buttons, levers, beds, rails,
   fluids, redstone, light, and scheduled ticks — distinct family components.
2. Tools, food, armor, weapons, projectiles, buckets, boats, maps, spawn eggs,
   potions, enchantments, and other groups — sequentially with concrete criteria.
3. Recipes and data packs: vanilla 26.3 validation, conflicts, cache, data
   assembly, and preservation of item components; network recipe synchronization.
4. For unsupported mechanics, cleanly reject or document limitations;
   do not consume an item or destroy a block entity without an implemented action.

## Stage 8. Reliability, Quality Assurance, and Readiness

1. Dedicated isolated tests: all registry entries, mapping, block state families,
   Q/Ctrl+Q, pickup/merge, creative/survival, two players, dimensions, and chunks.
2. Persistence and security: autosave/restart, crash and corrupted files,
   large/malformed packets, limits, atomicity, lock ordering, bounded queues.
3. Load testing: large numbers of item entities and active chunks; measure TPS,
   memory, queues, and restart duration without infinite loops.
4. ``cargo test --workspace --locked``, release, and version package; verify consistency of
   `.exe`, DLL, and data. Scripts may update `dist`: verify them prior to
   execution, stop server cleanly if required, do not force-kill on hang.
5. Real 26.3 client: Q, Ctrl+Q, pickup, inventory, placement/breaking of
   representative blocks, two players, and clean restart. Explicitly note
   features tested only in a GUI-less test client.
6. Update coverage report and documentation, honestly enumerate missing
   vanilla mechanics; leave server stopped for user launch.

## Ordering and Criteria for Full Completion

Do not skip item loss: Stage 1 → 2 → 3 before massive block
expansion. Execute Stages 4–7 iteratively by families, without waiting for artificial
completion of a single monolithic checklist. Full completion means that every
vanilla 26.3 entry has verified behavior, rendering, and persistence,
rather than merely existing in JSON. The scope of this work is substantial; its duration cannot
honestly be guaranteed. The executor continues available work without stopping after
a single sub-stage, and ultimately presents factual results and remaining gaps.
