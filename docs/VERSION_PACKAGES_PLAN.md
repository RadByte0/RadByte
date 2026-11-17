# RadByte Version Packages and Libraries Plan

## Goal

Prepare a clear file structure for server programs in which RadServer runs as a single executable, and Minecraft 26.3 data resides in an actively used `versions/26.3/` package. Do not mistake the mere presence of files for support of other protocols or dynamic libraries.

```text
RadServer/
├── rad-server.exe                 # Windows; corresponding executable on other OSes
├── rad.properties
├── config/
│   └── radserver-extra.properties
├── versions/
│   └── 26.3/
│       ├── version.json
│       └── data/                  # only data that is actually used
├── radons/
├── radonfigs/
└── world/
```

`libraries/` is created only for genuinely required dynamic libraries with implemented loading and compatibility verification. Native addons remain in `radons/`. Verify current file and directory names prior to implementation.

## Principles

- Initially support **only 26.3 / protocol 777**. Do not promise connections from other clients or their gameplay behavior without complete implementation and verification.
- Data in an external package must affect execution. If a given table is required at compile time, clearly mark its compile-time dependency and establish a single authoritative source rather than duplicating it with divergent values.
- Absence, corruption, or mismatch of the required package must produce a clear error prior to opening the network port; do not substitute a corrupted package with another version. Document handling rules for older installations in advance.
- Separating Rust modules does not require DLLs. Introduce new crates only along real boundaries of responsibility; do not create cyclic dependencies.
- Do not copy Mojang/Paper code or assets without license verification. Preserve data provenance, generation rules, and checksums.
- Do not alter user worlds, access lists, addons, secrets, or configs during testing or packaging. Use temporary worlds.

## Stage 0. Audit and Baseline

1. Read applicable AGENTS.md, inspect current checkout state, active processes, and results of ``cargo test --workspace --locked``; do not overwrite others' changes.
2. Find all usages of `mc777_data`, `PROTOCOL_VERSION`, packet IDs, registries, tags, NBT, feature flags, and hardcoded 26.3 values across all five crates. Separately identify runtime and compile-time dependencies.
3. Audit `RadCore/data/mc777/{blocks,items,entities,menus}.json`, `PROVENANCE.json`, `the removed data-generation script`, associated generators, and actual table usage. Do not treat them as an exhaustive package set.
4. Verify execution paths directly, via `dist`, and tests with a temporary working directory. Determine actual dependencies of RadProxy and RadLimbo.
5. Record baseline functional, protocol, and persistence tests.

**Done:** Documented data map, known limitations, and failures prior to changes.

## Stage 1. Module Boundaries

1. Separate representations of `GameVersion`, protocol ID, and feature descriptors. Clearly distinguish network version, content version, and world format.
2. Decouple general codecs from 26.3 packet IDs and registries. Preserve existing APIs or add backward-compatible adapters; do not alter RadAPI v1 ABI.
3. In code, introduce a version package loader and explicit component dependencies on loaded data. For numerical constants requiring compile-time availability, align generation with the same source or migrate consumers to lookup.
4. Do not perform large-scale refactoring simultaneously with protocol overhaul. Run relevant tests after each logical move.

**Done:** Clear interface between core and 26.3 data; no silent divergence between embedded and external tables.

## Stage 2. Package Format and Generation

1. Define documented `version.json`: schema version, game version, protocol ID, compatibility with package format version, relative file paths, hashes, and provenance. Do not confuse Minecraft version with RadByte ABI version.
2. Prohibit path traversal, absolute paths, duplicates, excessive sizes, unknown critical fields, and invalid types. Explain handling of optional fields and future schema versions.
3. Create a reproducible package generator from authoritative data. Validate integrity and semantics: unique IDs/names, consistent references, ranges.
4. Checksums detect accidental modifications, but alone do not prove authenticity; design signing separately for trusted third-party packages.
5. Data inside the package must not contain personal data or secrets.

**Done:** Package is reproducible, versioned, possesses a complete list of required data, and validates independently of network startup.

## Stage 3. Runtime Loading for 26.3

1. Add package selection configuration only after implementing its effect; maintain support for existing configurations and document defaults.
2. Determine installation directory predictably and support explicit paths for services/tests. Do not silently depend on the current working directory.
3. Prior to listener creation, load manifest, verify compatibility, integrity, and all required tables; report error path and reason without leaking secrets. Explicitly define behavior upon partial update.
4. Connect real consumers: registries, IDs/tables, login/configuration, status, and Play — based on audit results. Ensure test package modification visibly affects expected lookup/packet or is rejected.
5. Migrate older installations explicitly: construct package from trusted local assets or report required update. Do not overwrite config/world, do not download assets from the network without clear necessity.
6. Align RadCore, RadServer, RadProxy, and RadLimbo; do not force packages onto programs that do not genuinely need them. Do not alter protocol 777.

**Done:** Server actively reads required version data; corrupted data does not result in startup with invalid registries or non-obvious fallback.

## Stage 4. Libraries and Portability

1. Keep standard Rust dependencies statically linked unless separate loading offers specific benefit. Do not create an empty `libraries/` directory merely for appearance.
2. If a loadable protocol module is needed, first agree upon and document a stable C ABI, interface version, ownership/buffer/error rules, lifecycle, architecture compatibility, and graceful failure.
3. For native modules: `.dll` Windows, `.so` Linux, `.dylib` macOS; precise target and dependencies, loading path control, and error isolation. Do not treat Rust ABI as stable and do not load untrusted libraries.
4. Keep RadAPI v1 and WASM/native Radons contracts compatible; verify actual implementation of each type before claiming support.
5. Support builds for available targets; mark platforms where execution could not be verified as unverified rather than ready.

**Done:** Either justified absence of DLLs or a genuinely verified loading contract; public ABI unchanged without migration.

## Stage 5. Packaging, Updating, and Testing

1. Update ``cargo build --workspace --release --locked``, `Cargo workspace build`, launcher, smoke tests, and documentation so that release builds and places all necessary data into `dist/RadServer/versions/26.3/`. For other OSes, create equivalent verified instructions or scripts.
2. Update version package via staging, validation, and atomic move; do not leave a half-updated installation. Do not overwrite working data, config, list files, addons, or secrets. Create backups before migration.
3. Tests: generator, schema, hashes, missing/extraneous/corrupted files, path traversal, wrong version/protocol, oversized input, launch from a different directory, compatibility with old installation and config.
4. Integration tests in temporary `dist`: startup, status, login, configuration, play, blocks/inventory, and restart/persistence. Separately test proxy and limbo if they use version data.
5. Verify execution with real 26.3 client if available and if it does not disrupt user workflow; explicitly indicate absence of verification.
6. Execute ``cargo test --workspace --locked``, release build, and smoke. Gracefully stop working server with persistence only when updating `dist` is necessary, without forced termination; leave launch to user after build.

**Done:** Clean install and upgrade pass checks, user data is preserved, list of actually supported platforms and versions is accurate.

## Stage 6. Future Minecraft Versions

For the next version, separately implement and test packet IDs and structures, registries, configuration/play, block/item semantics, world, and version transitions. Translating client protocol and handling old server mechanics are distinct tasks; adding JSON to `versions/` does not automatically achieve either. Do not migrate worlds between versions without an explicit tool and backup.

## Continuous Execution and Reporting

The implementer works stage by stage, inspects actual code, and does not duplicate existing work. Runs sufficient tests after each stage and proceeds. If a specific platform or real client is unavailable, carries out independent work, records limitations, and makes no claim of verified support. Concludes by reporting actual `dist` structure, verification results, runtime data presence, preserved compatibility guarantees, and pending items.
