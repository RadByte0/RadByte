# Data Dependency Graph (Updated after Stages 1–4)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

## 1. Decoupling Status by Crates and Subsystems

| Crate / Subsystem | Dependency on `mc777_data` / Tags without `bundled-mc777` | Build and Test Status without `bundled-mc777` | Notes |
| --- | --- | --- | --- |
| **`RadAPI`** | None | `PASS` (1/1 test) | Fully independent |
| **`RadProxy`** | None (`rad-core` with `default-features = false`) | `PASS` (24/24 tests) | Fully independent |
| **`RadCore`** | Isolated behind `#[cfg(feature = "bundled-mc777")]` and `RegistryProvider` | `PASS` (41/41 unit + 3/3 doc tests) | `ConfigUpdateTags::try_for_registries` fails with `MissingData` if `bundled-mc777` is disabled |
| **`RadLimbo`** | Explicit dependency via `ConfigUpdateTags::try_for_registries` + `bundled-mc777` feature | `PASS` (5/5 tests `--no-default-features`) | Fails configuration handshake without real tags (no silent fallback) |
| **`RadServer` (host binary)** | None in `src/main.rs` (`LoadedVersionPackage` at runtime) | `PASS` (`cargo check -p rad-server --bins --no-default-features`) | `tests/headless_wire_e2e.rs` contains 1 integration test with `#[cfg(feature = "bundled-mc777")]` |
| **`RadVersions/common` (`rad-version`)** | Fully decoupled (`shapes.rs`, `physics.rs` migrated to `RegistryProvider`) | `PASS` (4/4 tests `--no-default-features`) | `block_collision_shape_with`, `block_interaction_shape_with`, `validate_player_move_with_pose_lookup_with` |
| **`RadVersions/v26_3` (`rad-version-26-3`)** | Decoupled from compile-time `mc777_data` via `Version26_3Registries` and `*_with` | `PASS` (25/25 unit tests `--no-default-features` on synthetic fixtures) | Production `ServerState::new_with_world_dir_and_package` passes `allow_synthetic = false` (fail-closed without real registries); `net/login.rs` uses `ConfigUpdateTags::try_for_registries` and fails in configuration without tags |
