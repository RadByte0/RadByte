# RadVersion Plan and Foundations for Future RadByte Client

> [!NOTE]
> Historical plan. The current structure of `RadVersions/common/` and `RadVersions/v26_3/` is described in [VERSION_ARCHITECTURE.md](VERSION_ARCHITECTURE.md).

## 1. Goal and Scope

Extract shared mechanics from `RadVersion26_3` into clear modules, create
a single `RadVersion` directory/crate for descriptions of supported versions, and prepare
the protocol and data for reuse by a future client. Current work
focuses on **26.3 / protocol 777**. The future `RadClient` is a separate milestone,
which is not considered implemented upon server refactoring.

A single running server has one active game and mechanics version. The project does not
embed ViaVersion and does not translate different client versions automatically.
An unsupported protocol is rejected with a clear message. Directories or
manifests of other versions are not considered proof of their support.

"Client" here refers to a future standalone RadByte client application,
as well as a shared network library suitable for implementing a test or
full client. Support for the official Minecraft 26.3 client remains
mandatory; the new program must not replace it as the sole test.

## 2. Actual Starting Point

- The workspace contains `RadCore`, `RadAPI`, `RadServer`, `RadProxy`, `RadLimbo`, and
  `RadVersion26_3`. `RadCore` already has codecs, NBT, packets, and `GameVersion`.
- `RadServer` currently verifies the package and loads a native version DLL. In
  `RadVersion26_3`, config, net, gameplay, world, and radon are located, not
  just version differences. Its C ABI v1 is used by the launcher.
- The 777 data/generators and their provenance issues are described in
  `docs/PROVENANCE_CLEAN_ROOM.md`. The use of shared data does not remove
  the obligation to verify rights to code, assets, and the public distribution.
- The current `versions/26.3/` package and existing worlds/configs may already
  be in use. Do not assume they are interchangeable after renaming.

## 3. Target Logical Structure

The names of new crates below are an architectural reference; create them only
when boundaries and dependencies are confirmed by code, without cycles or duplication.

```text
RadByte/
├── RadCore/           shared wire primitives, codec, NBT, safe types
├── RadProtocol/       packet schemas and client/server session primitives
├── RadVersion/        version descriptors, capabilities, tables, and adapters
│   ├── src/           general interfaces and implementation for 26.3
│   └── data/          verified version data per provenance policy
├── RadGame/           shared server block rules, physics, and inventory
├── RadWorld/          world data, storage, and migrations
├── RadServer/         startup, server loop, sessions, and configuration
├── RadProxy/          routing, not an obligatory protocol translator
├── RadLimbo/
├── RadAPI/            stable addon ABI/API
└── RadClient/         future standalone application; do not create fake UI
```

Internal files per version are permitted if this eases maintenance, but there
is no need to copy the entire game server into `v26_3`, `v26_4`, etc. A dynamic
version module versus static linking is a separate decision based on requirements;
the presence of a `RadVersion` directory does not imply the necessity of a DLL.

Dependency direction: shared types/protocol do not import `RadServer`, GUI, or
version runtime; server and future client use clear shared
contracts. `RadGame` does not pull server filesystem or addon ABI
into the client; client-side prediction, if needed, is a verified separate model.

## 4. Versions and World Rules

1. Separate `game_version`, `protocol_version`, data/registry version,
   schema/storage version, server build version, ABI version, and capabilities.
   Numbering across these concepts is not equivalent.
2. For the world, explicitly store mechanics profile and data version. Determine
   status of a legacy world without a marker using a documented compatibility rule, with
   rejection and guidance if the determination is ambiguous.
3. A single run operates with one verified version; do not claim
   automatic support for future Minecraft versions or multi-version compatibility.
4. The version registry describes packet IDs, structures, state mappings, and required
   capabilities. Conditionally adding `v26_x` is not working support; each
   release requires codecs, data, semantics, tests, and real clients.
5. If version behavior differs, the shared mechanism invokes explicitly
   selected profile rules. Do not silently switch world rules during
   a routine executable update.
6. World transitions are a separate tool with dry-run, compatibility verification,
   backup/restore, controlled writes, and refusal of unsafe
   downgrade. Test on copies; do not promise an automatic reverse path.

## 5. Shared Protocol Layer for Server and Client

1. Define direction/state-aware packet IDs and structures for 26.3; keep
   packet, codec, NBT, registry, chat, and transport types separate from UI and specific
   server state. Verify exact contracts with independent fixtures.
2. Add verified client session primitives: DNS/connect, handshake,
   status/ping, login, encryption/compression, configuration, Play, disconnect,
   timeouts, memory bounds, and backpressure. Do not leak client-side state into
   the server session unnecessarily.
3. Explicitly distinguish offline-mode from legitimate online-mode. For Microsoft/Mojang
   authentication, use officially available methods and current terms,
   without storing passwords, bypassing licenses, or logging tokens.
   Protect keys, token lifetimes, failure/retry logic, and concurrent sessions.
