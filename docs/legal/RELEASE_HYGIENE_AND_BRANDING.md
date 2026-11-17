# Future Release Hygiene and Branding Review (Stage 6)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

## 1. Minecraft / Mojang Branding

- The required disclaimer is present in [README.md](../../README.md):
  > **NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**
- Crate and product names (`RadByte`, `RadServer`, `RadProxy`, `RadLimbo`, `RadCore`, `RadAPI`, `RadVersions`) do not imitate official Mojang/Microsoft trademarks.
- Recommendation prior to publication: retain this disclaimer in the future GitHub release description and documentation as well, and do not use Minecraft logos, textures, sounds, or graphic assets.

## 2. Secrets, Tokens, Configurations, and Worlds

- In the source code of `RadLimbo` and `RadProxy`, only test/template values for `forwarding_secret` are used (`"change-me-radbyte-secret"`, `"limbo-test-secret"`). No real production secrets were found in the code.
- In [`cargo build --workspace --release --locked`](../../`cargo build --workspace --release --locked`), when generating new configs in `dist/`, a random local 16-byte `forwarding_secret` is generated — therefore, after a local run, the `dist/` directory **must not** be published as a ready archive without cleanup.
- All `.jar` files in the workspace are located exclusively in `Paper-26.3/` (which is excluded in `.gitignore` and in staging).

## 3. `.gitignore` Verification

The existing [.gitignore](../../.gitignore) already excludes `/target/`, `/dist/`, `/Paper-26.3/`, `/artifacts/`, `/versions/`, `/world/`, `*.mca`, `/radons/*.wasm`, `*.log`, `*.tmp`.

**Recommended additions prior to the first commit to a public Git repository** (changes to `.gitignore` have not been made automatically at this time):
- `*.jar`
- Local root configs/state files unless they are templates (`radproxy.toml`, `radlimbo.toml`, `rad.properties`, `server.properties`, `eula.txt`, `*.pid`)

## 4. Checklist for Future Clean Release Archive

1. Add `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE` (after legal verification).
2. Do not include local `world/`, `logs/`, `Paper-26.3/`, or `radproxy.toml` with a local `forwarding_secret`.
3. Add only clean binaries (`rad-server`, `rad-proxy`, `rad-limbo`, `rad_version_26_3.dll`), template configs with a changeable placeholder secret, `LICENSE-*`, `NOTICE`, and `README` to the release archive.
