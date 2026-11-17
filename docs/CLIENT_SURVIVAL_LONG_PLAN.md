# Long-Term Plan: Stable Login and Test Survival for RadByte

## Goal

Bring RadServer 26.3 (protocol 777) to a state where a real client connects
directly and via RadProxy, does not encounter protocol errors, sees the world,
and two players can safely build, use inventory and furnaces, and restore state
after a restart. After that, systematically expand capabilities toward vanilla
parity. Full parity is a large separate scope, not a guarantee of completion
within a single run or a specific number of hours.

Statuses for each outcome: **unverified**, **code-confirmed**,
**automated test passed**, **real client passed**, **reconciled with vanilla**.
Headless test results do not replace verification with a real client.

## Current Observations

- In `RadCore/src/packet/configuration.rs`, there are tag entries for
  `minecraft:infiniburn_overworld`, `minecraft:infiniburn_nether`,
  `minecraft:infiniburn_end`, and 12 `minecraft:sulfur_cube_archetype/*`; there is a test
  `configuration_tags_reference_the_negotiated_registry_ids`. The presence of tags
  in the code does not yet prove semantic correctness of their members or successful client login.
- `RadVersion26_3` has parameters for view/simulation distance and batched
  chunk sending. Verify the current implementation and actual `dist`, as
  code and files may have changed since previous runs.
- In `versions/26.3/`, a native version library is used. Confirm
  consistency across the binary, DLL, and package data in the current build.
- The existing plans `RADBYTE_MASTER_EXECUTION_PLAN.md`,
  `VANILLA_CONFIG_COMPLETION_PLAN.md`, and `VERSION_PACKAGES_PLAN.md` supplement
  this document; in case of conflicts, verify the current code and behavior.

## Working Rules

1. Before making changes, read AGENTS.md, check checkout, running processes,
   ports, third-party changes, and test results. Do not overwrite others' work.
2. Use dedicated test directories, ports, and worlds. Do not edit working
   `world`, `ops.json`, `whitelist.json`, secrets, or configs
   unless necessary; make a backup before migrating.
3. Reproduce the problem before fixing it as much as possible. Record
   error text, client version, protocol phase, and the test that detects regression.
4. Avoid fictitious parameters and tests that merely echo the implementation.
   Verify actual behavior and negative scenarios.
5. Do not output secrets to logs and artifacts. Do not assume Mojang authentication
   for offline/direct mode. Describe formats and security models honestly.
6. After substantial changes, run targeted tests and ``cargo test --workspace --locked``;
   before updating `dist`, check whether the script triggers packaging or
   stops the current server. Do not forcefully terminate the server if it hangs.
7. The executor continues independent tasks across stages; do not stop
   merely because an intermediate step is complete. If a real client or
   external service is needed and unavailable, preserve the results and clearly
   note the limitation without declaring verification passed.

## Stage 0. Baseline State and Reproduction

1. Locate the current `latest.log` and client disconnect report, determine
   the full cause of the error and timestamp; do not confuse it with earlier errors from a different
   run. Check server state and related warnings.
2. Reconcile current code, timestamp/hash of `.exe`, version DLL, manifest, and data in
   `dist/RadServer/versions/26.3`; verify that the running process uses
   these exact files. Do not output secret contents.
3. Run baseline `fmt`, `clippy -D warnings`, tests, and documentation.
   Distinguish prior failures from new ones; record the exact result.
4. Create an isolated smoke server with its own config, port, and world.
   Attempt to join with a 26.3 client and gather details about the failure phase.
5. Verify that corrected tags and their actual values match
   the 26.3 data source. Do not conclude full protocol parity
   merely due to the absence of a specific message in the logs.

**Done:** known which build and which scenario were verified; the error is either
absent in the real client or has a concrete, reproducible description.

## Stage 1. Registry Loading and Configuration

1. Reconcile the Configuration flow: brand, feature flags, known packs,
   registry data, update tags, and finish acknowledgment with protocol 26.3.
   For each packet, verify ID, ordering, types, bounds, and connection state.
2. Verify all dynamic and static registries, tag names and references,
   IDs/indices, required elements expected by the client, and known
   packs compatibility. Reconcile tag composition and semantics against vanilla data, rather than merely
   adding empty tags when the client reports missing ones.
3. Create independent fixtures/data with verified provenance; automatically
   verify missing registry/tag keys, invalid references, and ranges.
   Handle dynamic registries without data according to the protocol.
4. Scenarios with cached/uncached known packs and missing packs;
   client logs must not contain registry loading errors.
5. Test negative packets, timeouts, size limits, and memory constraints.

**Done:** real client passes Configuration without registry errors;
automated tests detect missing and invalid required tags.

## Stage 2. Login, Play Login, and Disconnect Errors

1. Verify handshake, status, login success, authentication mode,
   UUID/properties/session ID, encryption/compression, and login acknowledged.
2. Compare the structure and IDs of initial Play packets: login, abilities, game
   event, operator permissions, commands, spawn, health, time, teleport,
   player info, chunk cache, and world. Verify via client and wire fixtures.
3. Add disconnect diagnostics categorized by cause: client EOF, timeout, overflow,
   write/decode error, shutdown; do not log sensitive data.
4. Verify teleport ordering and confirmation; stale packets must not
   cause loops. Verify protection against invalid values.
5. Verify direct connection and trusted proxy separately. Document `online-mode` state,
   UUID source, profile signing, and trust boundaries.

**Done:** the client not only displays the world screen, but also maintains the session, can
move, and can reconnect; diagnostics explain failures.

## Stage 3. Chunks, View Distance, and Network Backpressure

1. Verify `view-distance` 2, 4, 8, 12, 16, and 32 at allowed boundaries;
   chunk center, nearest-first ordering, unloading, crossing borders and dimensions.
