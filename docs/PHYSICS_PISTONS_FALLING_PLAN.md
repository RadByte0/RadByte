# Plan: Physics, Falling Blocks, and Pistons for Minecraft 26.3

## Goal and Honest Status

Implement and verify player and entity movement, block physics, sand,
gravel, pistons, and redstone interaction for Java 26.3 / protocol 777.
Complete vanilla physics is a large scope of work; do not declare it complete
merely on the basis of a few tests or block-state IDs.
`docs/BLOCK_MECHANICS_REDSTONE_PLAN.md` describes the broader mechanism plan
and complements this document.

The current implementation of `WorldSim` processes gravity via scheduled block
changes rather than a full falling block entity. This differs from vanilla
animation and interaction; inspect the current code before starting work.

## 1. Reproduction and Baseline

- Read AGENTS.md, check active processes and external changes. In a temporary
  world, reproduce sand, red sand, gravel, concrete powder, anvil, regular and
  sticky piston, redstone block, and lever.
- Verify states, timings, collision shapes, and Java 26.3 behavior against
  reliable sources or a vanilla server. Do not treat ordinal ID assumptions as facts.
- Record the build, packet/data version, and current tests; separately note
  results of automated tests versus a real client.

## 2. Falling Blocks

1. Verify all relevant falling block types according to version data: sand,
   red sand, gravel, colored concrete powders, anvils, and other confirmed
   types. Preserve correct states and properties during movement.
2. Implement the `falling_block` entity: entity ID, state, position, velocity,
   gravity, collision, network spawn/move/remove, behavior upon unload and across
   different dimensions; account for adequate limits and simulation-distance.
3. Verify fluid/replaceable block substitution, falling stacks, interaction with
   torches, slabs, air, water, and world border; do not write two blocks upon
   a failed world update.
4. Concrete powder in contact with water hardens according to vanilla;
   anvils and other falling blocks have their own damage/wear rules.
5. Entity state persistence, scheduled ticks, restart, and safe behavior at chunk
   boundaries; zero loss or duplication of blocks/items.
6. Tests for all colors, high falls, two players, dimensions, negative
   coordinates, inactive chunks, water/lava, loading, restart, and client animation.

## 3. Pistons and Redstone

1. Determine exact placement/facing/extended states for regular and sticky
   pistons and input signal rules in 26.3. Do not equate registry ID presence with mechanics.
2. Design bounded block movement scheduling and collision handling: direction,
   push limit, obstructions, state, neighbors, block entities, ordering, and rollback.
3. Add piston head, moving blocks, client animation, extension, retraction,
   and sticky piston pulling; correctly persist components.
4. Ensure redstone updates and ticks on activation/deactivation, multiple
   sources, processing order, chunk boundaries, player/entity displacement;
   prevent container duplication and data loss.
5. Real tests: push single/multiple blocks, obstruction, push limit, sticky pull,
   redstone and lever/switch, concurrent events, piston across chunk boundary,
   restart mid-movement, two clients, and world protection.

## 4. Player Movement and Other Block Interactions

- Verify movement, adjacent jump, crouch, step height, stairs, slab,
  water/lava, falling, falling blocks, and collision shapes against Java 26.3.
- Correct movement across chunk/dimension boundaries, teleport confirmation,
  anticheat without false rejections of valid movement, flight states, and gamemode.
- Neighbor updates for support blocks, plants, dust, fluids, doors, and
  attached items; bounded deterministic queues and correct block updates.
- Persist all confirmed block state properties and component data on save
  and reload; do not overwrite the user's world with tests.

## 5. Testing and Completion

- Automated tests for actual states, wire messages, and restart; negative
  tests on entity counts, malformed packets, queues, races, and write interruptions.
- Comparative scenarios against vanilla and a real 26.3 client, not just headless;
  separately record anything that could not be verified.
- Check `scripts/verify.ps1` for side effects, run the entire test suite,
  produce a release build, and run isolated smoke testing with the version DLL and package data.
- Before updating a running server, save and shut down cleanly without force;
  preserve settings/worlds and leave server launch to the user.
- Update the readiness table: block/mechanism type, supported scenarios,
  persistence, client verification, remaining vanilla discrepancies.

The contributor works iteratively and does not stop after the first test. If the
scope cannot be finished in the current run, report what was actually accomplished
and clearly list remaining items without claiming a partial implementation as a
complete fix.
