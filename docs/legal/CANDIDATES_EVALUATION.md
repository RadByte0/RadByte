# Evaluation of Open-Source Rust Candidates and Protocol Crates (Stages 4 & 5)

> Historical material: status and paths describe a previous stage. Generated inventories and progress reports were removed during cleanup; for current provenance evidence, see [PROVENANCE_CLEAN_ROOM.md](../PROVENANCE_CLEAN_ROOM.md).

The evaluation was performed for the RadByte workspace with declared license `MIT OR Apache-2.0` and target version **Minecraft Java 26.3 (protocol 777)**.

## 1. Candidate Evaluation Table

| Candidate | URL | Author / Rights Holder | Code License | Data License / `NOTICE` | Transitive Risks | MC 26.3 Support | Decision |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Pumpkin** (`Pumpkin-MC/Pumpkin`) | `https://github.com/Pumpkin-MC/Pumpkin` | Pumpkin-MC contributors | `GPL-3.0` (server), `MIT OR Apache-2.0` (`pumpkin-plugin-api`, `pumpkin-plugin-wit`) | Has `assets/NOTICE.md`: Minecraft data (`assets/blocks.json`, `datapacks/`) is property of Mojang/Microsoft under Minecraft EULA; Geyser/Bedrock (`MIT`); ViaVersion (`GPLv3 / MIT`) | High (GPLv3 incompatible with `MIT OR Apache-2.0` for copying server code; embedded Mojang data) | Targets modern Java versions via custom Extractor; not a ready drop-in for 26.3 without GPLv3 | **REJECTED** (for copying/integrating code; `assets/NOTICE.md` model is useful only as a structural documentation example) |
| **SteelMC** (`Steel-Foundation/SteelMC`) | `https://github.com/Steel-Foundation/SteelMC` | Steel-Foundation | `AGPL-3.0-or-later` | Contains server data/constants | Critical (AGPL-3.0-or-later incompatible with `MIT OR Apache-2.0`) | Unconfirmed for 26.3 | **REJECTED** |
| **FerrumC** (`ferrumc-rs/ferrumc`) | `https://github.com/ferrumc-rs/ferrumc` | ferrumc-rs contributors | `MIT` (some forks/branches, notably Temper, moved to `GPL-3.0`) | Requires separate audit of embedded data in repository | Medium (requires verifying each specific commit and data provenance) | Targets 1.21+, not 26.3 (proto 777) | **NEEDS_LEGAL_REVIEW** (and technically unsuitable for 26.3) |
| **Valence** (`valence-rs/valence`) | `https://github.com/valence-rs/valence` | valence-rs contributors | `MIT` (logo `CC BY-NC-ND 4.0`) | Registry/protocol data tied to specific versions | Medium (tight coupling to ECS/Bevy framework) | Tracks latest stable on `main`, not 26.3 (proto 777) | **REJECTED** (architecturally and by protocol version) |
| **fastnbt** (`owengage/fastnbt`) | `https://github.com/owengage/fastnbt` | Owen Gage | `MIT OR Apache-2.0` | Clean NBT parser/serializer without Mojang data | Minimal | Independent of MC version (NBT format) | **APPROVED_FOR_TECHNICAL_TRIAL** |
| **simdnbt** (`azalea-rs/simdnbt`) | `https://github.com/azalea-rs/simdnbt` | azalea-rs contributors | `MIT` | Clean fast NBT parser without Mojang data | Minimal | Independent of MC version | **APPROVED_FOR_TECHNICAL_TRIAL** |
| **crab_nbt** (`CrabCraftDev/CrabNBT`) | `https://github.com/CrabCraftDev/CrabNBT` | CrabCraftDev | `GPL-3.0` | Clean NBT | High (`GPL-3.0` copyleft) | Independent of MC version | **REJECTED** |
| **azalea-protocol** (`azalea-rs/azalea`) | `https://github.com/azalea-rs/azalea` | azalea-rs contributors | `MIT` (individual workspace subcrates may have different/GPL terms) | Contains protocol definitions | Medium | Client stack targeting current versions | **NEEDS_LEGAL_REVIEW** |

## 2. Decision Regarding Stage 5 (Pilot Integration)

The RadByte workspace (`RadCore`) already implements its own protocol, NBT, and data modules for MC 26.3 (protocol 777). None of the server frameworks (Pumpkin, SteelMC, FerrumC, Valence) simultaneously combines a compatible license (`MIT OR Apache-2.0`), clean verified data, and support for Minecraft 26.3.

In accordance with the plan rule (*"If there is no suitable candidate, do not copy anything merely to demonstrate progress — justify this in the report"*), redundant integration of third-party crates (e.g. replacing working NBT with `fastnbt`/`simdnbt`) was **not performed** at this stage to avoid introducing unnecessary changes without practical benefit.
