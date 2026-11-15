# Blocks & Items Coverage Report (Minecraft 26.3 / Protocol 777)

- **Blocks in registry (`RadCore/data/mc777/blocks.json`)**: `1286` (`35723` states)
- **Items in registry (`RadCore/data/mc777/items.json`)**: `1658`
- **Real GUI Client 26.3 Verified**: `false` (verified via automated unit, wire, and multi-player TCP tests; not yet launched against GUI Minecraft 26.3)

## Block Families

| Family | Blocks | States | Has Item | Placement | Breaking | Loot | Interaction | Physics | Persistence |
|---|---:|---:|---:|---|---|---|---|---|---|
| `air_and_fluids` | 5 | 35 | 1 | partial (buckets/water sim) | n/a | none | water flow & item buoyancy | True | True |
| `full_blocks_and_stone` | 337 | 337 | 318 | default_state | survival/creative | basic (stone->cobble, grass->dirt, coal/diamond) | solid collision | True | True |
| `plants_crops_saplings` | 106 | 1059 | 70 | default_state | survival/creative | self | wheat crop growth sim | non-solid | True |
| `other_stateful_blocks` | 343 | 7383 | 291 | default_state | survival/creative | self if item exists | none yet | solid | True |
| `containers_and_workstations` | 41 | 442 | 41 | default_state | survival/creative | self | chest, crafting_table, furnace UI implemented | True | True |
| `beds_banners_signs` | 101 | 2048 | 59 | default_state | survival/creative | self | none yet | partial | True |
| `redstone_and_rails` | 52 | 2005 | 48 | default_state | survival/creative | self | lever, redstone wire/lamp, tnt implemented | partial | True |
| `stairs` | 97 | 7760 | 97 | default_state (needs facing/half/shape) | survival/creative | self | stair collision | True | True |
| `slabs` | 101 | 606 | 101 | default_state (needs top/bottom/double) | survival/creative | self | half-slab collision | True | True |
| `doors` | 22 | 1408 | 22 | default_state (needs 2-high/hinge) | survival/creative | self | none yet | partial | True |
| `fences_walls_gates` | 59 | 11232 | 59 | default_state | survival/creative | self | none yet | solid | True |
| `trapdoors` | 22 | 1408 | 22 | default_state | survival/creative | self | none yet | partial | True |

## Item Families

| Family | Count | Notes |
|---|---:|---|
| `air` | 1 | Empty slot sentinel |
| `block_items` | 1128 | Mapped via `mc777_data::blocks::by_name` (`item_id_to_block_state`); drops & pickup implemented |
| `materials_and_misc` | 329 | Drops, pickup, stack merge, chest/furnace/crafting persistence supported |
| `armor` | 35 | Equip slots in inventory; drops & components preserved; armor defense calculation pending |
| `usable_items_and_projectiles` | 114 | Bow, arrow, snowball implemented; buckets/potions/ender_pearl partial or pending |
| `food` | 16 | Nutrition implemented for core foods (`apple`, `bread`, `cooked_porkchop`, etc.) in `food_nutrition` |
| `tools_and_weapons` | 35 | Attack damage & basic tool tiering in `inventory.rs`; drops & components (`damage`) preserved |
