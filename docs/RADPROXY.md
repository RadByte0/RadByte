# RadProxy — Documentation and API

**RadProxy** is a high-performance L7 proxy for Minecraft Java Edition `26.3` (protocol `777`), built in Rust on top of the [`RadCore`](../RadCore/src/lib.rs) library.

---

## 1. Project Structure

- `RadCore/` — shared protocol core: `VarInt`/`VarLong`, `Packet` trait, zero-copy framing, `AES-128-CFB8` encryption (`EncryptedStream`), Mojang `hasJoined`, Velocity Modern Forwarding HMAC-SHA256, `TextComponent`, Network NBT.
- `RadProxy/` — multi-threaded / `thread-per-core` (Linux `SO_REUSEPORT`) proxy server with anti-bot filter, load balancing, HAProxy support, and the **Radon** plugin/extension system.
- `rad.toml` — main configuration file (created automatically on first launch).
- `radons/` — directory containing radons (`*.wasm` and declarative `*.toml` manifests).
- `radonfigs/` — directory containing configurations for individual radons (`<name>.toml`).

---

## 2. Configuration (`rad.toml`)

When starting, `rad-proxy` automatically generates a commented `rad.toml`. Hot reloading is supported (`reload` in the console or `/radproxy reload` in-game) without dropping active sessions.

### Main Sections
- **Network and Authentication:** `bind_addr`, `online_mode`, `haproxy_protocol`, `haproxy_trusted_proxies`, `haproxy_allow_untrusted` (defaults to `false` — if `haproxy_trusted_proxies` is empty, untrusted connections are rejected), `connect_timeout_ms`.
- **Player Forwarding (`forwarding_mode`):**
  - `"none"` — no forwarding;
  - `"legacy"` — BungeeCord IP/UUID forwarding via `Handshake.server_address`;
  - `"bungeeguard"` — legacy + `bungeeguard-token` from `forwarding_secret`;
  - `"modern"` — Velocity Modern Forwarding (`HMAC-SHA256`).
- **Server List and MOTD:** `motd`, `motds` (rotation), `favicon_path`, `max_players`, `enforce_max_players`, `ping_passthrough` (`"none" | "players" | "description" | "all"`), `sample_players_in_ping`.
- **Security:** `handshake_timeout_ms`, `max_packet_size`, `max_connections_per_ip`, `rate_limit_per_sec`, `kick_existing_players`, `whitelist_enabled`, `whitelist`, `banned_players`, `banned_ips`.
- **Permissions and Logging:** `admins`, `log_player_connections`, `log_command_executions`.
- **Metrics:** `metrics_bind_addr = "127.0.0.1:9100"` (optional HTTP `/metrics` for Prometheus and `/health`).
- **Routing:** `backend_addr`, `default_server`, `try_servers`, `load_balance_strategy` (`"least_players" | "round_robin"`), `[servers]`, `[server_groups]`, `[forced_hosts]`, `[restricted_servers]`.

---

## 3. Commands

### In-Game (Chat)
- `/server` — list servers with current player counts (`*` next to the current server).
- `/server <name>` — switch to a server or group from `[server_groups]`.
- `/glist` — total player count and per-server breakdown.
- `/find <player>` *(admin)* — find which server a player is on.
- `/send <player|all> <server>` *(admin)* — transfer a player or all online players.
- `/alert <text>` *(admin)* — global broadcast announcement to all players.
- `/kick <player> [reason]` *(admin)* — kick a player from the proxy.
- `/radproxy info|reload` *(admin)* — view statistics or reload `rad.toml`, `radons/`, and `radonfigs/`.

### In Proxy Console
`help`, `status` (`glist`), `servers`, `radons`, `alert <msg>`, `send <player|all> <server>`, `kick <player> [reason]`, `reload`, `stop`.

---

## 4. How the API Works

### 4.1. Rust `ProxyRadon` API (`RadProxy/src/radon.rs`)
For built-in Rust radons, the `ProxyRadon` trait is used:

```rust
pub trait ProxyRadon: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn on_enable(&self) {}
    fn on_disable(&self) {}
    fn on_handshake(&self, ctx: RadonContext, handshake: &Handshake) -> RadonAction {
        RadonAction::Continue
    }
    fn on_login(
        &self,
        ctx: RadonContext,
        handshake: &Handshake,
        login: &LoginStart,
    ) -> RadonAction {
        RadonAction::Continue
    }
}
```

- `RadonContext` provides `peer_addr: SocketAddr` and `online_players: i32`.
- `RadonAction`:
  - `Continue` — allow connection to proceed;
  - `Deny(TextComponent)` — reject connection with a formatted message;
  - `RouteTo(SocketAddr)` — redirect connection to a different backend.

### 4.2. Cross-Platform WASM Radon API (`*.wasm` in `radons/`)
A single compiled `.wasm` file runs on any OS (`Windows`, `Linux`, `macOS`, `x86_64`, `ARM64`) inside the `wasmi` sandbox with a limit of `WASM_HOOK_FUEL = 100_000` instructions per hook invocation and `StoreLimits` (`16 MiB` linear memory, `10_000` table elements, `4` instances, `2` memories, `4` tables).

**Exported functions that the `.wasm` module must provide:**
- `radon_on_handshake(protocol_version: i32, server_port: i32, next_state: i32) -> i32`
- `radon_on_login(online_players: i32) -> i32`
  - Returning `1` means `RadonAction::Deny`; any other value means `Continue`.

**Host Imports (namespace `"radbyte"`):**
- `rad_get_host(ptr: i32, max_len: i32) -> i32` — writes the connection hostname into WASM memory, returns length.
- `rad_get_username(ptr: i32, max_len: i32) -> i32` — writes player username into WASM memory (in `radon_on_login`).
- `rad_get_ip(ptr: i32, max_len: i32) -> i32` — writes client IP address.
- `rad_get_config(ptr: i32, max_len: i32) -> i32` — writes raw TOML from `radonfigs/<name>.toml`.
- `rad_set_deny_reason(ptr: i32, len: i32)` — sets denial reason text when returning `1`.

### 4.3. Radon Configurations (`radonfigs/`)
- All `*.toml` files from `radonfigs/` are loaded at startup and during `reload` via `RadonManager::load_radonfigs`.
- Raw TOML content is accessible by filename via `RadonManager::config_for("<radon_name>")`.
- Declarative `RadonManifest` configs (with `id`, `target = "proxy"`, `route_host`, `route_to`) automatically register hostname-based routing.

### 4.4. `RadCore` API (`RadCore/src/lib.rs`)
- `Packet` trait (`ID`, `STATE`, `DIRECTION`, `decode_body`, `encode_body`).
- `encode_packet` / `encode_packet_with_compression` and `read_next_frame` for framing.
- `EncryptedStream` — wrapper around `TcpStream` supporting `AES-128-CFB8` without unnecessary allocations.

---

## 5. Generating HTML Documentation (Rustdoc)

To build local HTML documentation for all public structs, methods, and examples:

```bash
cargo doc --workspace --no-deps --open
```
