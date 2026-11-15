# Vanilla Configuration and Parameter Organization Plan for RadServer

## Goal and Current State

The goal is predictable vanilla settings with real behavior, separate
chunk simulation, and a clear parameter ordering. The benchmark for the list is
the local `Paper-26.3/server.properties`; exact Java 26.3 behavior must be
verified against a vanilla server and a real client. Paper and vanilla may
differ. Do not claim parity until it has been verified.

In the current code, `view-distance` controls chunk sending, and in Login Play
the simulation distance value is copied from it (`RadServer/src/net/play.rs`).
There is no separate `simulation-distance`. The loop in `RadServer/src/net/mod.rs`
invokes `step_world_tick` and `WorldSim::step`; prior to changes, the coverage
of each update type must be verified rather than assuming all loaded chunks are identical.
`docs/VANILLA_26_3_PROPERTIES.md` is outdated: for example, it still lists
`bind-addr` and `world-seed`. It must be updated based on verification results.

## 1. Structure and Order

Keep `rad.properties` alongside the executable for supported vanilla
settings, and `config/radserver-extra.properties` for RadByte settings.
Do not create fictitious keys. Inside the files, maintain a stable order,
blank lines, and comments with section names; the `key=value` syntax is preserved.

Order of sections in `rad.properties`:

1. **Network:** `server-ip`, `server-port`, `network-compression-threshold`, `rate-limit`.
2. **Server List:** `motd`, `enable-status`, `hide-online-players`, `max-players`.
3. **Access:** `online-mode`, `enforce-secure-profile`, `white-list`, `enforce-whitelist`, `op-permission-level`, `prevent-proxy-connections`.
4. **World and Generation:** `level-name`, `level-seed`, `level-type`, `generator-settings`, `generate-structures`, `initial-enabled-packs`, `initial-disabled-packs`.
5. **Distances and Simulation:** `view-distance`, `simulation-distance`, `entity-broadcast-range-percentage`, `max-world-size`, `max-chained-neighbor-updates`, `pause-when-empty-seconds`.
6. **Game Rules:** `gamemode`, `force-gamemode`, `difficulty`, `hardcore`, `allow-flight`, `spawn-protection`, `player-idle-timeout`, `function-permission-level`.
7. **Resource Pack:** `resource-pack`, `resource-pack-id`, `resource-pack-sha1`, `resource-pack-prompt`, `require-resource-pack`.
8. **Remote Management:** `enable-query`, `query.port`, `enable-rcon`, `rcon.port`, `rcon.password`.
9. **Monitoring and System Parameters:** `enable-jmx-monitoring`, `debug`, `log-ips`, `max-tick-time`, `sync-chunk-writes`, `region-file-compression`, `status-heartbeat-interval`, `use-native-transport`, `bug-report-link`.
10. **Other 26.3 Features:** `accepts-transfers`, `broadcast-console-to-ops`, `broadcast-rcon-to-ops`, `chat-spam-threshold-seconds`, `command-spam-threshold-seconds`, `enable-code-of-conduct`, `text-filtering-config`, `text-filtering-version`, `management-server-enabled`, `management-server-host`, `management-server-port`, `management-server-allowed-origins`, `management-server-secret`, `management-server-tls-enabled`, `management-server-tls-keystore`, `management-server-tls-keystore-password`.

This is a target catalog, **not a list of already implemented keys**. Only add a
key after implementation or an explicitly documented deviation from vanilla.
Preserve `pvp` for compatibility, separately verify its alignment with gamerule
in 26.3; do not rename without migration.

Order of `config/radserver-extra.properties`: version; forwarding; timeouts and
network limits; tick/autosave; generator and spawn; mobs; messages and
lobby; compatibility/administrators. Do not equate `admins` with secure
authorization; define a secure replacement via UUID/`ops.json` going forward.

## 2. Persistence and Migration

- The generator for new files must output sections in a single defined order;
  adding a field in Rust must not alter the ordering of all other fields.
