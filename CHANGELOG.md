# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0] - 2026-08-27

`xmtp` and `xmtp-cli` 0.10.0 on the libxmtp **v1.11.0** / `xmtp-ffi` **0.2.0** train.
`xmtp-sys` stays **0.2.0** (already released; do not retag FFI).

`xmtp --version` prints `{CARGO_PKG_VERSION} (libxmtp …)` from `libxmtp_version()`.

### Added

- History sync is explicit: `Client::send_sync_request` / `send_sync_request_to`,
  `Env::history_sync_url()`, `ClientBuilder::history_sync_url`.
  `ClientBuilder::build()` does **not** send a sync request.
- Device-sync archives: create/import/metadata, list/process/send, `ArchiveOptions`,
  `disable_device_sync()`. Empty-string URL is not a disable switch.
- CLI: `xmtp sync` (`--history-url`), `xmtp archive {create,import,metadata,list,process,send}`,
  `RUST_LOG` FFI tracing.
- Encrypted profiles only: `db.key` (0600), profile dir 0700. Missing `db.key` is refused.
- Extra content decode: transaction reference, wallet send calls, actions, intent.
- `xmtp-sys` SHA-256 map (`sha256sums/{version}`) when present; zip-slip-safe extract.
- `make header`, `ffi-build`, `ffi-check`, `regenerate-bindings`, `tag-ffi`, `tag-sdk`, `pre-commit`.
- Dependabot for `/xmtp-ffi`. Root and `xmtp-ffi` `deny.toml`.
- CI: `ffi-check.yml`, dual `cargo deny`, header dirty check, rustc 1.97.1 compile.

### Changed

- Workspace version 0.9.3 → 0.10.0 (`xmtp`, `xmtp-cli`). FFI crates remain 0.2.0.
- libxmtp git pin **v1.11.0**.
- `ClientBuilder::encryption_key` returns `Result<Self>` (`Err(InvalidArgument)` unless 32 bytes).
- Pin workspace and `xmtp-ffi` rustc to 1.97.1. Advertise MSRV 1.94 on `xmtp` and `xmtp-sys`.
- Skip cbindgen in `xmtp-ffi` unless `XMTP_GEN_HEADER=1`. Committed `include/xmtp_ffi.h` is the source of truth.
- `make fmt` / `make clippy` / `make header` use pinned `nightly-2026-08-03`; `make ffi-build` / `make ffi-check` use 1.97.1.
- `make clippy` is check-only; `make clippy-fix` auto-fixes.
- New profiles always encrypt the local DB. Existing `db.key` is never overwritten.

### Removed

- `Client::request_device_sync` — use `send_sync_request` / `send_sync_request_to`. No alias.
- CLI `--db` path override.

### Fixed

- Reject non-32-byte DB encryption keys in the SDK before the FFI call.
- Replace untrusted `from_raw_parts` in `xmtp-ffi` with `checked_slice` / `checked_slice_nonempty` / `checked_key32`.
- `to_c_string` reports interior NUL instead of returning a silent null pointer.
- Stream `Subscription` drops `_ctx` last and leaks it if `is_closed` stays 0 after `xmtp_stream_end`.
- Install rustls crypto provider; scheme-prefix `gateway_host`.
- Convert device-sync pin strings before `into_raw`.
- Archive metadata and archive count errors.

### Release smoke (required, not automated)

Do this on Dev before pushing `v0.10.0`:

```sh
xmtp new ci-smoke --env dev
xmtp dm <peer-address-or-inbox> --profile ci-smoke
```

Then `make tag-sdk` (print-only) or `make tag-sdk CONFIRM=1` to create a **local** `v0.10.0` tag.
Do not push the tag until the smoke DM succeeds.
