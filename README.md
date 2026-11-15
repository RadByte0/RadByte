# RadByte — High-Performance Minecraft 26.3 (Protocol `777`) Ecosystem in Rust

**RadByte** is a fast, modular server ecosystem for Minecraft Java Edition **26.3** (protocol **`777`**), written in Rust with a focus on low memory consumption (`mimalloc`), a clean protocol layer, and WASM extensions (`wasmi`).

**NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**

Current versioned data and certain tables have provenance that still requires
verification prior to public distribution. This is not a complete clean-room implementation:
see the [provenance audit and independent development plan](docs/PROVENANCE_CLEAN_ROOM.md).
Publication is currently blocked pending verification of code and data rights; `.gitignore`
excludes known data of unconfirmed provenance, so a copy cloned solely
from Git may not support a normal build or operation with a real client.

Configuration templates are stored in `config/templates/`. For local startup, use `./scripts/Start-RadByte.ps1`: working configs, worlds, access lists, and plugins will be stored in `dist/RadServer/`. Packaging creates separate working directories in `dist/`.

## Repository Structure

| Crate | Purpose | Config |
| :--- | :--- | :--- |
| [`RadCore`](RadCore) | Shared core: `VarInt`/`VarLong`, Network NBT, codec, `AES-128-CFB8`, `Velocity Modern Forwarding`, MC 26.3 packets, CLI utilities | — |
| [`RadAPI`](RadAPI) | Stable SDK (`v1`) for native and `.wasm` plugins (Radons) for `RadProxy` and `RadServer` | — |
| [`RadProxy`](RadProxy) | High-performance L7 proxy, `IpFilter`, `bungeecord:main`, `/metrics`, `DeclareCommands`, WASM radons | [`radproxy.toml`](config/templates/radproxy.toml) |
| [`RadLimbo`](RadLimbo) | Ultra-lightweight limbo/fallback server (`Login → Configuration → Play`) | [`radlimbo.toml`](config/templates/radlimbo.toml) |
| [`RadServer`](RadServer) | Version module host-launcher: reads `versions/version.toml`, verifies manifest/ABI/SHA-256, and loads the version library | [`rad.properties`](config/templates/rad.properties) |
| [`RadVersions/common`](RadVersions/common) | `rad-version` crate: version descriptors (`MC_26_3`), world compatibility checks (`WorldMechanicsProfile`, `level.dat`), safe backup, shared `shapes`/`physics` | — |
| [`RadVersions/v26_3`](RadVersions/v26_3) | Standalone Minecraft 26.3 implementation (`protocol 777`, C ABI v1 `rad_version_26_3`): 20 TPS, chunks/Anvil `.mca`, inventory, simulation/redstone, survival, mob AI, `RadAPI v1` WASM | — |

Additionally:
- [`docs/`](docs) — [documentation index](docs/DOCS_INDEX.md), architecture, configuration, domain plans, and provenance audit materials.
- [`scripts/`](scripts) — scripts for verification (`verify.ps1`) and isolated packaging into `dist/` (`package.ps1`).

## Standalone Service Startup and Testing

To avoid mixing configs, `world/`, and binaries in a single `target/release/` directory, build and stage each service into its own isolated `dist/<Service>/` folder:

```powershell
.\scripts\Build.ps1 -DataPackage <path-to-compatible-data-package>
```

Without `-DataPackage`, binary files (`rad-server`, `rad-proxy`, `rad-limbo`) are built and staged without the `versions/26.3/` data package (running `RadServer` requires an explicit compatible data package).

Afterwards, each component can be run and tested completely independently:

```powershell
cd dist\RadServer
.\rad-server.exe
```

On Linux/macOS, the executable is named `./rad-server` (`cargo build --workspace --release` places binaries in `target/release/`, and `versions/26.3/` and configs must be copied alongside; building and running on Linux/macOS have not been verified in the current Windows environment). A separate `libraries/` directory is not created, as Rust dependencies are linked statically into a single executable, while addons are located in `radons/` and `radonfigs/`.

Structure of `dist/RadServer/`:
- `rad-server.exe` (`rad-server` on Linux/macOS), `rad.properties`, `config/radserver-extra.properties`
- `versions/26.3/version.json` and `versions/26.3/data/{items,blocks,entities,menus}.json` (validated at startup before opening the port; can be regenerated via `.\scripts\generate_version_package.ps1`)
- `versions/version.toml` — Minecraft version selection, for example `version = "26.3"`. The current build only supports 26.3; setting or choosing another number does not add support for it on its own. Quotes are required so the number is not parsed as a TOML number.
- `radons/`, `radonfigs/`, `world/`

Similarly for `dist\RadProxy\` and `dist\RadLimbo\`.

## Whitelist, Operators (`op`), and `rad.properties` Configuration (MC 26.3)

The game mode is specified with words: `gamemode=survival`, `creative`, `adventure`, or `spectator`. The legacy `game-mode` name and numeric values remain available for backwards compatibility. For an already saved player, the setting is enforced when `force-gamemode=true`. RadByte-specific settings are located in `config/radserver-extra.properties`.

A complete mapping table from `server.properties` (Minecraft Java 26.3) parameters to `rad.properties` is provided in [`docs/VANILLA_26_3_PROPERTIES.md`](docs/VANILLA_26_3_PROPERTIES.md).

### How to Enable the Whitelist, Add Yourself, and Grant the First `op`

1. In [`rad.properties`](config/templates/rad.properties), enable the whitelist (by default in the test profile `white-list=false`, whereas in vanilla 26.3 `white-list = true`):
   ```toml
   white-list=true
   enforce-whitelist=false
   op-permission-level=4
   ```
2. Start `RadServer` (or the network via `dist\scripts\Start-RadByte.ps1 -Profile Network`).
3. In the `RadServer` console (permission level `4`), run:
   ```text
   whitelist add <YourNicknameOrUUID>
   op <YourNicknameOrUUID>
   ops
   save-all
   ```
   Or enable/reload the list without restarting:
   ```text
   whitelist on
   whitelist reload
   whitelist list
   ```
4. With a player holding level `3+` `op`, in-game commands `/op <player> [1..4]`, `/deop <player>`, `/ops`, `/whitelist <on|off|list|reload|add|remove>`, `/kick`, `/ban`, and `/pardon` are available; at level `4`, `/save`, `/reload`, and `/stop` are available. The client automatically receives a filtered command tree (`DeclareCommands`) matching its permission level.

`ops.json` and `whitelist.json` are stored in the RadServer working directory alongside `rad.properties` as JSON arrays. Operator entries contain `uuid`, `name`, `level`, and `bypassesPlayerLimit`; whitelist entries contain `uuid` and `name`. The `bypassesPlayerLimit` value is preserved on import, but does not yet control bypassing the player limit. For names added without a known UUID, the entry may temporarily lack a `uuid` and is not fully compatible with Paper. On first save, existing entries from `world/access_control.json` or legacy `world/ops.json` and `world/whitelist.json` are migrated without deleting the source files. Existing files alongside the config take precedence. Edit lists after stopping the server; `whitelist reload` re-reads the whitelist without overwriting operators.

## Quality Assurance and Testing

For a release build of all services and automatic updates to `dist/`, stop the services and run `./scripts/Build.ps1`. Server startup: `./scripts/Start-RadByte.ps1`; entire network: `./scripts/Start-RadByte.ps1 -Profile Network`. `cargo build` on its own does not package `dist`. The standard `cargo build` command writes results to `target`, following default Cargo behavior.

```powershell
.\scripts\verify.ps1
```

Runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo doc --workspace --no-deps`.
