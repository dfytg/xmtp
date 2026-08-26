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

# Operator gate: refuse ffi-v* unless xmtp-ffi and xmtp-sys versions match VERSION.
# Default VERSION=0.2.0. Override: VERSION=x.y.z just tag-ffi
# Print-only unless CONFIRM=1 (creates local tag). Does not push.
tag-ffi:
    #!/usr/bin/env sh
    set -eu
    root="{{justfile_directory()}}"
    ffi=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/xmtp-ffi/Cargo.toml" | head -1)
    sys=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/xmtp-sys/Cargo.toml" | head -1)
    version="${VERSION:-0.2.0}"
    if [ -z "$ffi" ] || [ -z "$sys" ]; then
        echo "failed to read versions from xmtp-ffi/Cargo.toml and xmtp-sys/Cargo.toml" >&2
        exit 1
    fi
    if [ "$ffi" != "$sys" ]; then
        echo "xmtp-ffi $ffi != xmtp-sys $sys" >&2
        exit 1
    fi
    if [ "$ffi" != "$version" ]; then
        echo "crate versions $ffi != VERSION=$version" >&2
        exit 1
    fi
    echo "xmtp-ffi=$ffi xmtp-sys=$sys"
    echo
    echo "git tag ffi-v$version"
    echo "git push --tags"
    echo
    echo "After ffi-build.yml:"
    echo "  five GitHub Release assets must exist:"
    echo "    xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz"
    echo "    xmtp-ffi-aarch64-unknown-linux-gnu.tar.gz"
    echo "    xmtp-ffi-aarch64-apple-darwin.tar.gz"
    echo "    xmtp-ffi-x86_64-pc-windows-msvc.zip"
    echo "    xmtp-ffi-aarch64-pc-windows-msvc.zip"
    echo "  hash job (checksums) must be green"
    echo "  then: cargo publish -p xmtp-sys"
    if [ "${CONFIRM:-}" = "1" ]; then
        git tag "ffi-v$version"
        echo
        echo "created tag ffi-v$version (not pushed)"
    else
        echo
        echo "print-only; re-run with CONFIRM=1 to create git tag ffi-v$version"
    fi

# Operator gate: refuse v* unless workspace.package.version matches VERSION.
# Default VERSION=0.10.0. Override: VERSION=x.y.z just tag-sdk
# Print-only unless CONFIRM=1 (creates local tag). Does not push.
tag-sdk:
    #!/usr/bin/env sh
    set -eu
    root="{{justfile_directory()}}"
    sdk=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)
    version="${VERSION:-0.10.0}"
    if [ -z "$sdk" ]; then
        echo "failed to read version from Cargo.toml" >&2
        exit 1
    fi
    if ! grep -q '^version.workspace = true' "$root/xmtp/Cargo.toml"; then
        echo "xmtp does not inherit workspace version" >&2
        exit 1
    fi
    if ! grep -q '^version.workspace = true' "$root/xmtp-cli/Cargo.toml"; then
        echo "xmtp-cli does not inherit workspace version" >&2
        exit 1
    fi
    if [ "$sdk" != "$version" ]; then
        echo "workspace version $sdk != VERSION=$version" >&2
        exit 1
    fi
    echo "xmtp=$sdk xmtp-cli=$sdk"
    echo
    echo "git tag v$version"
    echo "git push --tags"
    echo
    echo "Required Dev smoke before push:"
    echo "  xmtp new ci-smoke --env dev"
    echo "  xmtp dm <peer> --profile ci-smoke"
    echo
    echo "After release.yml + publish.yml:"
    echo "  CLI binaries on GitHub Release v$version"
    echo "  cargo publish of xmtp and xmtp-cli"
    if [ "${CONFIRM:-}" = "1" ]; then
        git tag "v$version"
        echo
        echo "created tag v$version (not pushed)"
    else
        echo
        echo "print-only; re-run with CONFIRM=1 to create git tag v$version"
    fi

changelog:
    test -s CHANGELOG.md

pre-commit: fmt clippy test build changelog
