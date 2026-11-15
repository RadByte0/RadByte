# Block Hitbox & Interaction Coverage Report (Minecraft Java 26.3 / protocol 777)

## Data Source and Provenance

- Local snapshot: `RadCore/data/mc777/blocks.json` (`kind: extracted`, `mapping_source: vineflower-direct`).
- According to `docs/PROVENANCE_CLEAN_ROOM.md`, this data is **not** marked as clean-room.
- The general fallback `VoxelShape::Full` is **not** counted toward `verified` (classified as `unknown_full_fallback`).

## Summary Metrics

| Metric | Numerator / Denominator | Percentage |
|---|---:|---:|
| Collision shapes (`verified` states) | 28586 / 35723 | 80.02% |
| Collision shapes (`approximate` states) | 1004 / 35723 | 2.81% |
| Collision shapes (`unknown_full_fallback` states) | 6133 / 35723 | 17.17% |
| Interaction/outline shapes (`verified` states) | 28586 / 35723 | 80.02% |
| Block types (`verified` types) | 639 / 1286 | 49.69% |
| Verification with real 26.3 GUI client | 0 / 35723 | not verified (headless/unit verified) |

## Priority Families (Stages 3–5)

| Family | Collision Status | Interaction Status | Notes |
|---|---|---|---|
| Slabs (`*_slab`: top/bottom/double) | `verified` | `verified` | Exact `0.5`/`1.0` shapes in `shapes.rs` |
| Stairs (`*_stairs`: 80 states/type) | `verified` | `verified` | 40-variant lookup table (`facing` x `half` x `shape`) |
| Fences (`*_fence`: 32 states/type) | `verified` | `verified` | Center + 4 arms, `y=0..1.5` (`y0-1` accounted for in AABB) |
| Walls (`*_wall`: 324 states/type) | `verified` | `verified` | `up` + 4 arms, `y=0..1.5` |
| Fence gates (`*_fence_gate`: 32 states/type) | `verified` | `verified` | Empty collision when `open=true`, `1.5m` when `open=false` |
| Doors (`*_door`: 64 states/type) | `verified` | `verified` | `3/16` based on `facing`, `open`, `hinge` + auto-top/breaking |
| Trapdoors (`*_trapdoor`: 64 states/type) | `verified` | `verified` | `3/16` bottom/top/side based on `open`, `half`, `facing` |
| Glass panes & Iron bars | `verified` | `verified` | Center `2/16` + 4 arms |
| Carpets (`*_carpet`) | `verified` | `verified` | `1/16` (`0.0625`) |
| Snow layers (`minecraft:snow`) | `verified` | `verified` | Layer 1 empty collision (`2/16` interaction), layers 2..8 `(layers-1)*2/16` |
| Repeater / Comparator | `verified` | `verified` | `2/16` (`0.125`) + click interaction |
| Hopper / Cauldron | `verified` | `verified` | 5-box shape (can stand inside) |
| Chests / Lectern / Bell / Bed / Anvil | `approximate` | `approximate` | Main height and dimensions accounted for, some details simplified |
| Non-solid (fluids, plants, cobweb, rails, torches, lever, buttons, plates, moving_piston) | `verified` | `verified` | Empty collision, separate `block_interaction_shape` |
