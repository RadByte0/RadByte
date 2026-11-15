# RadByte Radon API — Developer Guide

**Radon** is the extension/plugin system for `RadProxy` (and future services within the RadByte ecosystem).
Three radon formats are supported:

1. **Declarative `*.toml` manifests** (in `radons/` or `radonfigs/`) — no programming required.
2. **Cross-platform `*.wasm` modules** (in `radons/`) — a single file works across Windows, Linux, and macOS inside the `wasmi` sandbox.
3. **Native Rust implementations** of the `ProxyRadon` trait.

---

## 1. Declarative `*.toml` Radon (No Code)

File `radons/skyblock-route.toml` (or in `radonfigs/`):

```toml
id = "skyblock-route"
version = "1.0.0"
api_version = 1
target = "proxy"
description = "Route sky.example.com directly to Skyblock"
route_host = "sky.example.com"
route_to = "127.0.0.1:25568"
```

---

## 2. Cross-Platform WASM Radon (`*.wasm`)

The proxy and `RadServer` execute each `.wasm` module within a `wasmi` sandbox with a fuel limit of `WASM_HOOK_FUEL = 100_000` instructions per hook invocation and `StoreLimits` (`16 MiB` linear memory, `10_000` table elements, `4` instances, `2` memories, `4` tables).

### Exports That `.wasm` Can Implement
- `radon_on_handshake(protocol_version: i32, server_port: i32, next_state: i32) -> i32`
  - `next_state`: `1` = `Status`, `2` = `Login`, `3` = `Transfer`.
  - Return `1` to reject the connection (`RadonAction::Deny`), or `0` to allow it.
- `radon_on_login(online_players: i32) -> i32`
  - Return `1` to reject the login (`RadonAction::Deny`), or `0` to allow it.

### Host Functions (namespace `"radbyte"`)
| Function | Availability | Signature | Description |
|---|---|---|---|
| `rad_get_host` | `RadProxy` | `(ptr: i32, max_len: i32) -> i32` | Copies hostname (`Handshake.server_address`) into WASM memory, returns bytes written (or `-1` on error). |
| `rad_get_username` | `RadProxy`, `RadServer` | `(ptr: i32, max_len: i32) -> i32` | Copies player username into WASM memory. |
| `rad_get_ip` | `RadProxy` | `(ptr: i32, max_len: i32) -> i32` | Copies client IP address into WASM memory. |
| `rad_get_config` | `RadProxy`, `RadServer` | `(ptr: i32, max_len: i32) -> i32` | Copies content of `radonfigs/<name>.toml` into WASM memory. |
| `rad_set_deny_reason` | `RadProxy` | `(ptr: i32, len: i32)` | Sets the denial reason text (up to 4096 UTF-8 bytes) shown to the player when returning `1`. |
| `rad_send_chat` | `RadServer` | `(ptr: i32, len: i32)` | Sends a chat message to the player who triggered the event. |
| `rad_broadcast_chat` | `RadServer` | `(ptr: i32, len: i32)` | Sends a chat message to all players on the server. |
| `rad_teleport_player` | `RadServer` | `(x: f64, y: f64, z: f64)` | Teleports player to coordinates `(x, y, z)`. |
| `rad_set_block` | `RadServer` | `(x: i32, y: i32, z: i32, state_id: i32)` | Sets block state at coordinates `(x, y, z)`. |

### Example WASM Radon in Rust (`wasm32-unknown-unknown` via `rad-api`)

```toml
[dependencies]
rad-api = { path = "../RadAPI", default-features = false }
```

```rust
use rad_api::v1::wasm;

#[unsafe(no_mangle)]
pub extern "C" fn radon_on_login(_online_players: i32) -> i32 {
    if wasm::username() == "BadBot" {
        wasm::set_deny_reason("Blocked by WASM Radon");
        return wasm::ACTION_DENY;
    }
    wasm::ACTION_CONTINUE
}
```

Building into a single cross-platform file:
```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```
Copy `.wasm` into `radons/`, and its configuration (if needed) into `radonfigs/<name>.toml`.

---

## 3. The `RadAPI` Crate and Selective Loading (`v1`, `v2`, ...)

Shared interfaces are maintained in a dedicated library crate [`RadAPI`](../RadAPI/src/lib.rs) (`rad-api`) with Cargo features for each version:
- `rad-api = { path = "../RadAPI", default-features = false, features = ["v1"] }` — loads **only `v1`** (ideal for lightweight `.wasm` radons).
- `rad-api = { path = "../RadAPI", default-features = false, features = ["v2"] }` — loads **only `v2`** (reserved for future API revisions).
- `rad_api::v1` (`API_VERSION = 1`) — `ProxyRadon`, `RadonContext`, `RadonAction`, `RadonManifest`, `RadonTarget`, and `rad_api::v1::wasm`.
- `rad_api::latest` — alias to the currently active/enabled API version.
