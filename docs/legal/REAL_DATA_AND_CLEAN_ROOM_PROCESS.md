# Analysis of 26.3 Real Data and Independent Process (Stages 5 & 6)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

## 1. Minecraft 26.3 Real Data (protocol 777)

Current rights status for each category of real data:

| Data Category | Current Artifacts in Working Copy | Rights to Acquire | Rights to Transform | Rights to Use | Rights to Distribute | Status |
| --- | --- | --- | --- | --- | --- | --- |
| **Block states & IDs** | `RadCore/data/mc777/blocks.json`, `mc777_data.rs` (`vineflower-direct` / `reports/registries.json`) | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | **NEEDS_LEGAL_REVIEW** |
| **Items, Entities, Menus** | `RadCore/data/mc777/{items,entities,menus}.json`, `mc777_data.rs` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | **NEEDS_LEGAL_REVIEW** |
| **Configuration Tags** | `configuration_static_tags.rs`, `configuration_core_tags.rs` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | **NEEDS_LEGAL_REVIEW** |
| **World / Anvil Format and Physics Constants** | `RadVersions/v26_3/src/world/*`, `shapes.rs` | Behavioral compatibility (interoperability) requires legal assessment | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | `NEEDS_LEGAL_REVIEW` | **NEEDS_LEGAL_REVIEW** |

**Important caveat:**
- Local acquisition of Minecraft data by the user or moving it to runtime loading (`versions/26.3/data/*.json`) **does not** automatically eliminate legal questions regarding rights to extract, transform, use, or distribute.
- Until a legal opinion is obtained, none of these real tables are to be included in the publication candidate.

## 2. Independent (Clean-Room-like) Process and Its Current Limitations

### Current Limitations (Honest Assessment)
1. The workspace lacks a `.git` directory, making cryptographic verification of author and commit history impossible.
2. The current implementation and AI sessions have already had access to `RadCore/data/mc777/*`, `mc777_data.rs`, and `configuration_*_tags.rs`. Rewriting code in a new session or with a new prompt **does not** make the result formally clean-room.
3. Accordingly, formal clean-room status is **NOT ENSURED** and is not claimed.

### Recommended Process for New Subsystems / Future Clean Implementation
1. **Permissible specification sources:** Only lawyer-approved public specifications of the protocol / format (e.g., officially permitted documentation or independent behavioral observations without copying decompiled code or proprietary assets).
2. **Separation of roles:**
   - *Researcher (Spec Author):* Analyzes permitted sources and drafts a behavioral specification (inputs, outputs, packet types, invariants, edge cases) without including snippets of third-party code or proprietary tables.
   - *Implementer:* Has no access to source materials/decompilations/unverified tables; works exclusively from the specification and approved test fixtures.
   - *Reviewer (Provenance Reviewer):* Verifies code compliance with the specification, license cleanliness of dependencies, and the access log.
3. **Access and change log:** Tracking in Git (signed commits / DCO or CLA), list of public/open sources, date, author, reviewer.
4. **Rules for tests and fixtures:**
   - Unit tests must use small synthetic fixtures explicitly marked with `is_synthetic_fixture: true` (`SyntheticFixtureRegistryProvider`).
   - Synthetic fixtures are not to be represented as complete Minecraft 26.3 registries and are not used as proof of real 26.3 client operation.
