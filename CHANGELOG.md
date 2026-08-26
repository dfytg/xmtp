# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `make header`, `make ffi-build`, `make ffi-check`, `make regenerate-bindings`, and `make pre-commit` (mirrored in the Justfile).
- Dependabot cargo updates for `/xmtp-ffi`.
- Root `deny.toml` and `xmtp-ffi/deny.toml` license/source allow-lists from a real `cargo deny check` run.

### Fixed

- Reject non-32-byte DB encryption keys in the SDK before the FFI call.
- Replace untrusted `from_raw_parts` in `xmtp-ffi` with `checked_slice` / `checked_slice_nonempty` / `checked_key32`.
- `to_c_string` reports interior NUL instead of returning a silent null pointer.
- Stream `Subscription` drops `_ctx` last and leaks it if `is_closed` stays 0 after `xmtp_stream_end`.

### Changed

- Pin workspace and `xmtp-ffi` rustc to 1.97.1. Advertise MSRV 1.94 on `xmtp` and `xmtp-sys`.
- Skip cbindgen in `xmtp-ffi` unless `XMTP_GEN_HEADER=1`. Committed `include/xmtp_ffi.h` is the source of truth.
- `make fmt` / `make clippy` / `make header` use pinned `nightly-2026-08-03`; `make ffi-build` / `make ffi-check` use 1.97.1 (no `cargo +`).
- `make clippy` is check-only; `make clippy-fix` auto-fixes.
