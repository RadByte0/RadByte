# Plan to Eliminate NU Dependency

Date: 2026-11-15. Status: planned; implementation not completed by this document.

## Outcome

RadByte must build from the main tree without reading `NU/`, old `dist/`, or temporary copies from previous sessions. Packaging must automatically produce all three services in `dist/`. Protocol code must be decoupled from versioned registries and tags.

Distinct criteria: NU-independent build; operational capability with a compatible data package; verified provenance of materials for publication. Fulfilling the first criterion does not imply fulfillment of the other two. This plan does not abandon third-party Rust crates.

## What Currently Depends on NU

`scripts/package.ps1` creates a temporary copy of the source tree and supplements it with the following files:

| Material in NU | Current Usage | Replacement |
| --- | --- | --- |
| `RadCore/src/packet/configuration.rs` | Configuration phase packets mixed with version data | New packet implementation that receives data via an explicit interface |
| `RadCore/src/packet/configuration_core_tags.rs`, `configuration_static_tags.rs` | Embedded tags | Versioned data package and validated loader |
| `RadCore/src/mc777_data.rs` | Static registries | Loaded package provider without hidden fallback |
| `RadCore/data/mc777/{blocks,items,entities,menus}.json` | Data for building and packaging the version | External package with tracked source, version, and terms of use |

`PROVENANCE.json` is required as proof of provenance. Old `cli.rs` and `metrics/mod.rs` have already been replaced. The two generators in `NU/scripts/` are not needed for normal builds; they should not be copied over automatically.

## 1. Preserve Working State and Evidence

- Preserve existing worlds, configs, access lists, and the working local setup prior to replacing the implementation.
- Preserve `PROVENANCE.json` in `docs/legal/evidence/` along with existing hashes and upstream repository references. Do not alter recorded evidence under the guise of updating status.
- Use the [provenance document](../PROVENANCE_CLEAN_ROOM.md) as the primary explanation of known sources and restrictions. Do not create new duplicate reports.

Done when removing the operational dependency on `NU` will not destroy the sole evidence of provenance or user data.

## 2. Define Data Package Contract

- Inspect existing `RegistryProvider`, version package loader, and manifest format; extend them instead of creating a parallel system.
- Determine required fields: registry identifiers and block state IDs, properties, items, entities, menus, tags, and Configuration data actually needed by each service.
- Align schema, game/protocol version, hashes, provenance, and compatibility with the version library. A hash verifies integrity, not distribution rights.
- Validate sizes, duplicates, ID ranges, tag references, and completeness of required data. Eliminate placeholder fallbacks such as `block_id = 0` across all records.

Done when the contract is specified and verifiable against a small self-contained test fixture.

## 3. Replace Configuration Code

- Write a specification for required packets based on documented permissible sources; record the sources used for the new implementation.
- Implement serialization, decoding, and state transitions independently of tables. Renaming or reformatting an old file is not considered a new independent implementation.
- Pass registries and tags through the provider. Remove `include!` of old tables and references to `mc777_data` from the new execution path.
- Add meaningful tests for packet ordering, round-trip, invalid lengths, and incomplete data packages. Test fixtures must have their own documented provenance.

Done when main-tree modules compile without the three Configuration files from `NU` and without generated static registries.

## 4. Migrate All Services to Provider

- Audit consumers across RadCore, RadLimbo, RadProxy, RadServer, and RadVersions, including game mechanics and world saving.
- Remove default dependency on `bundled-mc777` and all hidden fallbacks to old tables. Refactor APIs where data is currently exposed as global constants.
- Without the required package, the server must terminate startup with a specific error before binding the game port. The full package must not be required for building or unit testing.
- Verify stability of IDs and block states. An incompatible package must not trigger automatic migration or overwrite world data; require explicit compatibility verification and backup.

Done when all consumers operate through the contract, and missing or incompatible data is handled without corrupting worlds.

## 5. Separate Data Acquisition from Distribution

- Select a source for the full package; verify its provenance and license/usage terms. Do not declare existing JSON files from `NU` permissible solely based on the intermediary repository's license.
- If distribution of the package cannot be confirmed, do not include it in public releases. Provide an explicit local packaging parameter, such as `-DataPackage <path>`, without automatic lookups in `NU` or old `dist`.
- Local import or generation also requires a clearly defined source and workflow; simply copying files or running an extractor does not grant redistribution rights.
- Do not substitute full registries with placeholder or zero IDs merely to pass startup. Restrict synthetic data strictly to isolated unit tests.

Done when code compiles cleanly without this data, and every consumed full package has explicit provenance and separate distribution status.

## 6. Streamline Build and Packaging

- Remove tree supplementation from `NU`, temporary source code copies, and assumptions about legacy local directories from `scripts/package.ps1`.
- Build the main workspace via `scripts/Build.ps1`. Align `scripts/verify.ps1`, examples, and CI to the same data contract.
- Separate binary/DLL compilation from bundling the verified data package. Local package assembly for testing all services should be achieved in a single command with an explicit package path.
- Keep services, runtime operational files, and management `scripts/` in `dist`. Store reports, build manifest, and version backups in `target/`. Do not overwrite saved worlds or configuration files.
- Clear the `quarantined_inputs` flag only after verified physical removal of those inputs; evaluate publication eligibility independently.

## 7. Final Verification Without NU

Execute in a clean, isolated directory with an empty Cargo target, free of `NU`, previous `dist`, and temporary session files. Do not rename or delete the user's active working folder.

- [x] Main workspace compiles from `Cargo.lock`; unit tests pass without full Minecraft data.
- [x] Build and packaging do not read `NU` or search for it via absolute paths, environment variables, or scripts.
- [x] RadServer, RadProxy, RadLimbo, version DLLs, and valid manifests are produced when supplied with an explicit compatible package.
- [ ] Direct status/Configuration/Play, network status, and proxy login succeed; GUI client launch verified separately or honestly marked unverified. *(Services start and bind ports, but network probes for Status/Configuration/Play and proxy login were interrupted and not completed; `NU/RadCore/data/mc777` also does not contain `tags.json`, so Configuration without a full package containing `tags.json` is rejected; GUI client was not tested.)*
- [ ] Shutdown, restart, world saving, and backup/restore succeed on a dedicated test world. *(Saving test world of `11` files and backup creation via `Backup-RadByte.ps1` (`5` files) verified; restart and `restore` not completed.)*
- [x] Missing, corrupted, and incompatible data packages are rejected prior to accepting players or mutating world state.
- [ ] Provenance of new code, test fixtures, and data recorded; unresolved distribution questions remain explicit. *(Evidence `PROVENANCE.json` preserved in `docs/legal/evidence/PROVENANCE.json`, but `mc777` data distribution rights remain `NOT_DETERMINED`, and the source for `tags.json` has not yet been selected.)*

Only after completing these verifications should deprecated copies and generators be removed from the working project. Retain all provenance evidence. Update checklist items in this document rather than creating a new report after each step.
