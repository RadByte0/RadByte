# Coverage Report: Redstone Tab (Minecraft Java 26.3 / protocol 777)

| Group / Element | Registry / Inventory | Placement & Support | Signal / Mechanics | Tests | Status |
|---|---|---|---|---|---|
| Redstone dust (`redstone_wire`) | Yes | Yes (`supports_redstone`, disallowed without support, dot/cross) | Yes (`0..=15`, multi-source, loops, step-up/down) | 6 unit tests in `sim.rs` | Supported (GUI partially verified) |
| Lever (`lever`) | Yes | Partial (floor/wall without wall detachment check) | Yes (`toggle_lever_in`, `0`/`15`, lamps) | `redstone_lever_propagates_power_and_tnt_carves_crater` | Partial |
| Redstone lamp (`redstone_lamp`) | Yes | Yes | Yes (`lit=true`/`false` from neighbors and dust) | `redstone_lever_propagates_power_and_tnt_carves_crater` | Supported |
| Block of redstone (`redstone_block`) | Yes | Yes | Yes (constant power source 15) | Separate test needed | Partial |
| Redstone torch / wall torch | Yes | Yes (`FloorSupportCheck`, `WallSupportCheck`) | Partial (support present; inversion/burnout in progress) | `wall_torch_drops_when_supporting_wall_is_broken` | Partial |
| Buttons (`stone_button`, `oak_button`, etc.) | Yes | Partial | Yes (`press_button_in`, `ButtonRelease` 20/30 ticks) | `button_press_powers_redstone_and_releases_after_delay` | Supported |
| Pressure plates | Yes | Yes (`FloorSupportCheck`) | No (no entity/weight detection) | — | Partial |
| Repeater (`repeater`) | Yes | Yes (`orient_block_on_place`, `FloorSupportCheck`) | Yes (`RepeaterTick` 2/4/6/8 ticks, click for delay 1–4; no side locking) | `repeater_delays_signal_and_cycles_delay` | Partial |
| Comparator (`comparator`) | Yes | Yes (`orient_block_on_place`, `FloorSupportCheck`) | No | — | None |
| Pistons / Sticky pistons | Yes | Yes | Yes (`update_piston_at_in`, up to 12 blocks, `piston_head`, sticky pull; no `moving_piston`/slime) | `piston_push_and_sticky_piston_pull` | Partial |
| Copper bulb / Target / Observer / Daylight | Yes | No | No | — | None |
| Sculk sensor / Shrieker / Lightning rod | Yes | No | No | — | None |
| Hopper / Dispenser / Dropper / Crafter | Yes | No | No | — | None |
| Rails / Minecarts / Chest boats | Yes | Rail support present | No | — | Partial |
| Doors / Trapdoors / Fence gates | Yes | Yes | Yes (`toggle_openable_in`, both door halves) | `door_trapdoor_and_fence_gate_toggle_open` | Partial |
| TNT | Yes | Yes | Yes (`detonate_tnt_in`, explosion) | `redstone_lever_propagates_power_and_tnt_carves_crater` | Partial |
