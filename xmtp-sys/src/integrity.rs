//! GNU `sha256sum` parsing and zip-slip path checks used by `build.rs`.

use std::collections::BTreeMap;
use std::path::{Component, Path};

/// Filenames permitted at the archive root. Nested paths are rejected.
pub(crate) const ALLOWED_ARCHIVE_FILES: &[&str] = &["libxmtp_ffi.a", "xmtp_ffi.lib", "xmtp_ffi.h"];

/// GitHub Release asset filename for `target`.
pub(crate) fn release_asset_name(target: &str) -> String {
    if target.contains("windows") {
        format!("xmtp-ffi-{target}.zip")
    } else {
        format!("xmtp-ffi-{target}.tar.gz")
    }
}

/// Parse GNU `sha256sum` text (`<64 hex><two spaces or space-star><filename>`).
///
/// Empty lines are skipped. Any other malformed line is an error. Digests are stored lowercase.
pub(crate) fn parse_gnu_sha256sum(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let Some((hash, name)) = parse_gnu_sha256sum_line(line) else {
            return Err(format!("line {}: not GNU sha256sum format: {line}", i + 1));
        };
        if entries.insert(name.to_owned(), hash).is_some() {
            return Err(format!("duplicate filename {name}"));
        }
    }
    Ok(entries)
}

fn parse_gnu_sha256sum_line(line: &str) -> Option<(String, &str)> {
    let hash = line.get(..64)?;
    if !hash.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let rest = line.get(64..)?;
    let name = rest
        .strip_prefix("  ")
        .or_else(|| rest.strip_prefix(" *"))?;
    if name.is_empty() {
        return None;
    }
    Some((hash.to_ascii_lowercase(), name))
}

/// Whether a previously extracted lib can be reused.
///
/// No map (`expected_hex` is `None`): `lib_exists` is enough.
/// Map present: the stamp must equal the map digest (missing/mismatch → re-download).
pub(crate) fn cached_extract_is_fresh(
    lib_exists: bool,
    expected_hex: Option<&str>,
    stamp: Option<&str>,
) -> bool {
    if !lib_exists {
        return false;
    }
    let Some(expected) = expected_hex else {
        return true;
    };
    stamp.is_some_and(|s| s.trim().eq_ignore_ascii_case(expected))
}

/// Fail-closed: the map must contain `asset` and the digest must match.
#[cfg(test)]
pub(crate) fn verify_asset_hash(
    map: &BTreeMap<String, String>,
    asset: &str,
    actual_hex: &str,
) -> Result<(), String> {
    let Some(expected) = map.get(asset) else {
        return Err(format!("SHA-256 map has no hash for asset {asset}"));
    };
    verify_digest(asset, expected, actual_hex)
}

/// Compare hex digests. `expected` comes from the map; `actual_hex` from the download.
pub(crate) fn verify_digest(asset: &str, expected: &str, actual_hex: &str) -> Result<(), String> {
    let expected = expected.to_ascii_lowercase();
    let actual = actual_hex.to_ascii_lowercase();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 mismatch for {asset}: expected {expected}, got {actual}"
        ))
    }
}

