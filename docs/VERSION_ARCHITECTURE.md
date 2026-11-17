# RadByte Version Architecture & Release Packaging

## 1. Source Code Structure

- `RadServer/` — version-agnostic host launcher (`rad-server`). Reads `versions/version.toml`, validates the selected package under `versions/<version>/`, verifies the native library's SHA-256, OS/arch, and C ABI (`RAD_VERSION_ABI_VERSION`), and delegates execution via `rad_version_run_server`.
- `RadCore/` — shared wire protocol framing, NBT, crypto/Velocity forwarding, `mc777_data` tables, and the generic `LoadedVersionPackage` / C ABI contract (`RadVersionInfo`, `RadServerStartOptions`, `VersionManifest`).
- `RadVersions/common/` — shared version contracts (`rad-version` crate): `VersionDescriptor`, `MC_26_3`, `WorldMechanicsProfile`, `check_world_compatibility`, `backup_world_dir`, and shared `shapes` / `physics` core.
- `RadAPI/` — WASM plugin ABI (`v1`).
- `RadVersions/v26_3/` — isolated implementation of **Minecraft Java 26.3 (protocol 777)** (`rad-version-26-3` / `rad_version_26_3` `cdylib`+`rlib`), owning 26.3-specific world generation, gameplay handlers, `Play` session loop, and `verify_mc777_tables()`.
- `RadProxy/`, `RadLimbo/` — proxy and limbo server crates.

## 2. Example RadServer Working Directory

```text
RadServer/
├── rad-server[.exe]
├── rad.properties
├── versions/
│   ├── version.toml              # User selection: version = "26.3"
│   └── 26.3/
│       ├── version.json          # Signed/hashed manifest (schema, protocol, DataVersion, ABI, files)
│       ├── rad_version_26_3.dll  # (Linux: librad_version_26_3.so, macOS: librad_version_26_3.dylib)
│       └── data/
│           ├── items.json
│           ├── blocks.json
│           ├── entities.json
│           └── menus.json
└── world/                        # User data; keep backups before changes
```

## 3. Version Selection & Launch Contract

1. `RadServer` resolves `versions/` relative to `--config` / `--versions-dir` / executable directory.
2. `RadServer/src/version_selection.rs` reads `versions/version.toml` (creating a default `version = "26.3"` only if `versions/` already exists and `version.toml` does not; rejecting symlinks, unknown fields, and unsafe/non-numeric version strings).
3. `LoadedVersionPackage::load_from_versions_dir` checks against path traversal and symlinks, verifies `manifest.game_version == selected_version`, and verifies `size_bytes` + `sha256` for all required datasets (`items`, `blocks`, `entities`, `menus`).
4. `verify_and_resolve_library_path` verifies `abi_version == RAD_VERSION_ABI_VERSION`, `target_os`, `target_arch`, library filename extension, `size_bytes`, and `sha256` without following symlinks or escaping the package directory.
5. `RadServer` loads the native library and checks C ABI symbols:
   - `rad_version_abi_version() == RAD_VERSION_ABI_VERSION`
   - `rad_version_info(&mut info)` matches `struct_size`, `abi_version`, `game_version`, `protocol_version`, `world_data_version`, `target_os`, and `target_arch` from `version.json`.
6. `rad_version_run_server(&opts)` starts the version module.

## 4. Adding a New Minecraft Version Implementation

1. Create `RadVersions/v<major>_<minor>/` as a new `cdylib` + `rlib` crate depending on `../../RadCore`, `../common`, and `../../RadAPI`, and register it in root `Cargo.toml`.
2. Export `rad_version_abi_version`, `rad_version_info`, and `rad_version_run_server` matching `RadCore::version_package` C ABI v1.
3. Add verified versioned registry data and use the Rust `rad-tools package-version` command for the currently supported 26.3 package. Supporting another version requires extending the Rust packager and validating that version's protocol and data first.
4. Do not copy `RadVersions/v26_3/` under another version number without implementing and verifying that version's protocol and data.

## 5. World Compatibility & Backups

- `rad_version::check_world_compatibility` checks `radbyte_world_profile.json` (`profile_id`, `game_version`, `protocol_version`, `world_data_version`, `storage_schema_version`) and `level.dat` (`DataVersion`) without mutating the world directory.
- Unmarked non-empty worlds without matching `DataVersion` return `UnmarkedInsufficientData` or `Incompatible` and refuse to auto-stamp.
- `backup_world_dir` rejects same-path, nested-in-source, source-in-destination, symlinks, and non-empty destinations.
- Cross-version world upgrades/downgrades are refused unless an explicit, backed-up migration is implemented and verified.

## 6. Testing & Packaging

Run the full verification gate:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo doc --workspace --no-deps
```

`cargo build --workspace --release --locked` places executables and the native library under `target/release/`. It does not assemble a runtime directory or include the separately supplied version data. The Rust `rad-tools package-version` command creates a new `versions/26.3/` package and refuses to overwrite an existing package. Keep worlds and configuration in a separate working directory and make backups before replacing files.
