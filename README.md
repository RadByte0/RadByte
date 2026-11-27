# RadByte ⚡

[![Latest Release](https://img.shields.io/github/v/release/RadByte0/RadByte?style=for-the-badge&color=E87D0D)](https://github.com/RadByte0/RadByte/releases/latest)
[![Minecraft](https://img.shields.io/badge/Minecraft-26.3%20(777)-success?style=for-the-badge)](#components-)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey?style=for-the-badge)](#quick-start-)
[![Ko-Fi](https://img.shields.io/badge/Ko--fi-F16061?style=for-the-badge&logo=ko-fi&logoColor=white)](https://ko-fi.com/anarchbyte)
[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20A%20Coffee-FFDD00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://buymeacoffee.com/anarchbyte)

**Modular Minecraft Java 26.3 (protocol 777) server, proxy, and limbo stack — written in Rust.**

RadByte gives you a single-binary launcher experience for hosting a game server (**RadServer**), an L7 reverse proxy with WebAssembly plugins (**RadProxy**), and an ultra-lightweight waiting room (**RadLimbo**) — complete with an interactive terminal UI, live CPU/RAM metrics, automatic setup, and ready-to-import Pelican/Pterodactyl eggs.

> [!NOTE]
> **Project status:** RadByte targets Minecraft Java **26.3** (protocol **777**). Compatibility and gameplay mechanics are still in active development. Version data and some existing code have unresolved provenance — see the [provenance audit](docs/PROVENANCE_CLEAN_ROOM.md) before redistributing. RadByte is **not an official Minecraft product** and is not affiliated with Mojang or Microsoft.

---

## Components 🧩

| Component | What it does | Needs Java / MC Data? |
| :--- | :--- | :--- |
| **RadServer** | Hosts the Minecraft 26.3 game world via a dynamically loaded native version module (`rad-version-26.3`) | Auto-downloads on first run |
| **RadProxy** | High-performance L7 Minecraft proxy with hostname routing, forwarding (`modern` / `bungeeguard` / `legacy`), and WASM **Radons** | No |
| **RadLimbo** | Ultra-lightweight fallback / waiting-room server | No |
| **RadAPI** | Versioned plugin & WASM guest SDK (`apis/v1/`), auto-installed **only** when `radons/` has plugins | No |

---

## Quick Start 📦

No manual compilation required — grab the latest binaries from the **[Releases page](https://github.com/RadByte0/RadByte/releases/latest)**.

### Interactive Terminal (Windows, Linux, macOS, SSH)
1. Download **`rad-server`**, **`rad-proxy`**, or **`rad-limbo`** for your OS and architecture (`x86_64` for Intel/AMD, `aarch64` for ARM64, or `linux-musl` for Alpine/Docker).
2. Place each executable in its own folder and run it.
3. On first launch, an **interactive arrow-key setup wizard** opens right in your terminal:
   - **RadServer:** Pick a release from the dropdown (`/` to search), toggle older releases, toggle **Auto-update**, accept the [Minecraft EULA](https://aka.ms/MinecraftEULA) (or pick a local `import/` folder), and hit **Install & Start Server**.
     - RadServer automatically fetches a portable Adoptium JRE if `java` isn't installed, runs Mojang's `--reports` generator, verifies the native `rad-version` library via SHA-256, and cleans up all temporary files.
   - **RadProxy / RadLimbo:** Configure Listen IP, Port, Backend Address, Forwarding Mode, and Auto-update, then hit **Save & Start**.

> [!TIP]
> **Console Controls:**
> - Type commands directly in the bottom `> ` prompt (`help`, `op`, `say`, `reload`, `stop`).
> - Scroll logs inside the terminal with your **Mouse Wheel**, **`PgUp` / `PgDn`**, or **`↑` / `↓`**.
> - Re-open the first-run configuration anytime with `--setup`.

### Pelican / Pterodactyl / Docker (`--nogui`)
Ready-to-import panel eggs are included in [`deploy/`](deploy/):
- [`deploy/egg-radbyte.json`](deploy/egg-radbyte.json) — **RadServer** (auto-installs via `EULA=true` and portable Java if missing)
- [`deploy/egg-radproxy.json`](deploy/egg-radproxy.json) — **RadProxy**
- [`deploy/egg-radlimbo.json`](deploy/egg-radlimbo.json) — **RadLimbo**

When running inside Pelican, Pterodactyl, Docker, or with `--nogui`, the interactive TUI is automatically disabled — you get a clean startup ASCII banner, compact `[HH:MM:SS] INFO ...` logs, and full environment variable support (`EULA=TRUE`, `SERVER_IP`, `SERVER_PORT`, `BACKEND_ADDR`, `FORWARDING_MODE`, `RADBYTE_AUTO_UPDATE=true|false`).

---

## Key Features ✨

* **Antigravity-Style Console UI:** Clean `RadByte` header with live **per-thread CPU%** (`100%` per core) and **RAM in Mbit**, internal log scrolling, and a pinned command input bar.
* **Built-In Auto-Updater:** Automatically checks GitHub Releases on startup and updates the launcher, `versions/` library, and `apis/` runtime when enabled (`auto-update=true` in `rad.properties`, or disable via `--no-auto-update` / `RADBYTE_AUTO_UPDATE=false`).
* **On-Demand `RadAPI`:** `RadAPI` is packaged separately per OS (`rad-api-v1-*`) and is only downloaded into `apis/v1/` when you actually place `.wasm` or `.toml` plugins in `radons/`.
* **Vanilla-Clean Server Files:** Stores player permissions and bans in standard `ops.json`, `whitelist.json`, and `banned-players.json` — no redundant clutter files.
* **Cross-Platform Ready:** Pre-built for **Windows** (`x64`, `ARM64`), **Linux** (`x64`, `ARM64`, plus `musl` for Alpine/Docker), and **macOS** (`Intel`, `Apple Silicon`).

---

## Building from Source 🛠️

### Prerequisites
Install a recent stable [Rust](https://rustup.rs/) toolchain (Edition 2024) and a native C/C++ linker for your platform.

### Compile & Test
```bash
cargo build --workspace --release --locked
```

Useful checks:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

<details>
<summary><b>Architecture Overview</b> (for contributors)</summary>

* **`RadCore/`** — Zero-copy Minecraft protocol codecs, packet definitions, CLI/TUI (`console_ui`, `setup_ui`), and auto-update primitives.
* **`RadServer/`** — Host launcher, Mojang `--reports` data installer, portable JRE fetcher, and dynamic `rad-version` loader.
* **`RadVersions/v26_3/`** — Minecraft 26.3 (protocol 777) world, entity, redstone, command, and networking implementation compiled as a native shared library.
* **`RadProxy/`** — High-performance L7 Minecraft proxy with HAProxy, velocity/legacy forwarding, and `wasmi` Radon sandbox.
* **`RadLimbo/`** — Lightweight waiting-room server.
* **`RadAPI/`** — Versioned plugin & WASM guest SDK (`rlib` + `cdylib`).
* **`RadTools/`** — Release staging, release notes generator, and version package builder.

</details>

---

## Documentation & Support 📚

- Browse [`docs/DOCS_INDEX.md`](docs/DOCS_INDEX.md) for architecture, configuration templates ([`config/templates/`](config/templates/)), and development notes.
- If RadByte helps your server or project, consider supporting AnarchByte via [Ko-fi](https://ko-fi.com/anarchbyte), [Buy Me a Coffee](https://buymeacoffee.com/anarchbyte), or [GitHub Sponsors](https://github.com/sponsors/AnarchByte0).

---

## License 📄

Licensed under **MIT OR Apache-2.0**; see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE). Third-party dependencies retain their own licenses; see [the provenance audit](docs/PROVENANCE_CLEAN_ROOM.md) and [dependency inventory](docs/legal/DEPENDENCY_LICENSES.md).