2. Verify current streaming transmission and byte budget: a slow
   client does not create an unbounded queue, while a normal client is not disconnected due to
   a brief burst. Verify cancellation, receiver closure, writer
   failure, and the absence of holding a world lock while awaiting network I/O.
3. Verify batch framing/acknowledgment, chunk data, heightmaps, biomes,
   block states, light, and client loading against vanilla expectations.
4. Test fast movement, teleportation, multiple players, slow reading, and
   disconnect during transmission. Check memory/TPS at large radii.
5. Verify `simulation-distance` separately: chunk state, ticks, and packets;
   do not infer functionality solely from sending a number in Play Login.

**Done:** client with user configuration `12/8` connects and moves
without disconnection; performance and load boundaries are documented.

## Stage 4. World, Physics, and Building

1. Verify initial spawn, generation, negative coordinates, extreme Y values,
   chunk borders, interaction with saved worlds, and block state consistency.
2. Movement, collisions, point-blank jumping, stairs, slabs, falling, water, flight
   in permitted modes. Speed verification must not reject valid movement.
3. Breaking and placement: reach, direction, boundaries, spawn protection, gamemode,
   item consumption, drops, and packet acknowledgment. Two clients see
   the identical state; actions during latency are not duplicated.
4. Reconcile basic block collision shapes, orientation, and essential blocks with
   the protocol and vanilla behavior. Unknown blocks must not corrupt the world.
5. Test after restart: multiple modifications across different chunks and dimensions are preserved.

**Done:** user moves stably, builds, and sees the same
blocks after disconnecting and restarting the server.

## Stage 5. Inventory, Containers, and Persistence

1. Verify inventory, hotbar, cursor, offhand, armor, split/merge/drop,
   creative/survival clicks, acknowledgments, and desync resolution.
2. Crafting, chest, and furnace: recipes, consumption, fuel, progress, two players,
   closing/disconnecting, overflow, and invalid clicks without item duplication.
3. Verify `level.dat`, region `.mca`, playerdata, block entities, and
   versioning; determine what is vanilla-compatible and what is an internal format.
4. Preservation of position, UUID, gamemode, health, hunger, inventory, item
   states, containers, and scheduled ticks. Test clean restart and restore
   from backup, including interrupted writes and corrupted files.
5. Autosave, shutdown, and disk errors: atomic writes, consistent snapshots,
   revisions, and clear error reporting; do not lose changes during writes.

**Done:** two players craft items, place them in a container,
smelt in a furnace, and retrieve the saved state after a restart.

## Stage 6. Basic Survival and Administration

1. Health, hunger, fall damage, death, respawn, spawn point, and playerdata.
2. Basic mobs and projectiles: spawn, movement, combat, ticks inside/outside active
   regions, dimensions, and persistence according to the actually supported set.
3. Commands and client command tree: op/deop, gamemode, teleport, give, whitelist,
   help, save, and stop; permissions, online/offline UUID, and permission updates.
4. Paper/vanilla configs: supported keys are sorted, actually functional,
   validated, and have verified values; do not mock missing keys.
5. `ops.json`/`whitelist.json`: persistence, reload, conflicts, and errors,
   secure trust model for usernames and UUIDs.

**Done:** the described survival feature set can be tested with friends in
a controlled environment; security boundaries are clearly defined.

## Stage 7. Proxy, Fallback, and Version Package

1. RadProxy → RadLimbo/RadServer: status, login, encryption/compression,
   modern forwarding and invalid secret, IP/UUID/profile, and timeouts.
2. Server switching and fallback with a real client: registries, dimension,
   chunks, entities, commands, player state, and recovery after backend failure.
3. Native version DLL: ABI verification, schema/protocol/game version, hashes,
   safe paths, missing/incompatible files, lifecycle, and diagnostics.
4. Dedicated tests for clean install and upgrade in `dist`; do not confuse
   client protocol forwarding with supporting mechanics of other Minecraft versions.

**Done:** direct and proxy scenarios are verified separately; version package, DLL,
and `.exe` are consistent; failure does not obscure the root cause or corrupt data.

## Stage 8. Performance, Security, and Cross-Platform Support

1. Packets and NBT: limits on lengths, element counts, decompression, configs,
   timeouts, and fuzz/property tests for genuinely high-risk parsers.
2. Queues, RwLock, tick loop, and disk I/O: lock ordering, backpressure,
   cache size, bounded tasks, stress testing, absence of deadlocks and memory leaks.
3. Measure baseline TPS, latency, memory, login time, and restart time across multiple
   distances and player counts; establish targets based on acquired metrics.
4. Builds and execution on Windows, Linux, and macOS — only actually verified targets;
   do not assume native library ABI and access permissions are automatically portable.

## Stage 9. Vanilla Content After Stable Testing

Systematically maintain a checklist of blocks, states, items/components, recipes,
entities and AI, dimensions, generation and structures, redstone, enchantments, potions,
advancements, loot, gamerules, commands, and datapacks. For each: version data,
actual mechanics, client rendering, persistence, automated test, and reconciliation
with vanilla. Do not treat a registry entry as a completed gameplay feature.

## Release Criteria for Testing

- ``cargo test --workspace --locked`` and release build pass; isolated direct and
  proxy smoke tests are executed, or unavailable checks are explicitly stated.
- Real Minecraft 26.3 connects without protocol error, remains connected,
  moves, sees chunks at `view-distance=12`, with working permissions/commands.
- Two players build, use inventory, craft, and smelt; restart
  preserves the world and player state; backup/restore is verified.
- `dist` contains consistent executable, version library, and data package;
  existing user settings and data are preserved.
- The report includes actual tests, known defects, authorization constraints,
  platforms, and reproduction steps; server is left stopped for user launch.
