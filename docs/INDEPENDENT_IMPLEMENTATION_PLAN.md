# RadByte: Independent Implementation and Verifiable Provenance Plan

Date: November 15, 2026. Goal — prepare an independent Minecraft-compatible implementation with documented sources and rights for a potential public release. The plan mitigates risks, but does not constitute a legal opinion and does not guarantee the absence of claims. Do not claim that the current project or resulting code is clean-room solely because this plan was executed.

## Mandatory Rules

- Preserve the working RadByte, worlds, configs, backups, and third-party installations. Do not delete original files for the sake of cleanup.
- Before starting, read the applicable AGENTS.md, [PROVENANCE_CLEAN_ROOM.md](PROVENANCE_CLEAN_ROOM.md), and LEGAL_REUSE_RELEASE_PLAN.md.
- Determine permitted sources and methods of acquisition/use/distribution with legal counsel. A public webpage, repository, or locally obtained data does not by itself prove copying rights.
- Do not transfer decompiled code, data, or tables with unknown rights, or protected assets into the release candidate based on assumptions.
- A true clean-room process requires controlled separation of access and roles: a person or agent who has seen a potentially impermissible source does not become an independent developer merely through a new prompt or session. Record the absence of proper isolation as a limitation.
- No publications, global license changes, or data deletions without explicit instruction from the owner.

## M0. Baseline State and Preservation

- [ ] Verify workspace, current changes, Git history if present, active processes, component versions, and script safety.
- [ ] Freeze hashes, tests, package structure, and limitations; compile a list of existing worlds and files that must not be modified.
- [ ] Prepare a separate isolated staging area using an allowlist, excluding files of unknown provenance from the release candidate. If a copy fails to build — honestly record the reason.

## M1. Provenance Inventory

- [ ] For code, data, fixtures, graphics, documentation, generators, and artifacts, record `path, SHA-256, author, primary source, commit/version, acquisition method, license, terms, evidence, distribution location, status`.
- [ ] Statuses: `CONFIRMED`, `UNKNOWN`, `NEEDS_REVIEW`, `EXCLUDED`; technical status does not substitute for a legal decision.
- [ ] Separately investigate `RadCore/data/mc777/`, tables generated from them, `configuration_static_tags.rs`, registry/tag snapshots, test data, and third-party JARs. Do not claim that they infringe rights solely based on source labeling.
- [ ] Reconstruct authorship history and third-party contributions as much as possible; explicitly record the absence of `.git` or history.
- [ ] Build a dependency graph: which generators, runtime files, packages, and tests depend on `UNKNOWN`.

## M2. Permissible Sources and Licensing Rules

- [ ] For each source, determine permissible actions: reading, analysis, specification authoring, copying, generation, and distribution; preserve source, date, revision, and verification.
- [ ] Verify actual licenses of each file, embedded data, and dependencies; for MIT/Apache, retain required copyright, license, and NOTICE.
- [ ] Separately evaluate GPL/AGPL and component interaction terms; do not assume that a DLL or separate process automatically alters licensing obligations.
- [ ] Define rules for protocol data, interoperability, trademarks, and Minecraft assets taking jurisdiction and legal advice into account.
- [ ] Do not modify `MIT OR Apache-2.0` and do not add false notices; establish author rights before finalizing project licenses.

## M3. Isolated Specification Process

- [ ] Under rules agreed upon with legal counsel, assign a separate role to investigate permissible sources, maintain the provenance log, and create behavioral specifications.
- [ ] The specification describes input/output, observable behavior, errors, limitations, and tests without copying impermissible code or protected expressive materials.
- [ ] Provide implementers only with verified specifications and permissible fixtures. Record which files/sources were accessible to them; prior access to original code poses a risk to a clean-room claim.
- [ ] If roles, independence, and access control cannot be ensured, continue technical audit and independent development without claiming formal clean-room status; convey limitations to legal counsel.

## M4. Subsystem Implementation

Determine sequence according to dependency graph and utility; do not replace working code without rights verification and tests.

- [ ] Protocol types/codec/NBT and network limits.
- [ ] Registries and version package: state, ID, block/item mapping, tags, and their provenance; do not fabricate data for builds.
- [ ] World and persistence: block states, chunks, world/playerdata, safe backup, and unsupported migrations.
- [ ] Physics and interaction shapes: placement, break, neighbor updates, falling blocks.
- [ ] Inventory, drops, crafting, menus, component persistence.
- [ ] Redstone, ticks, mechanics, pistons, followed later by entities, AI, worldgen, commands, and other items from [RADBYTE_FEATURE_CHECKLIST_26_3.md](RADBYTE_FEATURE_CHECKLIST_26_3.md).
- [ ] For each replacement: specification and permissible sources → isolated implementation → independent test scenarios → cross-chunk/multiplayer/restart as needed → provenance entry → re-verification.
- [ ] Use third-party libraries only after auditing specific version, notices, data, transitive dependencies, and technical compatibility.

## M5. Provenance Verification, Automation, and Release

- [ ] Generate SBOM for actually included dependencies/platforms/features; include required license/copyright/NOTICE.
- [ ] Verify actual contents of source and binary archives, generated files, embedded archives, logs, secrets, JARs, assets, and history/tags/releases if present; `.gitignore` is not considered proof.
- [ ] Assemble staging only from verified or explicitly permitted materials; if impossible — `BLOCKED`, without data substitution.
- [ ] Run fmt, clippy, tests, doc, 26.3 test run, persistence/restart, and GUI client if available. Record exact versions, hashes, and `PASS/FAIL/NOT_RUN`.
- [ ] Prepare legal package: sources, role/access log, `UNKNOWN` files, release composition, authorship, dependencies, EULA/branding, jurisdictions, and open questions.
- [ ] Consider publication only after rights holder review and user decision. Final status determination depends on evidence and legal evaluation.

## Acceptance Gates

1. **Inventory:** Every release file has proof of provenance or is marked as unknown and excluded.
2. **Independence:** Access boundaries and roles are documented; if unfulfilled, clean-room is not claimed.
3. **Technical Readiness:** Verified build, network, persistence, and restart scenarios; GUI/platforms noted separately.
4. **Publication:** Licensing terms and rights are verified, open questions are submitted to legal counsel and decided by the owner.
