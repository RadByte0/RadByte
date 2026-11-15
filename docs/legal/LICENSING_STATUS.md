# Licensing and Attribution Status (Stage 2)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

## 1. Declared Workspace License

- In the root `Cargo.toml` (`[workspace.package]`), the following is declared:
  - `license = "MIT OR Apache-2.0"`
  - `repository = "https://github.com/radbyte/radbyte"`
- The project's global license was **not modified** during this technical audit.

## 2. Missing Root License and Authorship Files

At the time of the review, the workspace root `c:\Users\maksi\Documents\Projects\radbyte` **lacks**:
- `LICENSE`
- `LICENSE-MIT`
- `LICENSE-APACHE`
- `NOTICE`

A `.git` directory is also absent, making it impossible to verify commit history and authorship (contributor provenance / DCO / CLA). All proprietary files retain `NEEDS_REVIEW` status pending confirmation by the rights holder.

## 3. Third-Party Dependencies (`Cargo.lock`)

According to `cargo metadata --locked --format-version 1` (see [SBOM_CARGO_LOCK.md](SBOM_CARGO_LOCK.md)):
- Total third-party packages: **246**.
- Packages with unknown or empty license: **0**.
- Packages with mandatory GPL / AGPL: **0**.
- Disjunctive license with LGPL option: **1** (`r-efi 6.0.0` with expression `MIT OR Apache-2.0 OR LGPL-2.1-or-later`). Since the license is disjunctive (`OR`), for compatibility with `MIT OR Apache-2.0`, the `MIT OR Apache-2.0` branch is selected.
- Other permissive licenses among transitive dependencies include `MIT`, `Apache-2.0`, `Unicode-3.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`, `Unlicense`, `CDLA-Permissive-2.0`, `BSL-1.0`. For certain dependencies (e.g., `Apache-2.0`, `BSD-*`, `ISC`, `Unicode-3.0`), a `NOTICE` / third-party notices file should be assembled when publishing source code and especially binary distributions.
- **Actual verification of files in the local Cargo cache:** Out of 246 crates, **238** contain a `LICENSE*`/`LICENCE*`/`COPYING*` file on disk, **1** contains `NOTICE*`, and **8** (`anes@0.1.6`, `r-efi@6.0.0`, `valuable@0.1.1`, `wasmi@0.40.0`, `wasmi_collections@0.40.0`, `wasmi_core@0.40.0`, `wasmi_ir@0.40.0`, `wasmparser@0.221.3`) have an SPDX declaration in `Cargo.toml` without a standalone license file in the package (requiring upstream repository checks prior to binary release).

## 4. Actions Prior to Potential Publication (Without Fabricating Signatures)

1. Add standard `LICENSE-MIT` and `LICENSE-APACHE` after rights holder approval.
2. Assemble `NOTICE` with copyright notices for third-party Rust crates (in particular `Apache-2.0`, `BSD`, `Unicode-3.0`, `Zlib`, `ISC`).
3. Confirm provenance and rights for proprietary files (as `.git` is absent) and resolve the legal status of excluded `mc777` tables and static/core tags.
