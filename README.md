<div align="center">

# RadByte

**A modular Minecraft Java server project, written in Rust.**

[Downloads](https://github.com/RadByte0/RadByte/releases) · [Getting started](#getting-started) · [Documentation](docs/DOCS_INDEX.md) · [Support](#support-the-project)

</div>

RadByte brings together a game server, a proxy, and a lightweight fallback server. It is an actively developed project for people who want to experiment with a Rust-based Minecraft Java server stack.

> **Project status:** RadByte targets Minecraft Java **26.3**, protocol **777**. Compatibility and gameplay are still being developed; do not assume complete vanilla behavior or production readiness. Version data and parts of the existing project have unresolved provenance. Review the [provenance audit](docs/PROVENANCE_CLEAN_ROOM.md) before redistributing or relying on the project.

RadByte is an independent project. It is **not an official Minecraft product** and is not approved by or associated with Mojang or Microsoft.

## Choose a component

| Component | What it does |
| --- | --- |
| **RadServer** | Hosts a game server through a version-specific native library. On Windows it includes a desktop console and resource dashboard. |
| **RadProxy** | Routes Minecraft connections and supports Radons, the project's WebAssembly extensions. |
| **RadLimbo** | Provides a lightweight fallback server. |

## Getting started

1. Open the [latest release](https://github.com/RadByte0/RadByte/releases/latest) and choose **RadServer**, **RadProxy**, or **RadLimbo**.
2. Download the file for your operating system and processor: `x86_64` for most Intel/AMD computers or `aarch64` for ARM computers. Windows files end in `.exe`.
3. Put each component in its own folder before starting it. It creates or reads configuration and runtime files in its working directory.

### RadServer

RadServer needs a **compatible version data package** (`blocks.json`, `items.json`, `entities.json`, `menus.json`; `tags.json` is optional), which is not bundled in the repository or GitHub release. You can set it up in three ways:

1. **Automatic setup (EULA):**
   - **Windows GUI:** Open `rad-server-…exe`, click **`Auto install (EULA)`**, and confirm the [Minecraft EULA](https://aka.ms/MinecraftEULA).
   - **Terminal / Docker (`--nogui`):** Run `rad-server --nogui`. When prompted in an interactive terminal, type `yes` to agree to the Minecraft EULA (or pass `--accept-eula`, set `EULA=TRUE`, or set `eula=true` in `eula.txt`).
   - RadServer downloads the matching `rad-version-26.3-…` native library from GitHub Releases (verified via SHA-256) and the official Minecraft 26.3 `server.jar` from Mojang (verified via SHA-1), runs `--reports` (automatically fetching a temporary portable Adoptium JRE if `java` is not installed), converts the four tables locally, and deletes the temporary files before starting.
2. **`import/` drop-in folder:** Place your locally supplied `blocks.json`, `items.json`, `entities.json`, and `menus.json` into `import/` next to `rad-server` before starting `--nogui`; RadServer will install `versions/26.3/` automatically on startup.
3. **Manual folder picker / CLI:** On Windows GUI, browse to a folder containing the four JSON files and click **`Install version`**, or use `rad-tools package-version` below.

### Pelican / Pterodactyl Eggs

Ready-to-import panel eggs are provided in [`deploy/`](deploy/):
- [`deploy/egg-radbyte.json`](deploy/egg-radbyte.json) — `RadServer` (auto-installs via `EULA=true` and portable Java if needed)
- [`deploy/egg-radproxy.json`](deploy/egg-radproxy.json) — `RadProxy`
- [`deploy/egg-radlimbo.json`](deploy/egg-radlimbo.json) — `RadLimbo`

### Prepare a version package manually

From the server's working directory, with a source checkout and Rust installed, run:

```text
cargo run --manifest-path <checkout>/Cargo.toml --locked -p rad-tools -- package-version <data-directory> <library-file> <versions-directory>
```

For example, set `<versions-directory>` to `./versions` while working in the server folder. The tool validates the data and library, then creates `versions/version.toml` and `versions/26.3/` with the package manifest. It refuses to overwrite an existing `26.3` package. The library must match the operating system and architecture on which the package is prepared.

RadProxy does not need the version library. RadLimbo can use a compatible local data package but does not load the version library.

## Build from source

Install a recent stable Rust toolchain and a native compiler for your platform, then run:

```text
cargo build --workspace --release --locked
```

Cargo places the binaries and native version library in `target/release/`. Building does **not** include the externally supplied version data or create a complete server installation. Run each service from its own working directory with its configuration and required version package in place. The repository uses Cargo for builds and tests; it does not include shell-based build or runtime scripts.

Useful checks:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

The repository excludes some data of unconfirmed provenance, and some local or integration tests need additional data. A successful build does not establish legal clearance or complete client compatibility.

## Configuration and documentation

Configuration templates live in [`config/templates/`](config/templates/). Each service uses its own `rad.properties`; RadServer has additional settings in `config/radserver-extra.properties`. See the [documentation index](docs/DOCS_INDEX.md) for architecture, configuration, and development notes. Back up worlds and configuration before trying a new build.

## Support the project

If RadByte is useful to you, you can support AnarchByte through [Ko-fi](https://ko-fi.com/anarchbyte), [Buy Me a Coffee](https://buymeacoffee.com/anarchbyte), or [GitHub Sponsors](https://github.com/sponsors/AnarchByte0).

## License and provenance

The workspace declares **MIT OR Apache-2.0**; see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). Third-party dependencies have their own licenses, and the declarations do not resolve the outstanding provenance questions for project code or version data. See [the provenance audit](docs/PROVENANCE_CLEAN_ROOM.md) and [dependency inventory](docs/legal/DEPENDENCY_LICENSES.md).