- Do not overwrite existing configs during normal startup. Create a separate
  migration/formatting command with backups, previews, atomic writes, and clear
  conflict reporting.
- Support legacy `bind-addr`, `world-seed`, `game-mode`, the old
  `radserver-extra.properties` path, and TOML as the code already does. Reject
  duplicate aliases for the same field with file name and line number; do not
  pick silently.
- Preserve unknown comments and user values during migration; do not silently
  ignore unknown active keys. Never print secrets to logs.
- Do not modify addresses, permissions, UUIDs, world, seed, or generator
  automatically. Formatting twice must produce the exact same result; test write
  crashes.

## 3. Simulation Distance — First Functional Stage (IMPLEMENTED FOR RADSERVER)

1. An independent, validated parameter `simulation-distance` (`2..=32`) has been added to `# Distances` in `rad.properties` (template `4`; if absent in an older config, inherits `view-distance`).
2. `PlayLogin` sends the actual `simulation_distance` to the client independently of `view_distance`.
3. `SimulationArea` computes square (Chebyshev) zones per dimension for all players and `SimulationTicket` without loading chunks; with no players or tickets, the area is empty (matching the removal of permanent spawn chunks in 1.21.9+/26.3; portal/pearl tickets are not yet implemented).
4. Connected to `WorldSim::step_in_area` (with suspension in `suspended` without blocking active chunks, queue limiting, and persistence in `sim_queue.json`), `FurnaceStore::tick_in_area`, and `EntityManager::tick_in_area` (mobs + projectiles).
5. Automated unit/wire tests passed; direct comparison against a local vanilla 26.3 JAR and a live GUI client is `NOT_RUN` (vanilla JAR is absent in the environment).

## 4. Stages for Remaining Vanilla Parameters

**A. Audit of existing parameters.** For each current key, trace parser →
validation → runtime → client → persistence, verify edge cases, and
document actual discrepancies. Separately verify whitelist, op bypass,
spawn protection, difficulty, max player count, and status ping.

**B. Access and network.** Implement account verification via Mojang session
server for direct `online-mode`, encryption, valid UUIDs/skins, reconnects,
and failure handling; verify direct and proxy scenarios separately.
Next, secure profiles, compression, rate limiting, and load throttling.
Do not grant `online-mode=true` if the server does not actually verify accounts.

**C. World and persistence.** Validate `level-seed` (vanilla allows text),
rules for new/existing worlds, `level-type`, generator options, structures,
datapacks, world border, region writing, and persistence completeness. Do not
call a custom seeded generator vanilla prior to verification. Implement safe
migration and backups; test restarts and crash termination.

**D. Game rules.** Implement hardcore, idle timeout, allow-flight, function
permissions, and broadcast range alongside corresponding protocol messages,
states, and persistence. For gamerules, use the vanilla rule model of the
corresponding version; do not conflate them with properties simply because of a
similar name.

**E. Resource packs and remote access.** Implement resource packs and client
responses, then Query and RCON with passwords, network isolation, and rate
limiting; test rejections/timeouts/errors and blocking access without
authorization.

**F. Operations and other APIs.** For watchdog, logging, management server,
TLS, text filtering, telemetry/JMX, and platform-specific transport, first
determine feasibility in Rust and compliance with 26.3. Every key must be
fully implemented, explicitly marked as unsupported, or omitted.
`max-tick-time` must not mask data loss on crash exit.

## 5. Readiness Verification

- Automated tests of the real behavior of each added parameter: default,
  value change, invalid value, and restart.
- Protocol and client tests for 26.3: direct connection, via proxy,
  login, movement, dimension change, resource pack, and negative cases as needed.
- Migration tests for existing configs, world, and playerdata; secret safety
  and DoS protection; endurance tests for simulation and restarts.
- `scripts/verify.ps1`, release build, and launch verification from `dist` in
  a temporary directory. Do not alter the production/active world during automated tests.
- Update `README`, documentation, examples, compatibility matrix, and changelog;
  candidly designate missing features and unverified checks.
