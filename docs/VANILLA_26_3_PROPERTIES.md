# Minecraft Java 26.3 `server.properties` → RadByte Mapping

Comparison of standard `server.properties` parameters for **Minecraft Java Edition 26.3** (protocol `777`) with the `RadByte` configuration (`rad.properties`, `config/radserver-extra.properties`, and `radproxy.toml`).

> **Environment note:** The local vanilla Minecraft 26.3 server JAR is not installed (only `java.exe` is present), so the list and default values have been cross-checked against the `server.properties` specification for Minecraft Java 26.3 (including the move of `pvp`/`allow-nether`/`spawn-monsters` to gamerules since 1.21.9+ and `white-list = true` by default in 26.3).

| Vanilla Parameter (`server.properties`) | Default (26.3) | RadByte Parameter (`rad.properties` / other) | Implementation Status | Notes / Behavior |
| :--- | :--- | :--- | :--- | :--- |
| `server-ip` + `server-port` | `""` / `25565` | `server-ip` + `server-port` (`127.0.0.1` / `25567`) | **Working and verified** | In `rad.properties` they are configured separately (the public port `25565` is typically listened on by `RadProxy`). |
| `motd` | `"A Minecraft Server"` | `motd` (`"RadServer 26.3 - Hub"`) | **Working and verified** | Passed in `StatusResponse` (`handle_status`). |
| `max-players` | `20` | `max-players` (`200`) | **Working and verified** | Displayed in `StatusResponse` and blocks entry of non-op players when full (`"The server is full!"`). |
| `view-distance` | `10` | `view-distance` (`4`, `2..=16`) | **Working and verified** | Passed in `PlayLogin` and `SetChunkCacheRadius`; limits chunk streaming radius. |
| `simulation-distance` | `10` | `simulation-distance` (`4` in template, `2..=32`; inherits `view-distance` if omitted) | **Working and verified** | Passed in `PlayLogin`; limits active per-dimension simulation of furnaces, mobs/AI, projectiles, and scheduled block ticks (preserving pending tasks in `sim_queue.json`). |
| `gamemode` | `survival` (`0`) | `gamemode` (`adventure`: `survival`, `creative`, `adventure`, `spectator`) | **Working and verified** | Player's initial game mode. |
| `force-gamemode` | `false` | `force-gamemode` (`false`) | **Working and verified** | Forcibly applies `gamemode` on each login. |
| `difficulty` | `easy` (`1`) | `difficulty` (`easy` / `0..=3`) | **Working and verified** | Scales mob damage, starvation thresholds (`10`/`1`/`0` HP), and peaceful regeneration (`peaceful`). |
| `pvp` *(gamerule in 26.3)* | `true` | `pvp` (`true`) | **Working and verified** | Enables or blocks PvP attacks in `Attack::ID`. |
| `white-list` | `true` | `white-list` (`false` in template) | **Working and verified** | Controls validation during Login prior to entering the world; when absent in an older config, takes state from `access_control.json`. |
| `enforce-whitelist` | `false` | `enforce-whitelist` (`false`) | **Working and verified** | Kicks online players without whitelist/op when `/whitelist on` or `/whitelist reload` is executed. |
| `op-permission-level` | `4` | `op-permission-level` (`4`, `1..=4`) | **Working and verified** | Default permission level for `/op`; levels `1..=4` filter commands and `DeclareCommands`. |
| `spawn-protection` | `16` | `spawn-protection` (`16`, `0` disables) | **Working and verified** | Protects the radius around the Overworld spawn from block breaking/placing by players of permission level `< 1`. |
| `level-name` | `"world"` | `level-name` (`"world"`) | **Working and verified** | Relative name of the world directory (`<level-name>/region`, `playerdata`, `access_control.json`, `sim_queue.json`) and `LevelName` in `level.dat`. |
| `level-seed` | `""` | `level-seed` (`0`) + `world-type` | **Working and verified** | Seed for `world-type=seeded` and `level.dat`. |
| `enable-status` | `true` | `enable-status` (`true`) | **Working and verified** | If `false`, `handle_status` ignores Server List Ping. |
| `hide-online-players` | `false` | `hide-online-players` (`false`) | **Working and verified** | When `true`, returns `online: 0` in `StatusResponse`. |
| `online-mode` | `true` | `radproxy.toml` (`online_mode`) / `forwarding_mode` | **Requires a different model** | Mojang authentication operates on `RadProxy`; `RadServer` trusts signed `modern`/`bungeeguard` forwarding. |
| `network-compression-threshold` | `256` | `radproxy.toml` (`compression_threshold`) | **Partially implemented** | Supported at the proxy layer; `RadServer` accepts uncompressed local connections or connections forwarded from the proxy. |
| `autosave` | internal (6000 ticks) | `autosave-interval-secs` (`300`) | **Working and verified** | Periodic atomic saving of chunks, `playerdata`, `containers`, and `access_control`. |
| `hardcore` | `false` | `hardcore` (`false`) | **Working and verified** | Forcibly sets `difficulty=hard` (`3`), passes `is_hardcore=true` in `PlayLogin`, and puts a dead player into `Spectator` (`3`) upon `Respawn`. |
| `player-idle-timeout` | `0` | `player-idle-timeout` (`0`, `0..=1440` min) | **Working and verified** | Tracks `last_activity` on active packets and kicks idle players via `PlayDisconnect` (`"You have been idle for too long!"`). |
| `max-world-size` | `29999984` | `max-world-size` (`29999984`, `1..=29999984`) | **Working and verified** | Restricts player X/Z movement (with rollback via `SynchronizePlayerPosition`) and block interaction/editing within `[-max-world-size, +max-world-size]`. |
| `allow-flight`, `enable-rcon`, `enable-query`, `resource-pack*`, `rate-limit`, `enforce-secure-profile` | — | — | **Not yet supported** | Not added as placeholder fields without real server logic. |
