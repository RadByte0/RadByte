# Dependency Licenses & Provenance Scan Summary

> **Note:** This document is a technical inventory generated from repository tooling (`cargo metadata --format-version 1 --locked` and a provenance marker scan, now `cargo run -p rad-tools -- audit-provenance`). It is **not** legal advice and does not establish legal cleanliness or copyright clearance.

## 1. Workspace License Declaration
- `Cargo.toml` declares `license = "MIT OR Apache-2.0"`.
- Root `LICENSE` / `LICENSE-MIT` / `LICENSE-APACHE` files have not been added yet because the copyright holder name and final licensing decision must be confirmed by the project owner.

## 2. Third-Party Cargo Dependency Licenses (`Cargo.lock`, 252 external crates)
Observed via `cargo metadata --format-version 1 --locked` (filtering packages where `source` is non-null):

| License Expression | Crate Count |
| --- | ---: |
| `MIT OR Apache-2.0` | 131 |
| `Apache-2.0 OR MIT` | 21 |
| `MIT` | 40 |
| `Unicode-3.0` | 18 |
| `MIT/Apache-2.0` | 12 |
| `Apache-2.0` | 4 |
| `Unlicense OR MIT` | 4 |
| `ISC` | 3 |
| `Zlib` | 2 |
| `Apache-2.0 OR ISC OR MIT` | 2 |
| `Unlicense/MIT` | 2 |
| `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 2 |
| `BSD-2-Clause OR Apache-2.0 OR MIT` | 2 |
| `MIT OR Apache-2.0 OR Zlib` | 1 |
| `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 1 |
| `Apache-2.0 AND ISC` | 1 |
| `Apache-2.0 OR BSL-1.0` | 1 |
| `BSD-3-Clause` | 1 |
| `Zlib OR Apache-2.0 OR MIT` | 1 |
| `(MIT OR Apache-2.0) AND Unicode-3.0` | 1 |
| `CDLA-Permissive-2.0` | 1 |
| `Apache-2.0/MIT` | 1 |

`fastnbt 2.6.3` is a development-only dependency of `rad-core`. Its serializer and deserializer are used as an independent compatibility reference in the network NBT tests; no FastNBT implementation code is copied into RadByte's runtime. The crate metadata declares `MIT OR Apache-2.0`. The tested source revision is recorded with its notices in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

## 3. Automated Provenance Scan

The scanner can be rerun with `cargo run -p rad-tools -- audit-provenance`.
Last run output:
- `source_files_scanned`: `174`
- `source_statuses`: `{"AUTHORSHIP_AND_RIGHTS_UNVERIFIED": 169, "LOCAL_DATA_REVIEW": 5}`
- `errors_or_symlinks`: `0`
- **Limitation:** Automated marker scan only; does not verify authorship, clean-room separation, or redistribution rights.

## 4. External Data Package Status
- `NU/` and stale `dist/RadServer/versions/26.3/` were removed from the workspace.
- Historical provenance evidence for `mc777` is preserved in `docs/legal/evidence/PROVENANCE.json` (`redistribution_rights: NOT_DETERMINED`).
- `RadServer` requires an externally supplied `-DataPackage` at packaging/runtime; no Minecraft game data is bundled in the repository or compiled binaries.
