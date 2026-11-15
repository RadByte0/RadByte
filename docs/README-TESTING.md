# RadByte Local Testing Guide

From the repository root, run `./scripts/Build.ps1` (or `./scripts/Build.ps1 -DataPackage <path>` to stage a compatible version data package into `dist/RadServer/versions/26.3/`). After a successful release build,
`dist/RadServer`, `dist/RadProxy`, and `dist/RadLimbo` contain the executables
and runtime configuration. Stop running services before rebuilding. Existing
configuration, plugins, access lists, and worlds are preserved.

Run `./scripts/Start-RadByte.ps1` from the repository root for the direct server,
or add `-Profile Network` to start all three services. The commands below
are run from `dist/`. Rust's `cargo build` alone does not package `dist`.

Note: `RadServer` requires a valid `versions/26.3/` data package (provided explicitly via `-DataPackage <path>` during `Build.ps1` / `package.ps1`) and refuses to open the game port if the package is missing, corrupted, or incompatible.

## Profiles

1. **Direct (`RadServer` only)** — quickest way to test local survival gameplay on `127.0.0.1:25567`:
   ```powershell
   .\scripts\Start-RadByte.ps1 -Profile Direct
   ```
2. **Network (`RadLimbo` + `RadServer` + `RadProxy`)** — tests proxy routing, modern forwarding, and fallback (`127.0.0.1:25565`):
   ```powershell
   .\scripts\Start-RadByte.ps1 -Profile Network
   ```
   > Note: When switching `RadServer` behind `RadProxy`, set `forwarding-mode=modern` in `RadServer\config\radserver-extra.properties`. Its `forwarding-secret` must match `forwarding_secret` in both `RadProxy\radproxy.toml` and `RadLimbo\radlimbo.toml`.

## Lifecycle, Backup & Smoke Verification

- **Stop gracefully:**
  ```powershell
  .\scripts\Stop-RadByte.ps1
  ```
  Writes `.radserver.stop` for `RadServer` so `save_all()` runs before exit, and stops only verified tracked PIDs.
- **Backup & verify restore:**
  ```powershell
  .\scripts\Backup-RadByte.ps1 -RestoreTo .\backups\verify-restore
  ```
- **Run packaged smoke & generate run artifacts (`target/test-runs/<run-id>/`):**
  ```powershell
  .\scripts\Test-RadByte.ps1
  ```

## Known Scope Notes

- World `.mca` region files store RadByte's custom `RAD1` zlib chunk payload (see `docs/STORAGE_FORMAT.md`), not vanilla Anvil NBT.
- Live Mojang/Java client GUI connection and authentication are not automated in `Test-RadByte.ps1` and should be tested manually if required.
