# SHA-256 maps for `libxmtp_ffi` release assets

One file per `xmtp-sys` / `xmtp-ffi` version, named `{version}` (no extension). GNU `sha256sum` text mode:

```
<64 hex>  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz
<64 hex>  xmtp-ffi-aarch64-unknown-linux-gnu.tar.gz
<64 hex>  xmtp-ffi-aarch64-apple-darwin.tar.gz
<64 hex>  xmtp-ffi-x86_64-pc-windows-msvc.zip
<64 hex>  xmtp-ffi-aarch64-pc-windows-msvc.zip
```

Two spaces between digest and filename. Binary mode (`<hash> *<name>`) is also accepted.

`build.rs` reads `sha256sums/{CARGO_PKG_VERSION}`. If that file is absent, checksum verification is skipped (`cargo:warning`). If it is present, a missing or mismatched hash for the current target asset fails the build.

Do not add a map for `0.1.11`. Hashes for `0.2.0` are committed with the `ffi-v0.2.0` release.
