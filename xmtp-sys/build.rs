//! Build script for xmtp-sys.
//!
//! 1. Locates or downloads the pre-built `libxmtp_ffi` static library.
//! 2. Optionally runs `bindgen` to regenerate Rust bindings (feature `regenerate`).
//! 3. Configures the linker for static linking + required system libraries.
//!
//! # Environment variables
//!
//! - `XMTP_FFI_DIR` — Path to a local FFI build directory containing both
//!   the static library and the `include/xmtp_ffi.h` header. When set,
//!   skips downloading. This is the primary flow for local development.
//!
//! - `XMTP_FFI_VERSION` — Override the FFI release version to download.
//!   Defaults to the crate version from `Cargo.toml`.
//!
//! - `XMTP_UPDATE_BINDINGS` — When set (any value) alongside the `regenerate`
//!   feature, the freshly generated `bindings.rs` is copied back to
//!   `src/bindings.rs` so it can be committed to the repository.
//!
//! Download builds verify the archive against `sha256sums/{version}` when that
//! file exists. There is no skip flag; use `XMTP_FFI_DIR` to avoid downloading.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[path = "src/integrity.rs"]
mod integrity;

/// GitHub repository for downloading FFI releases.
const GITHUB_REPO: &str = "qntx/xmtp";

/// Hex digest of the verified archive, written next to the extracted lib.
const ARCHIVE_STAMP: &str = "archive.sha256";

fn main() {
    println!("cargo:rerun-if-env-changed=XMTP_FFI_DIR");
    println!("cargo:rerun-if-env-changed=XMTP_FFI_VERSION");
    println!("cargo:rerun-if-env-changed=XMTP_UPDATE_BINDINGS");
    println!("cargo:rerun-if-env-changed=DOCS_RS");
    println!("cargo:rerun-if-changed=sha256sums");

    // docs.rs builds run in a network-isolated sandbox; skip downloading and
    // linking the native library entirely. The crate still compiles for docs.
    if env::var("DOCS_RS").is_ok() {
        return;
    }

    let target = env::var("TARGET").expect("TARGET not set");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    if let Ok(ffi_dir) = env::var("XMTP_FFI_DIR") {
        // Option 1: Local FFI build directory (development).
        let ffi_path = PathBuf::from(&ffi_dir);
        println!("cargo:warning=Using local FFI directory: {ffi_dir}");
        println!("cargo:rustc-link-search=native={ffi_dir}");

        // Optionally regenerate bindings from the header.
        #[cfg(feature = "regenerate")]
        {
            let header_path = find_header(&ffi_path);
            println!("cargo:rerun-if-changed={}", header_path.display());
            generate_bindings(&header_path, &out_dir);
        }
        let _ = ffi_path;
    } else {
        // Option 2: Download from GitHub Releases.
        let version = env::var("XMTP_FFI_VERSION")
            .unwrap_or_else(|_| env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION not set"));

        let lib_dir = out_dir.join("lib");
        let lib_file = lib_dir.join(lib_filename(&target));
        let asset = integrity::release_asset_name(&target);
        let expected = expected_archive_digest(&version, &asset);
        let stamp = fs::read_to_string(lib_dir.join(ARCHIVE_STAMP)).ok();
        if !integrity::cached_extract_is_fresh(
            lib_file.exists(),
            expected.as_deref(),
            stamp.as_deref(),
        ) {
            download_and_extract(&version, &target, &lib_dir, expected.as_deref());
        }

        println!("cargo:rustc-link-search=native={}", lib_dir.display());

        // Optionally regenerate bindings from the downloaded header.
        #[cfg(feature = "regenerate")]
        {
            let header_path = lib_dir.join("xmtp_ffi.h");
            assert!(
                header_path.exists(),
                "Header file not found: {}",
                header_path.display()
            );
            println!("cargo:rerun-if-changed={}", header_path.display());
            generate_bindings(&header_path, &out_dir);
        }
    }

    link_native_lib(&target);
    link_system_libs(&target);
}

/// Static library filename for the given target.
fn lib_filename(target: &str) -> &'static str {
    if target.contains("windows") {
        "xmtp_ffi.lib"
    } else {
        "libxmtp_ffi.a"
    }
}

/// Emit `cargo:rustc-link-lib=static=xmtp_ffi`.
fn link_native_lib(target: &str) {
    let _ = target;
    println!("cargo:rustc-link-lib=static=xmtp_ffi");
}

