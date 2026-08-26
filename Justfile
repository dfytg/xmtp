# Justfile for Rust project using Cargo

# rustfmt.toml `group_imports` and cbindgen parse.expand require nightly.
nightly := "nightly-2026-08-03"

all: fmt clippy-fix

# Build the project with all features enabled in release mode
build:
    cargo build --workspace --release --all-features

# Check the project for compilation errors without producing binaries
check:
    cargo check --workspace --all-features

# Update dependencies to their latest compatible versions
update:
    cargo update

# Run the project with all features enabled in release mode
run:
    cargo run --release --all-features

# Run all tests with all features enabled
test:
    cargo test --workspace --all-features

# Run benchmarks with all features enabled
bench:
    cargo bench --all-features

# Run Clippy linter with nightly toolchain (check only, for CI)
# Uses workspace lints from Cargo.toml
clippy:
    cargo +{{nightly}} clippy --workspace \
        --all-targets \
        --all-features \
        -- -D warnings

# Run Clippy linter with auto-fix (for development)
clippy-fix:
    cargo +{{nightly}} clippy --workspace \
        --fix \
        --all-targets \
        --all-features \
        --allow-dirty \
        --allow-staged \
        -- -D warnings

# Format the code using rustfmt with nightly toolchain
fmt:
    cargo +{{nightly}} fmt

# Generate documentation for all crates and open it in the browser
doc:
    cargo +{{nightly}} doc --all-features --no-deps --open

# Regenerate xmtp-ffi/include/xmtp_ffi.h (cbindgen; requires nightly)
header:
    XMTP_GEN_HEADER=1 cargo +{{nightly}} build --manifest-path xmtp-ffi/Cargo.toml

# Build the FFI static library with the crate pin (1.97.1); cbindgen skipped
# Optional: XMTP_SCCACHE=1 just ffi-build
ffi-build:
    #!/usr/bin/env sh
    if [ "${XMTP_SCCACHE:-}" = "1" ]; then export RUSTC_WRAPPER=sccache; fi
    cargo build --release --manifest-path xmtp-ffi/Cargo.toml

# Check the FFI crate with the crate pin (1.97.1); cbindgen skipped
ffi-check:
    #!/usr/bin/env sh
    if [ "${XMTP_SCCACHE:-}" = "1" ]; then export RUSTC_WRAPPER=sccache; fi
    cargo check --manifest-path xmtp-ffi/Cargo.toml

# Regenerate xmtp-sys/src/bindings.rs from the committed C header
regenerate-bindings:
    XMTP_FFI_DIR={{justfile_directory()}}/xmtp-ffi XMTP_UPDATE_BINDINGS=1 \
        cargo check -p xmtp-sys --features regenerate

changelog:
    test -s CHANGELOG.md

pre-commit: fmt clippy test build changelog
