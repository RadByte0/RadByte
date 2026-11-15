# Three Modes and Status of Staging Copies (Stage 5)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

## 1. Three Clearly Delineated Modes

### A. Local Mode with Available Local Data (`bundled-mc777` enabled)
- Uses existing local tables `mc777_data.rs`, `RadCore/data/mc777/*`, `configuration_static_tags.rs`, `configuration_core_tags.rs`, and local `versions/26.3/`.
- Intended **exclusively for local development and functionality verification**.
- Local operation **does not constitute** permission to publish or distribute this data.

### B. Technical Mode with Synthetic Fixtures (`artifacts/staging_allowlist/`, `bundled-mc777` disabled)
- Excludes `RadCore/data/mc777/*`, `RadCore/src/mc777_data.rs`, `RadCore/src/packet/configuration_static_tags.rs`, `RadCore/src/packet/configuration_core_tags.rs`, `Paper-26.3/`, `dist/`, `versions/`, `target/`.
- In staging, the following build and pass tests on synthetic fixtures (with `bundled-mc777` disabled): `rad-api`, `rad-core`, `rad-limbo`, `rad-proxy`, `rad-version` (`RadVersions/common`), `rad-version-26-3` (`RadVersions/v26_3`), as well as the `rad-server` binary (`cargo check -p rad-server --bins`).
- **Important:** synthetic fixtures (`SyntheticFixtureRegistryProvider`, `synthetic_26_3_test_provider`) are artificial data for unit tests and **do not prove** that a real Minecraft 26.3 client works; real handshake / client 26.3 in clean staging without local data remains `BLOCKED` / `NOT_RUN`.

### C. Release Candidate for Publication
- **Status:** **BLOCKED (not formed)**.
- All proprietary files without `.git` history and authorship confirmation have `UNKNOWN` status; root `LICENSE-MIT`, `LICENSE-APACHE`, and `NOTICE` are missing; real 26.3 data has `NEEDS_LEGAL_REVIEW` / `EXCLUDED` status.
- A candidate containing `UNKNOWN` files cannot be considered verified for publication.