/// Link platform-specific system libraries required by the FFI static library.
fn link_system_libs(target: &str) {
    if target.contains("linux") {
        for lib in ["pthread", "dl", "m", "gcc_s", "stdc++"] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    } else if target.contains("apple") {
        for framework in ["Security", "CoreFoundation", "SystemConfiguration"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rustc-link-lib=dylib=c++");
    } else if target.contains("windows") {
        for lib in [
            "ws2_32", "bcrypt", "ntdll", "userenv", "crypt32", "secur32", "ncrypt", "user32",
        ] {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }
}

/// Fail-closed only when `sha256sums/{version}` exists. Missing map → warning, not error.
fn load_checksum_map(version: &str) -> Option<BTreeMap<String, String>> {
    let map_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
            .join("sha256sums")
            .join(version);
    println!("cargo:rerun-if-changed={}", map_path.display());
    match fs::read_to_string(&map_path) {
        Ok(s) => Some(integrity::parse_gnu_sha256sum(&s).unwrap_or_else(|e| {
            panic!("Invalid GNU sha256sum map {}: {e}", map_path.display());
        })),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            println!(
                "cargo:warning=No SHA-256 map for FFI version {version}; skipping checksum verification"
            );
            None
        }
        Err(e) => panic!("Failed to read SHA-256 map {}: {e}", map_path.display()),
    }
}

fn expected_archive_digest(version: &str, asset: &str) -> Option<String> {
    let map = load_checksum_map(version)?;
    let Some(hash) = map.get(asset) else {
        panic!("SHA-256 map for {version} has no hash for asset {asset}");
    };
    Some(hash.clone())
}

/// Download the archive from GitHub Releases and extract it to `dest`.
fn download_and_extract(version: &str, target: &str, dest: &Path, expected: Option<&str>) {
    let asset = integrity::release_asset_name(target);
    let url = format!("https://github.com/{GITHUB_REPO}/releases/download/ffi-v{version}/{asset}");

    eprintln!("Downloading {url}");

    let resp = ureq::get(&url)
        .call()
        .unwrap_or_else(|e| panic!("Failed to download FFI library from {url}: {e}"));

    let mut bytes = Vec::new();
    resp.into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("Failed to read FFI library from {url}: {e}"));

    let actual = format!("{:x}", Sha256::digest(&bytes));
    if let Some(expected) = expected {
        integrity::verify_digest(&asset, expected, &actual).unwrap_or_else(|e| panic!("{e}"));
    }

    if let Err(e) = extract_ffi_archive(&bytes, target, dest) {
        let _ = fs::remove_dir_all(dest);
        panic!("{e}");
    }
    if let Err(e) = fs::write(dest.join(ARCHIVE_STAMP), format!("{actual}\n")) {
        let _ = fs::remove_dir_all(dest);
        panic!("Failed to write archive checksum stamp: {e}");
    }
}

/// Wipe `dest`, extract allowlisted members, require the static lib.
fn extract_ffi_archive(bytes: &[u8], target: &str, dest: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| format!("Failed to clear {}: {e}", dest.display()))?;
    }
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create {}: {e}", dest.display()))?;
    let cursor = io::Cursor::new(bytes);
    if target.contains("windows") {
        extract_zip(cursor, dest)?;
    } else {
        extract_tar_gz(cursor, dest)?;
    }
    let lib = dest.join(lib_filename(target));
    if !lib.exists() {
        return Err(format!(
            "Expected library file not found after extraction: {}",
            lib.display()
        ));
    }
    Ok(())
}

/// Extract a `.tar.gz` archive into `dest`. Rejects path traversal and unexpected entries.
fn extract_tar_gz(reader: impl io::Read, dest: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("Failed to read tar.gz archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Failed to read tar entry: {e}"))?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        let name = {
            let path = entry
                .path()
                .map_err(|e| format!("Failed to read tar entry path: {e}"))?;
            path.to_string_lossy().into_owned()
        };
        if kind.is_symlink() || kind.is_hard_link() {
            return Err(format!("Refusing to extract link from FFI tar: {name}"));
        }
        if !kind.is_file() {
            return Err(format!("Refusing non-file tar entry {name:?} ({kind:?})"));
        }
        write_allowed_entry(&name, dest, &mut entry)?;
    }
    Ok(())
}

/// Extract a `.zip` archive into `dest`. Rejects path traversal and unexpected entries.
fn extract_zip(reader: impl io::Read + io::Seek, dest: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("Failed to read zip archive: {e}"))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry {i}: {e}"))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_owned();
        if file.is_symlink() {
            return Err(format!("Refusing to extract symlink from FFI zip: {name}"));
        }
        write_allowed_entry(&name, dest, &mut file)?;
    }
    Ok(())
}

