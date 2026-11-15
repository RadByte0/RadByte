# Future RadClient Architecture and Module Boundaries

This document describes the boundaries for the future client application `RadClient`. **The graphical client itself is not yet implemented**; at the current stage, shared protocol and version foundations have been prepared.

## Currently Prepared Layers

1. **`rad-core::client`** (`RadCore/src/client.rs`):
   - Headless network client (`HeadlessClient`, `HeadlessPlaySession`, `ClientConfig`, `ClientError`).
   - Supports `Handshake`, `Status`/`Ping`, `Login` (with `SetCompression` and `LoginDisconnect`), `Configuration` (`KnownPacks`, `FinishConfiguration`), and `Play` (`PlayLogin`, `KeepAlive`, `SynchronizePlayerPosition` → `ConfirmTeleportation`).
   - Has frame size limits, timeouts, and does not store or log passwords or tokens.
2. **`rad-version`** (`RadVersions/common/src/lib.rs`, `shapes.rs`, `physics.rs`):
   - `VersionDescriptor`, `VersionCapabilities`, `MC_26_3` (`game_version = "26.3"`, `protocol_version = 777`, `world_data_version = 4790`, `storage_schema_version = 1`, `abi_version = 1`).
   - `WorldMechanicsProfile` and `check_world_compatibility` for explicit world compatibility verification.

## Boundaries of Future `RadClient` Subsystems (Not a Server Dependency)

- **Chunk state & meshing**: client-side chunk section cache, without access to server Anvil storage or `WorldSim`.
- **Rendering / GUI / Input**: renderer, window, camera, entity interpolation, and menus are not imported into `RadCore`, `RadVersions/common`, or `RadServer`.
- **Client-side prediction**: uses the same deterministic `VoxelShape` contracts, but does not pull in the server filesystem or `RadAPI`.
- **Security and resources**:
  - The client **never** loads native DLL/SO/DYLIB files from an untrusted server.
  - The project does not include or distribute Minecraft JARs, textures, or sounds without verified rights.
  - The official Minecraft Java 26.3 client remains a mandatory independent testing baseline.