/// Single-component allowlisted name, or `None` for traversal / unexpected paths.
///
/// `..` and absolute paths are rejected, not sanitized. Backslashes are treated as separators
/// so Windows-style zip-slip entries are rejected on Unix builders too.
pub(crate) fn safe_archive_entry_name(name: &str) -> Option<&'static str> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        return None;
    }
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return None;
    }
    let mut file: Option<&str> = None;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let s = part.to_str()?;
                if file.is_some() {
                    return None;
                }
                file = Some(s);
            }
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
        }
    }
    let file = file?;
    ALLOWED_ARCHIVE_FILES
        .iter()
        .copied()
        .find(|&allowed| allowed == file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sample_map() -> String {
        format!(
            "{EMPTY_SHA256}  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n\
             {EMPTY_SHA256}  xmtp-ffi-aarch64-pc-windows-msvc.zip\n"
        )
    }

    #[test]
    fn parse_text_mode_two_spaces() {
        let map = parse_gnu_sha256sum(&sample_map()).expect("parse");
        assert_eq!(map.len(), 2, "two assets");
        assert_eq!(
            map.get("xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz")
                .map(String::as_str),
            Some(EMPTY_SHA256)
        );
    }

    #[test]
    fn parse_binary_mode_star() {
        let line = format!("{EMPTY_SHA256} *xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n");
        let map = parse_gnu_sha256sum(&line).expect("parse");
        assert_eq!(
            map.get("xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz")
                .map(String::as_str),
            Some(EMPTY_SHA256)
        );
    }

    #[test]
    fn parse_skips_empty_lines() {
        let text = format!("\n{EMPTY_SHA256}  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n\n");
        let map = parse_gnu_sha256sum(&text).expect("parse");
        assert_eq!(map.len(), 1, "one asset");
    }

    #[test]
    fn parse_upper_hex_stored_lowercase() {
        let text = format!(
            "{}  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n",
            EMPTY_SHA256.to_ascii_uppercase()
        );
        let map = parse_gnu_sha256sum(&text).expect("parse");
        assert_eq!(
            map.get("xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz")
                .map(String::as_str),
            Some(EMPTY_SHA256)
        );
    }

    #[test]
    fn parse_rejects_malformed_and_duplicate() {
        assert!(parse_gnu_sha256sum("not-a-hash  file.tar.gz\n").is_err());
        assert!(parse_gnu_sha256sum("SHA256 (file.tar.gz) = deadbeef\n").is_err());
        let dup = format!(
            "{EMPTY_SHA256}  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n\
             {EMPTY_SHA256}  xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz\n"
        );
        assert!(parse_gnu_sha256sum(&dup).is_err());
    }

    #[test]
    fn verify_match_mismatch_and_missing_asset() {
        let map = parse_gnu_sha256sum(&sample_map()).expect("parse");
        let asset = "xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz";
        assert!(verify_asset_hash(&map, asset, EMPTY_SHA256).is_ok());
        assert!(verify_asset_hash(&map, asset, &EMPTY_SHA256.to_ascii_uppercase()).is_ok());
        assert!(verify_asset_hash(&map, asset, &"ab".repeat(32)).is_err());
        assert!(
            verify_asset_hash(&map, "xmtp-ffi-aarch64-apple-darwin.tar.gz", EMPTY_SHA256).is_err()
        );
    }

    #[test]
    fn cached_extract_requires_stamp_only_when_map_present() {
        assert!(!cached_extract_is_fresh(false, None, None));
        assert!(cached_extract_is_fresh(true, None, None));
        assert!(cached_extract_is_fresh(true, None, Some(EMPTY_SHA256)));
        assert!(!cached_extract_is_fresh(true, Some(EMPTY_SHA256), None));
        assert!(!cached_extract_is_fresh(
            true,
            Some(EMPTY_SHA256),
            Some(&"ab".repeat(32))
        ));
        assert!(cached_extract_is_fresh(
            true,
            Some(EMPTY_SHA256),
            Some(EMPTY_SHA256)
        ));
        assert!(cached_extract_is_fresh(
            true,
            Some(EMPTY_SHA256),
            Some(&format!("{}\n", EMPTY_SHA256.to_ascii_uppercase()))
        ));
    }

    #[test]
    fn empty_map_is_present_and_fail_closed() {
        let map = parse_gnu_sha256sum("").expect("empty file is a present map");
        assert!(
            verify_asset_hash(
                &map,
                "xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz",
                EMPTY_SHA256
            )
            .is_err()
        );
    }

    #[test]
    fn release_asset_name_matches_github_layout() {
        assert_eq!(
            release_asset_name("x86_64-unknown-linux-gnu"),
            "xmtp-ffi-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            release_asset_name("aarch64-pc-windows-msvc"),
            "xmtp-ffi-aarch64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn safe_entry_allows_root_artifacts_and_dot_slash() {
        assert_eq!(
            safe_archive_entry_name("libxmtp_ffi.a"),
            Some("libxmtp_ffi.a")
        );
        assert_eq!(
            safe_archive_entry_name("xmtp_ffi.lib"),
            Some("xmtp_ffi.lib")
        );
        assert_eq!(safe_archive_entry_name("xmtp_ffi.h"), Some("xmtp_ffi.h"));
        assert_eq!(
            safe_archive_entry_name("./libxmtp_ffi.a"),
            Some("libxmtp_ffi.a")
        );
    }

    #[test]
    fn safe_entry_rejects_zip_slip_and_unexpected_names() {
        assert_eq!(safe_archive_entry_name("../libxmtp_ffi.a"), None);
        assert_eq!(safe_archive_entry_name("foo/../xmtp_ffi.h"), None);
        assert_eq!(safe_archive_entry_name("/tmp/xmtp_ffi.h"), None);
        assert_eq!(safe_archive_entry_name("/xmtp_ffi.h"), None);
        assert_eq!(safe_archive_entry_name("foo/libxmtp_ffi.a"), None);
        assert_eq!(safe_archive_entry_name("..\\xmtp_ffi.lib"), None);
        assert_eq!(safe_archive_entry_name("xmtp_ffi.rs"), None);
        assert_eq!(safe_archive_entry_name(""), None);
        assert_eq!(safe_archive_entry_name("xmtp_ffi.h\0evil"), None);
        assert_eq!(safe_archive_entry_name(".."), None);
    }
}