fn write_allowed_entry(name: &str, dest: &Path, reader: &mut impl io::Read) -> Result<(), String> {
    let Some(safe) = integrity::safe_archive_entry_name(name) else {
        return Err(format!(
            "Refusing archive entry {name:?}: path traversal or unexpected file"
        ));
    };
    let out = dest.join(safe);
    let mut outfile =
        fs::File::create(&out).map_err(|e| format!("Failed to create {}: {e}", out.display()))?;
    io::copy(reader, &mut outfile)
        .map_err(|e| format!("Failed to extract {}: {e}", out.display()))?;
    Ok(())
}

/// Locate the C header file relative to a local FFI directory.
///
/// Tries multiple common layouts:
/// - `{ffi_dir}/include/xmtp_ffi.h` (when XMTP_FFI_DIR points to the crate root)
/// - `{ffi_dir}/xmtp_ffi.h`         (when header is alongside the lib)
/// - `{ffi_dir}/../../include/xmtp_ffi.h` (when pointing to target/release/)
#[cfg(feature = "regenerate")]
fn find_header(ffi_dir: &Path) -> PathBuf {
    let candidates = [
        ffi_dir.join("include").join("xmtp_ffi.h"),
        ffi_dir.join("xmtp_ffi.h"),
        ffi_dir
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("include").join("xmtp_ffi.h"))
            .unwrap_or_default(),
    ];
    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    panic!(
        "Cannot find xmtp_ffi.h near XMTP_FFI_DIR={}\nSearched: {:?}",
        ffi_dir.display(),
        candidates
    );
}

/// Run `bindgen` on the C header to produce `$OUT_DIR/bindings.rs`.
///
/// cbindgen emits both `enum Foo { .. };` and `typedef int32_t Foo;` for
/// `#[repr(i32)]` enums (C enum sizes are implementation-defined, so the
/// typedef ensures ABI safety). bindgen cannot reconcile both definitions,
/// so we strip the redundant `typedef int32_t` lines before generating.
#[cfg(feature = "regenerate")]
fn generate_bindings(header: &Path, out_dir: &Path) {
    let cleaned = preprocess_header(header, out_dir);

    let bindings = bindgen::Builder::default()
        .header(cleaned.to_str().expect("path is not valid UTF-8"))
        // Parse as C++ so enum names are valid type names after we strip
        // the conflicting `typedef int32_t` lines.
        .clang_arg("-xc++")
        // Use core types instead of std for maximum compatibility.
        .use_core()
        // Only generate bindings for our symbols, not system headers.
        .allowlist_function("xmtp_.*")
        .allowlist_type("Xmtp.*")
        .allowlist_var("XMTP_.*")
        // Derive common traits where possible.
        .derive_debug(true)
        .derive_default(true)
        .derive_eq(true)
        // Generate proper Rust enums with #[repr(i32)].
        .default_enum_style(bindgen::EnumVariation::Rust {
            non_exhaustive: true,
        })
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("bindgen failed to generate bindings from xmtp_ffi.h");

    let out_file = out_dir.join("bindings.rs");
    bindings
        .write_to_file(&out_file)
        .expect("Failed to write bindings.rs");

    // When XMTP_UPDATE_BINDINGS is set, copy the freshly generated bindings
    // back to src/bindings.rs so they can be committed to the repository.
    if env::var("XMTP_UPDATE_BINDINGS").is_ok() {
        let manifest_dir =
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
        let committed = manifest_dir.join("src").join("bindings.rs");
        fs::copy(&out_file, &committed).expect("Failed to copy bindings.rs to src/");
        println!(
            "cargo:warning=Updated committed bindings: {}",
            committed.display()
        );
    }
}

/// Strip `typedef int32_t XmtpFfi...;` lines from the header to prevent
/// bindgen from seeing conflicting definitions for enum types.
#[cfg(feature = "regenerate")]
fn preprocess_header(header: &Path, out_dir: &Path) -> PathBuf {
    let content = fs::read_to_string(header).expect("Failed to read header");
    let cleaned: String = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // Remove lines like: `typedef int32_t XmtpFfiConversationType;`
            !(trimmed.starts_with("typedef int32_t Xmtp") && trimmed.ends_with(';'))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let out = out_dir.join("xmtp_ffi_cleaned.h");
    fs::write(&out, cleaned).expect("Failed to write preprocessed header");
    out
}
