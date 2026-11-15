# RadServer Storage Format Specification

This document describes the on-disk world storage format used by `RadServer`.

> **Important:** `RadServer` stores chunks inside `.mca` region files using its own compact **`RAD1`** zlib-compressed payload, **not** Vanilla/Paper Anvil NBT chunk format. Foreign or corrupt `.mca` chunk entries are detected on read and protected from being overwritten by flat-world fallback data.

## 1. World Directory Layout

```text
world/
  region/
    r.<rx>.<rz>.mca      # Region files (32x32 chunks each, 8 KiB Anvil sector header + 4 KiB sectors)
  playerdata/
    <uuid>.json          # Per-player serialized state (atomic write via .tmp + rename)
  containers.json        # Persisted 27-slot Chest block entities (atomic write via .tmp + rename)
```

## 2. Region Files (`region/r.<rx>.<rz>.mca`)

- **Header (`8192` bytes):**
  - Bytes `0..4096`: 1024 big-endian 4-byte location entries `(offset_sectors_u24, sector_count_u8)`.
  - Bytes `4096..8192`: 1024 timestamp entries (4 bytes each).
- **Chunk Sector Payload:**
  - `length: u32` (big-endian, includes compression byte)
  - `compression: u8` (`2` = zlib)
  - `zlib_data`: bounded decompression (`<= 1 MiB` uncompressed).
- **Uncompressed `RAD1` Chunk Payload:**
  - Magic header: `b"RAD1"` (`4` bytes)
  - `24` Overworld sections (`Y = -64..320`), each containing `4096` block state IDs (`u16` little-endian, indexed `y * 256 + z * 16 + x`).
- **Safety Guarantees:**
  - Payloads exceeding `255` sectors (`1020 KiB`) are rejected.
  - Existing in-place sectors are reused when large enough.
  - If a chunk fails header/sector bounds, decompression limit, or `RAD1` magic/length validation, it is marked **protected** in `WorldState` and never overwritten on `set_block` or `save_all`.

## 3. Player Data (`world/playerdata/<uuid>.json`)

JSON object (`SavedPlayerData`) written atomically via `<uuid>.json.tmp` + `sync_all()` + `rename`:

- `x`, `y`, `z` (`f64`): world coordinates
- `yaw`, `pitch` (`f32`): head rotation
- `hp` (`f32`), `food` (`i32`), `saturation` (`f32`): survival vitals
- `game_mode` (`u8`): `0` = Survival, `1` = Creative, `2` = Adventure, `3` = Spectator
- `selected_hotbar` (`u8`): `0..=8`
- `inventory` (`Vec<SavedSlot>`): all `46` slots (`{ count: i32, item_id: i32 }`)

## 4. Containers (`world/containers.json`)

JSON array of Chest block entities written atomically via `containers.json.tmp` + `sync_all()` + `rename`:

- `x`, `y`, `z` (`i32`): block coordinates of the Chest (`block_state = 190`)
- `slots` (`Vec<{ count, item_id }>`): `27` chest slots
