# Building and testing RadByte

Use a stable Rust toolchain from the repository root.

## Build

```text
cargo build --workspace --release --locked
```

Cargo writes executables and the native version library to `target/release/`. Start a service directly from its own working directory, with its configuration files and any required version package beside it.

## Test

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Some end-to-end tests require a locally supplied, compatible version package in `versions/26.3`; tests that need it are marked ignored unless explicitly run with their prerequisites.

## Package version data

Provide the required version JSON files and a native library built for the current operating system and architecture. From the server's working directory:

```text
cargo run --manifest-path <checkout>/Cargo.toml --locked -p rad-tools -- package-version <data-directory> <library-file> <versions-directory>
```

The tool creates a new package and refuses to overwrite an existing `26.3` package. Back up the current package before replacing it.

## Back up worlds

Stop RadServer cleanly before copying its world directory. The Rust server handles graceful saves through its normal shutdown command; keep world copies outside the source directory and verify a restored copy before using it.
