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

RadServer also needs a **compatible version data package**, which is not included in the release. You must supply `blocks.json`, `items.json`, `entities.json`, and `menus.json` yourself; `tags.json` is optional. The data must match the supported version and its provenance and rights should be appropriate for your use. The executable and library alone are not a complete installation.

On **Windows**, open `rad-server-…exe`. If no usable package is installed, the version setup screen lets you choose 26.3 and a local data folder. It downloads the matching version library from the same release and verifies its SHA-256 against GitHub's release metadata. The dashboard opens after setup.

For terminal mode on Windows, pass `--nogui`:

```text
rad-server-x86_64-pc-windows-msvc.exe --nogui
```

On **Linux and macOS**, RadServer runs in the terminal. Download the matching `rad-version-26.3-…` library as well; Linux uses `.so` and macOS uses `.dylib`. Make the executable runnable with `chmod +x <server-file>`. Prepare the package with the Rust tool below, then start the server from the same working directory.

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
