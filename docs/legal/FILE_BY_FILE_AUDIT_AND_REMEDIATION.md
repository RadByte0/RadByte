# File-by-File Technical Audit and Rights Clearance Plan

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

Date: November 15, 2026. This is a technical investigation, not a legal assessment of individual files and not a guarantee of their publishability.

## Scope and Materials

- FILE_REVIEW_CURRENT.csv: one record for each of the 206 files outside output directories and local third-party installations; the entire file was automatically read, SHA-256 computed, text markers and line numbers identified. This is not a manual legal review of each line and not a confirmation of authorship.
- LOCAL_ARTIFACTS_INDEX.csv: 55,337 paths/sizes in `target`, nested `target`, `artifacts`, `dist`, `versions`, `Paper-26.3`; metadata only, contents, nested archives, and rights were not separately reviewed.
- FILE_REVIEW_SUMMARY.json: statistics and recorded limitations. Rerun the scanner with `cargo run -p rad-tools -- audit-provenance`; its own outputs are intentionally not scanned to avoid self-reference.
- Automated categories: 8 explicitly defined sources or derived registries require separate legal review; 7 local configs/data should be preserved and not accidentally included; 191 other files have unverified authorship and rights based on this scan. This is not an allegation of infringement or third-party ownership. Markers neither prove nor disprove copying.
- The state was captured prior to the creation of this report; a re-run will cover it as well. For the current release, a fresh scan with hashes is required.

## Findings and Actions

### P1. Version Data — Rights Unknown

`RadCore/data/mc777/{blocks,items,entities,menus}.json` specify `vineflower-direct` and `reports/...` in `_provenance`; `PROVENANCE.json` describes local snapshots. `RadCore/src/mc777_data.rs` explicitly names them as the generation source. `RadCore/src/packet/configuration_static_tags.rs` is marked auto-extracted, and `configuration_core_tags.rs` is marked extracted from client data. These are facts about claimed provenance, not a conclusion of illegality.

- [ ] Separate facts, assumptions, and questions regarding the rights to acquire, transform, use, and distribute each group.
- [ ] Do not include these specific materials and their derivatives in the publication candidate until legal grounds are established; do not delete local originals.
- [ ] Review available licensed sources/permissions or document an independent replacement; automated local acquisition is not proof of permissibility.
- [ ] Verify downstream: `the removed data-generation script`, ``rad-tools package-version``, version packages, statically compiled binaries, fixtures, and staging; separately verify source and binary for actual content.
- [ ] For replacements, verify correct 26.3 IDs/states/tags, network handshake, save compatibility, and real client operation; in the absence of data — fail-closed, not synthetic production.

### P1. Authorship and Chosen License

`Cargo.toml` declares `MIT OR Apache-2.0`; the available working folder does not contain `.git`, and root `LICENSE-MIT` and `LICENSE-APACHE` were not found during the check. Absence of history is not proof of absence of copyright. Existing inventory with group or automated statuses does not confirm chain of title.

- [ ] For the owner to establish authors and known third-party borrowings; check other local copies/history if available, without inventing evidence.
- [ ] At the file level, determine author rights and licensing basis; provide proper notices to third parties.
- [ ] If proper rights are established and upon the owner's decision, add authentic texts of the chosen licenses; declaration in Cargo is not permission for third-party content.

### P1. Public Package and Third-Party Artifacts

- [ ] Treat `Paper-26.3`, `target`, `dist`, `artifacts`, `versions`, and nested builds as artifacts; do not publish them automatically. They may contain JARs, binaries, worlds, logs, and third-party content. The presence of `.gitignore` does not verify release archives and history.
- [ ] Create separate staging based on an allowlist; each included file must have an evidence-based decision rather than an automated status. Do not modify working data.
- [ ] Recursively verify the actual source/binary archive, nested zip/JARs, embedded data, secrets, and Git/CI history, if any exist. This audit did not perform such a scan.

### P2. Dependencies

- [ ] For the actual target/features and build/runtime, separately obtain the complete dependency graph, SBOM, SPDX expressions, license texts, and copyright/NOTICE according to each license. `Cargo.lock`/metadata and the presence or absence of LICENSE in the cache do not constitute full verification.
- [ ] Do not remove `tokio`, `serde`, and other libraries solely due to license; document the selected `OR` condition, verify transitive data, and fulfill relevant distribution requirements.

### P2. Name, Examples, and Documentation

- [ ] Review `README.md`, assets, examples, project name, and official Minecraft rules; do not treat a disclaimer as permission for third-party materials.
- [ ] `examples` contains nested `target` with binaries; do not add them to the repository/archive. Verify sources and fixtures for each example separately.
- [ ] Identify 100% clean-room claims and revise only after verifying evidence; access to third-party material does not in itself prove copying.

## Completion Process

1. The owner confirms authors and known borrowings/transfers; sources and evidence are recorded in the inventory.
2. For each unconfirmed group: verify rights, obtain permission, replace with verified independent implementation/data, or exclude from the package. Do not rewrite everything automatically.
3. Conduct comparative tests on an isolated world: protocol, registries, blocks/items, reload, backup, GUI. Synthetic tests do not prove 26.3 compliance.
4. Prepare package review with specific questions regarding data and jurisdictions; if available, submit to a legal specialist. If unresolved, the NEEDS_REVIEW status remains.
5. Publish only upon a separate decision by the owner after reviewing the actual package, documentation, and unresolved questions. Absolute guarantees are unavailable.

Primary sources of terms: [Minecraft EULA](https://www.minecraft.net/en-us/eula), [Minecraft Usage Guidelines](https://www.minecraft.net/en-us/usage-guidelines), [MIT](https://opensource.org/license/mit), [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0).
