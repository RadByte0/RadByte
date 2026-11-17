# Complete Plan for RadVersion Unification and World Protection

> [!NOTE]
> Historical plan. The current structure of `RadVersions/common/` and `RadVersions/v26_3/` is described in [VERSION_ARCHITECTURE.md](VERSION_ARCHITECTURE.md).

## Goal and Scope

Replace the current structure of `RadVersion` + large `RadVersion26_3` with a single
clear version layer `RadVersion`, in which `src/v26_3/` describes specifically
Minecraft 26.3 / protocol 777, while shared server, world, game mechanics, and
network primitives are not duplicated for each version.

Retain **one main branch** and **one active game version per launch**.
Do not add embedded ViaVersion, protocol translators, or empty modules
for future versions. Preserve the possibility of creating a standalone RadByte client
via the shared protocol API; a graphical client is not included in the initial release.

## Current State to Verify

- `RadVersion/src/lib.rs` contains the 26.3 descriptor and world verification;
  `physics.rs` and `shapes.rs` have already been partially extracted.
- `RadVersion26_3/src/` still contains `config`, `gameplay`, `net`, `radon`,
  `world`, and native ABI v1. `RadServer` launches the version DLL from the package.
- `ConfirmedLegacyRadByteWorld` can currently be determined by matching `level.dat`
  `DataVersion`; the number itself **does not prove** that this is a RadByte world or that all
  data is compatible. The world profile must also be verified alongside the actual
  `level.dat`, rather than relying solely on marker values.
- The existing `RadCore::client` is a headless foundation, not a completed GUI client;
  online-mode/authentication and integration must be verified separately.
- Registry data has unverified provenance; adhere to
  `docs/PROVENANCE_CLEAN_ROOM.md`, do not label code clean-room automatically.

## Target Structure (Reference)

```text
RadByte/
├── RadServer/          startup, config, server sessions
├── RadCore/            shared wire types, codecs, NBT, network primitives
├── RadVersion/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── common/     contracts, profiles, compatibility
│   │   └── v26_3/      descriptor, packet/data mappings, version rules
│   └── data/           strictly per verified provenance policy
├── RadGame/ or RadServer/ modules   shared mechanics
├── RadWorld/ or RadServer/ modules  persistence, chunks, migration
├── RadProxy/
├── RadLimbo/
└── RadAPI/
```

Create separate crates only if justified by dependencies. The Rust module name
is `v26_3`, while the data directory may be `26.3`. Do not move
the entire content of the old DLL into `v26_3` under a new name, preserving duplication.

## M0. Baseline Verification

1. Read applicable AGENTS.md, check Git/its absence, running processes,
   current changes, and versions of `.exe`, DLL, and data. Do not overwrite others' work.
2. Review ``cargo test --workspace --locked`` and related scripts: they may modify
   `dist` and terminate the server. Preserve baseline tests and failures; do not display
   secrets or inventory contents in logs.
3. Map dependencies and usages of `PROTOCOL_VERSION`, version package,
   world schema, C ABI, RadAPI, status/login/configuration/Play, and server state.
4. Configure isolated temporary worlds and ports. For copies of existing worlds,
   make a backup; never modify the user's world during tests.

**Done:** current behavior, current errors, and compatibility points are measured.

## M1. Safe World Verification

1. Explicitly distinguish `new_empty`, `verified_radbyte`, `unmarked_ambiguous`,
   `incompatible`, `corrupt`, and other necessary states; the status "confirmed
   legacy RadByte" cannot be inferred solely from vanilla `DataVersion`.
2. If `radbyte_world_profile.json` is present, verify it alongside
   the actual `level.dat`, schema marker, known format characteristics, and other
   relevant metadata and references. Do not mask discrepancies and read errors
   as an empty/compatible world.
3. A world without verified provenance requires safe rejection or explicit import
   following a dry-run and backup. Do not automatically mark it as RadByte 26.3.
   Do not block viable existing user worlds without diagnostics and a verified
   migration path tested on a copy.
4. For a new world, ensure that the directory is truly empty (including
   unknown files); do not assume emptiness merely because only a few
   known files are listed. The check itself must not alter disk state.
5. Distinguish game/protocol/registry/world DataVersion/storage schema/ABI;
   define a complete compatibility matrix. Do not mark older lower schemas as
   fully compatible without a concrete migration path.
6. Create the marker only after validating the package, config, and world; ensure
   safe writes with symlink/reparse point verification, unique temporary
   names, `create_new`, and error handling. Verify ordering on bind failure.
7. Tests: new/legacy/foreign/incomplete/corrupt world; marker vs `level.dat`
   mismatch; missing/newer/older schema; unknown files; absence of
   modifications after rejection; restart on a copy and migration from backup.

**Done:** no unconfirmed world silently acquires new rules.

## M2. Safe Backup/Restore and Migrations

1. Verify `backup_world_dir`: absolute/relative and non-existent paths,
   `..`, source=destination, parent/child directories in both directions,
   Windows reparse points, symbolic links, junctions, and case-insensitive paths.