4. Convert server and client entity/chunk/item IDs through explicit
   version mappings. Do not embed assumptions about ID equivalence across versions.
5. Embed a test headless client via the public shared API; do not
   consider its roundtrip with the same codec an independent protocol verification.
   Add independent fixtures and tests with a real 26.3 client.
6. Pass only the minimal necessary data between modules; maintain resilience against
   malformed data, packet budgets, cancellation, timeouts, and slow peers.

## 6. Data, Assets, and Future RadClient

1. Define separate shared types for blocks/items/registries and client-facing
   data; rendering, audio, windowing, and input are not server dependencies.
2. If a Minecraft-compatible client is created later, design chunk
   rendering, camera, interpolation, GUI, input, assets, and localization separately.
   Implement incrementally after a verified headless network client.
3. Do not bundle or distribute Minecraft JAR, textures, sounds, or other
   protected assets without rights. Local use of purchased assets or
   alternative assets requires separate review of rules and licenses.
   Do not claim that extracted registry tables automatically carry an open license.
4. For any content package: schema, permitted paths, limits, hashes,
   provenance, and a clear single source of truth. A checksum does not authenticate authorship.
5. The client must not automatically load native libraries/addons from
   an untrusted server. Resource packs, if supported, must have
   clear trust boundaries, validation, and transparent user consent.

## 7. Migration of Current RadVersion26_3

1. Review AGENTS.md, current changes, processes, and baseline ``cargo test --workspace --locked``;
   preserve third-party changes. Establish 26.3 startup, version package, RadAPI,
   direct/proxy/limbo, and world restart as behavioral baseline tests.
2. Map dependencies and compile version-specific/global inventories: constants,
   `mc777_data`, packet IDs, version package, UUID, world format, and ABI.
3. Extract shared types and functions in small increments; run
   fmt, clippy, and relevant tests after each step. Do not alter architecture and
   gameplay mechanics simultaneously without necessity.
4. First ensure compatibility of legacy `rad-server.exe` with ABI v1 or
   ship matching executable + library as a unified release. Do not modify
   C ABI v1 silently; if a new contract is needed, specify explicit version, sizes,
   ownership, errors, and mismatch tests. Do not consider Rust ABI stable.
5. Gradually transfer world/gameplay/net ownership from the version library
   such that current protocol 777 and persistence function identically. Do not overwrite legacy
   `versions/26.3`, config, ops/whitelist, addons, and worlds.
6. Decide on the final launch model: statically linked engine with a single
   executable or a small ABI-stable version library. Do not maintain
   both modes merely for appearances. Moving/removing the legacy package requires
   documented verification and backup, without silent dangerous fallback.
7. Update packagers, `dist`, README, API documents, and current tests;
   verify execution from a different working directory and against legacy config/world copies.

## 8. Verification and Readiness Criteria

- Functional baseline 26.3: status, login/configuration/Play, movement,
  inventory, blocks, persistence, and restart, direct and existing proxy flows.
- Tests with mismatched version: status behaves per defined contract, Login
  without adapter receives correct disconnect; no accidental decoding
  of foreign protocol. Real client 26.3 and independent wire fixtures.
- Tests for version manifest and data: mismatch, corruption, path traversal,
  missing files, unsupported version, resource limits enforcement, concurrent
  launch, and atomic updates.
- ABI v1 compatibility or documented new release, verification of DLL
  loading, RadAPI/Radons, Windows, Linux, and macOS strictly within the limits of actually executed
  builds and tests. Do not promise platform parity without running on the target.
- Restoration of a legacy world copy: position/inventory, block entities, ticks,
  world clock, backup/restore, protection against invalid or newer formats.
- Measure and record baseline login latency/TPS/memory; verify timeout,
  non-blocking network, concurrency, and cancellation.
- ``cargo test --workspace --locked``, isolated smoke tests, and consistent release build;
  the script may update `dist` and terminate the server, verify this in advance.
  Do not overwrite working worlds/configs/addons; leave startup to the user.

## 9. Future Next Version and Client

Only after stabilizing 26.3, add **one real** subsequent version as
an architectural validation: inventory of packet, data, world, and mechanics changes;
dedicated version descriptor and necessary overrides; independent tests; backup
and migration. Compare the volume of shared code and impact on legacy world profiles.
Do not create empty modules for future versions or mark them as supported.

Next stage `RadClient`: dedicated design and milestone for headless network,
status/login, then chunk state/prediction, followed by rendering/UI/input,
assets, and legality verification. Establish criteria for independence, security,
and compatibility prior to investing in a full graphical client.

## 10. Execution Mode and Reporting

Work sequentially, do not stop after analysis or the first stage; do not
duplicate already implemented functionality. In case of external blocking, proceed with
independent tasks and document the reason. Do not touch working world and access
files without a backup. Prior to completion, report actually implemented items,
passed verifications, manual client/target OS runs, build paths, pending items, and
the next step. Do not label the project clean-room or a new version supported
without documented independent verification.
