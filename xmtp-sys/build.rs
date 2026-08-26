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

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[path = "src/integrity.rs"]
mod integrity;

/// GitHub repository for downloading FFI releases.
const GITHUB_REPO: &str = "qntx/xmtp";

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

        if !lib_file.exists() {
            download_and_extract(&version, &target, &lib_dir);
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

/// Download the archive from GitHub Releases and extract it to `dest`.
fn download_and_extract(version: &str, target: &str, dest: &Path) {
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

    verify_downloaded_archive(version, &asset, &bytes);

    fs::create_dir_all(dest).expect("Failed to create output directory");

    let cursor = io::Cursor::new(&bytes);
    if target.contains("windows") {
        extract_zip(cursor, dest);
    } else {
        extract_tar_gz(cursor, dest);
    }

    let lib = dest.join(lib_filename(target));
    assert!(
        lib.exists(),
        "Expected library file not found after extraction: {}",
        lib.display()
    );
}

/// Fail-closed only when `sha256sums/{version}` exists. Missing map → warning, not error.
fn verify_downloaded_archive(version: &str, asset: &str, bytes: &[u8]) {
    let map_path =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
            .join("sha256sums")
            .join(version);
    println!("cargo:rerun-if-changed={}", map_path.display());

    let text = match fs::read_to_string(&map_path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            println!(
                "cargo:warning=No SHA-256 map for FFI version {version}; skipping checksum verification"
            );
            return;
        }
        Err(e) => panic!("Failed to read SHA-256 map {}: {e}", map_path.display()),
    };

    let map = integrity::parse_gnu_sha256sum(&text).unwrap_or_else(|e| {
        panic!("Invalid GNU sha256sum map {}: {e}", map_path.display());
    });
    let actual = format!("{:x}", Sha256::digest(bytes));
    integrity::verify_asset_hash(&map, asset, &actual).unwrap_or_else(|e| panic!("{e}"));
}

/// Extract a `.tar.gz` archive into `dest`. Rejects path traversal and unexpected entries.
fn extract_tar_gz(reader: impl io::Read, dest: &Path) {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().expect("Failed to read tar.gz archive");
    for entry in entries {
        let mut entry = entry.expect("Failed to read tar entry");
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        let name = {
            let path = entry.path().expect("Failed to read tar entry path");
            path.to_string_lossy().into_owned()
        };
        if kind.is_symlink() || kind.is_hard_link() {
            panic!("Refusing to extract link from FFI tar: {name}");
        }
        if !kind.is_file() {
            panic!("Refusing non-file tar entry {name:?} ({kind:?})");
        }
        write_allowed_entry(&name, dest, &mut entry);
    }
}

/// Extract a `.zip` archive into `dest`. Rejects path traversal and unexpected entries.
fn extract_zip(reader: impl io::Read + io::Seek, dest: &Path) {
    let mut archive = zip::ZipArchive::new(reader).expect("Failed to read zip archive");
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .unwrap_or_else(|e| panic!("Failed to read zip entry {i}: {e}"));
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_owned();
        if file.is_symlink() {
            panic!("Refusing to extract symlink from FFI zip: {name}");
        }
        write_allowed_entry(&name, dest, &mut file);
    }
}

fn write_allowed_entry(name: &str, dest: &Path, reader: &mut impl io::Read) {
    let Some(safe) = integrity::safe_archive_entry_name(name) else {
        panic!("Refusing archive entry {name:?}: path traversal or unexpected file");
    };
    let out = dest.join(safe);
    let mut outfile = fs::File::create(&out)
        .unwrap_or_else(|e| panic!("Failed to create {}: {e}", out.display()));
    io::copy(reader, &mut outfile)
        .unwrap_or_else(|e| panic!("Failed to extract {}: {e}", out.display()));
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
