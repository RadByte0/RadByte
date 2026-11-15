# RadByte Dependency Audit (Stage 1 of the RadVersion Plan)

> [!NOTE]
> Historical document. Following the migration, shared code is located in `RadVersions/common/`, and the Minecraft 26.3 implementation is in `RadVersions/v26_3/` (see [VERSION_ARCHITECTURE.md](VERSION_ARCHITECTURE.md)).

## Crates and Dependency Map

```text
RadServer (launcher) ──▶ RadCore (ABI structs, VersionPackage, CLI)
                     └── libloading ──▶ versions/26.3/rad_version_26_3.dll (C ABI v1)

RadVersion26_3       ──▶ RadCore (protocol, codec, packet, nbt, mc777_data, security, version_package)
                     ──▶ RadAPI (v1 WASM/addon ABI)
                     ──▶ RadVersion (version descriptor MC_26_3, capabilities, world profile)

RadProxy             ──▶ RadCore (handshake, status, forwarding, codec)
RadLimbo             ──▶ RadCore (protocol, status, login/play limbo)
```

## Version Constants and Boundaries

| Component | Actual Value (26.3) | Purpose |
|---|---|---|
| `game_version` | `"26.3"` | Minecraft Java game version |
| `protocol_version` | `777` | Network protocol version (`rad_core::PROTOCOL_VERSION`) |
| `world_data_version` | `4790` | Anvil `DataVersion` in `level.dat` / chunks |
| `storage_schema_version` | `1` | Schema of additional world files (`radbyte_world_profile.json`, `sim_queue.json`, etc.) |
| `RAD_VERSION_ABI_VERSION` | `1` | C ABI v1 between `RadServer` and `rad_version_26_3.dll` |
| `SERVER_CONFIG_VERSION` | `1` | `rad.properties` configuration schema |
| `RadAPI` feature | `v1` | Stable WASM addon interface (`radons/`) |

## Separation: Shared vs 26.3-Specific

- **Shared (server + future client)**: `RadCore` (`types`, `codec`, `nbt`, `state`, `crypto`, `forwarding`, `client` headless primitives, `version_package`), `RadVersion` (`VersionDescriptor`, `VersionCapabilities`, `WorldMechanicsProfile`, world profile compatibility checking).
- **Specific to Minecraft 26.3 / protocol 777**: `RadCore::mc777_data` (777 registries/tables), `RadCore::packet` (777 packet IDs and layout), `RadVersion::MC_26_3`, `RadVersion26_3` (C ABI v1 CDYLIB, server simulation, world, physics, inventory, network loop).