2. Do not overwrite non-empty destination. Copy into staging with explicit
   reporting of partial failure and without automatic deletion of the original.
   Do not claim race/TOCTOU protection without verification; where required, use
   handle-based operations or documented guarantees.
3. Restore must be a separate operation, verify hashes/sizes, reject
   unsafe paths, and not replace the active world without a specific backup.
4. Integration tests with standard world layout, corrupted files,
   write failure, symlinks, nested paths, and re-launch.

**Done:** safe, verified operations are available on temporary copies.

## M3. Version Module 26.3

1. Create `RadVersion/src/v26_3/mod.rs` and migrate the 26.3 descriptor,
   version metadata/capabilities, and required mappings from existing sources.
2. `common/` contains only shared interfaces and types. The version registry advertises
   only genuinely implemented versions, currently 26.3. Do not modify protocol 777.
3. Gradually isolate packet IDs, disparate clientbound/serverbound layouts,
   registry state mappings, and rules that truly depend on 26.3. Shared
   primitives, codec, and NBT must not depend on the version implementation.
4. Do not duplicate data as unrelated compile-time and runtime sources: establish
   reproducible generation/hash verification, provenance, and semantic consistency.
5. Reject incompatible protocol at Login with a proper reason prior to invoking
   the version-specific parser; verify status ping separately. Do not perform translation.

**Done:** `RadVersion` genuinely contains module `v26_3`, not merely a description in `lib.rs`.

## M4. Extraction of Shared Server

1. Refactor in phases: network/session/state → world/storage → gameplay →
   config/radons/launcher. Preserve previous semantics; do not replace
   functional mechanics with stubs for the sake of clean structure.
2. For each extraction, define concrete shared contracts and boundaries, eliminate
   dependency cycles, run targeted tests first, followed by full verify.
3. Verify status, login/configuration/play, movement, collisions, blocks, redstone,
   inventory, dropped items, playerdata, time, chunks, world save/restart, and proxy.
4. After extraction, determine whether legacy `RadVersion26_3` serves as a temporary
   ABI shim. Remove it only after proven transition of build, tests,
   package, and compatibility. Do not extract code by copying, leaving dual sources of truth.

**Done:** shared server does not reside in a separate crate for each version.

## M5. DLL, ABI, API, and Packaging

1. Define and document the chosen final model: a unified single
   executable or executable + compact version DLL. Verify user needs
   and cross-platform requirements; do not create empty `libraries/`.
2. Preserve C ABI v1 as a compatibility shim or introduce an **explicitly new** ABI
   version: struct size, pointer lifetime, ownership, errors, target OS/arch,
   unwind, and handling of malformed modules. Do not consider Rust ABI stable.
3. Do not break RadAPI v1 or existing native/WASM addons; write compatibility and
   safe rejection tests for incompatible formats.
4. Update `Cargo.toml`, build, verify, generators, launcher, `dist`, package
   `versions/26.3` or its documented replacement, and manifests/checksums.
   Do not overwrite existing configs, worlds, ops/whitelist, radons, or secrets.
5. Stage release in a separate directory; verify binary/module/data
   compatibility and startup before moving; in case of required migration — backup and recovery.
   Verify launch from a different working directory and both clean/upgrade installations.

**Done:** release launches without data loss and clearly specifies compatibility.

## M6. Foundation for Future Client

1. Develop `RadCore::client`/new shared protocol layer: status,
   login, configuration, Play, timeouts, bounds, compression, encryption
   only if implemented; separate client and server contracts without cycles.
2. Design online authentication according to current official terms, without
   storing passwords, bypassing licenses, or logging tokens; if not ready,
   state this and safely reject unsupported modes.
3. Independent wire fixtures and real official client as reference; headless
   roundtrip with the same codec is not proof of full protocol compliance.
4. Define future boundaries for rendering, chunk cache, prediction, GUI, input, and
   assets; do not create a sham graphical client. Do not distribute Minecraft JAR,
   textures, and sounds without verified rights.

## M7. Acceptance

- Full verification of `fmt`, `clippy -D warnings`, workspace tests, docs,
  ``cargo test --workspace --locked``, release build, and isolated `dist` smoke tests.
- Official 26.3 client: status → login → configuration → Play, movement,
  inventory, blocks, time, save/restart; proxy/limbo if supported.
- Unsupported protocol safely rejected; version package and
  ABI mismatch, bad hashes, path traversal, timeout, and resource limits handled.
- Copies of legacy worlds/configs, backup/restore, playerdata, block entities,
  scheduled ticks; rejection of foreign/newer/ambiguous formats.
- OS/architecture versions are considered verified only after actual execution
  on the target. GUI client and new game versions are not marked as ready.
- Update README, architecture, migration, limitations, clean-room provenance, and
  results table. Leave the user's server stopped after the build.

## Execution and Reporting

Work in stages without stopping after analysis or a single fix; do not
duplicate already completed work. In case of external blocking, continue independent
tasks. Do not promise completion of a massive plan in arbitrary time; do not
obscure incomplete requirements, test results, or the absence of GUI testing.
Report: structure before/after, what was actually migrated, whether an ABI shim remains,
world and backup status, complete test results, `dist`, risks, and remaining work.
