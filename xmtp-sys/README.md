# xmtp-sys

Raw FFI bindings to [`libxmtp_ffi`](https://github.com/qntx/xmtp) — the XMTP messaging protocol static library.

> **Note:** This crate provides unsafe, low-level bindings. Prefer the
> safe [`xmtp`](https://crates.io/crates/xmtp) crate for application code.

## How it works

All types and functions are **auto-generated** by [`bindgen`](https://docs.rs/bindgen) from the C header `xmtp_ffi.h` produced by [`cbindgen`](https://docs.rs/cbindgen). Pre-generated bindings are committed to the repository so end users do **not** need `libclang` installed.

At build time, the build script:

1. Downloads the pre-built static library from [GitHub Releases](https://github.com/qntx/xmtp/releases) for the current target platform (or uses a local path via `XMTP_FFI_DIR`).
2. When `sha256sums/{version}` exists, hashes the downloaded archive and fails the build on a missing or mismatched digest for the current target asset.
3. Extracts only `libxmtp_ffi.a` / `xmtp_ffi.lib` / `xmtp_ffi.h`, rejecting `..` and absolute paths.
4. Configures the linker to link the static library plus required system dependencies.

## FFI artifact integrity

Committed files under [`sha256sums/`](sha256sums/) use GNU `sha256sum` format (64 hex digits, two spaces, GitHub Release asset filename). `build.rs` looks up `sha256sums/{CARGO_PKG_VERSION}` (or `XMTP_FFI_VERSION` when set).

- **No file for this version** (current `0.1.11`): skip verification and emit `cargo:warning`.
- **File present:** fail the build if the current target asset is missing from the map or the digest does not match.
- **`XMTP_FFI_DIR`:** skips download and checksums (local staticlib).
- There is no checksum-skip environment variable for download builds.

## Environment variables

| Variable | Description |
| --- | --- |
| `XMTP_FFI_DIR` | Path to a local FFI build directory. Skips downloading. |
| `XMTP_FFI_VERSION` | Override the FFI release version (default: crate version). |
| `XMTP_UPDATE_BINDINGS` | When set with `regenerate` feature, copy generated bindings back to `src/bindings.rs`. |

## Features

| Feature | Description |
| --- | --- |
| `regenerate` | Re-generate bindings from `xmtp_ffi.h` at build time (requires `libclang`). |

## Supported platforms

| Target | Status |
| --- | --- |
| `x86_64-unknown-linux-gnu` | ✅ |
| `aarch64-unknown-linux-gnu` | ✅ |
| `aarch64-apple-darwin` | ✅ |
| `x86_64-pc-windows-msvc` | ✅ |
| `aarch64-pc-windows-msvc` | ✅ |

Intel macOS (`x86_64-apple-darwin`) is unsupported.

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
