//! Profile management commands: new, list, remove, clear, default.

use std::fs;
use std::io;

use xmtp::{AlloySigner, Client, LedgerSigner, Signer};

use super::NewArgs;
use super::config::{self, ProfileConfig, SignerKind};
use crate::decode;

/// Create a new profile, register with the XMTP network, and save config.
///
/// Returns the config (with address) and a ready-to-use client.
pub(crate) fn create(args: &NewArgs) -> xmtp::Result<(ProfileConfig, Client)> {
    let dir = config::profile_dir(&args.name);
    if dir.join("profile.conf").exists() {
        return Err(xmtp::XmtpError::InvalidArgument(format!(
            "profile '{}' already exists",
            args.name
        )));
    }

    let db_key = config::init_profile_dir(&dir)?;
    let key_path = dir.join("identity.key");
    let db_path = dir.join("messages.db3");

    // Determine signer kind and create signer.
    let (signer_kind, signer): (SignerKind, Box<dyn Signer>) = if let Some(index) = args.ledger {
        (
            SignerKind::Ledger(index),
            Box::new(LedgerSigner::new(index)?),
        )
    } else {
        if let Some(ref hex) = args.import {
            import_hex_key(hex, &key_path)?;
        } else if let Some(ref src) = args.key {
            fs::copy(src, &key_path).map_err(|e| xmtp::XmtpError::Io(format!("copy key: {e}")))?;
            config::chmod(&key_path, 0o600)?;
        }
        (SignerKind::File, Box::new(load_or_create_key(&key_path)?))
    };

    // Copy database if provided.
    if let Some(ref src) = args.db {
        fs::copy(src, &db_path).map_err(|e| xmtp::XmtpError::Io(format!("copy db: {e}")))?;
    }

    // Register with the XMTP network.
    let address = signer.identifier().address;
    let cfg = ProfileConfig {
        env: args.env,
        rpc_url: args.rpc_url.clone(),
        signer: signer_kind,
        address: address.clone(),
    };
    let client = config::build_client(
        &cfg,
        &db_path.to_string_lossy(),
        Some(signer.as_ref()),
        &db_key,
        None,
    )?;
    let inbox_id = client.inbox_id()?;

    // Save profile config (address is now known after signer creation).
    cfg.save(&args.name)?;

    // Set as default if this is the first profile ever.
    if !config::data_dir().join(".default").exists() {
        config::set_default(&args.name)?;
    }

    println!("Profile '{}' created.", args.name);
    println!("  Address:  {address}");
    println!("  Inbox ID: {inbox_id}");
    println!("  Env:      {}", config::env_name(args.env));
    Ok((cfg, client))
}

/// List all saved profiles.
pub(crate) fn list(json: bool) -> xmtp::Result<()> {
    let base = config::data_dir();
    if !base.exists() {
        if json {
            println!("{}", serde_json::json!({"profiles": []}));
        } else {
            println!("No profiles found.");
        }
        return Ok(());
    }

    let default = config::default_profile();

    let mut entries: Vec<_> = fs::read_dir(&base)
        .map_err(|e| xmtp::XmtpError::Io(format!("read dir: {e}")))?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(fs::DirEntry::file_name);

    if json {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if let Ok(cfg) = ProfileConfig::load(&name) {
                    serde_json::json!({
                        "name": name,
                        "address": cfg.address,
                        "signer": cfg.signer.to_string(),
                        "env": config::env_name(cfg.env),
                        "is_default": *name == default,
                    })
                } else {
                    serde_json::json!({
                        "name": name,
                        "is_default": *name == default,
                    })
                }
            })
            .collect();
        println!("{}", serde_json::json!({"profiles": items}));
        return Ok(());
    }

    if entries.is_empty() {
        println!("No profiles found.");
        return Ok(());
    }

    for entry in &entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let star = if *name == default { " *" } else { "" };

        if let Ok(cfg) = ProfileConfig::load(&name) {
            let addr = if cfg.address.is_empty() {
                "—".into()
            } else {
                decode::truncate_id(&cfg.address, 14)
            };
            println!(
                "  {name:<16} {addr:<16} [{:<10}] [{}]{star}",
                cfg.signer,
                config::env_name(cfg.env),
            );
        } else {
            println!("  {name:<16} [no config]{star}");
        }
    }
    println!("\n  * = default");
    Ok(())
}

/// Remove a single profile directory.
pub(crate) fn remove(name: &str) -> xmtp::Result<()> {
    let dir = config::profile_dir(name);
    if !dir.exists() {
        println!("Profile '{name}' does not exist.");
        return Ok(());
    }
    fs::remove_dir_all(&dir).map_err(|e| xmtp::XmtpError::Io(format!("remove: {e}")))?;
    println!("Removed profile '{name}'.");
    Ok(())
}

/// Remove ALL profiles after confirmation from stdin.
pub(crate) fn clear() -> xmtp::Result<()> {
    let base = config::data_dir();
    if !base.exists() {
        println!("Nothing to clear.");
        return Ok(());
    }

    eprint!(
        "This will delete ALL data in {}.  Continue? [y/N] ",
        base.display()
    );
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|e| xmtp::XmtpError::Io(format!("stdin: {e}")))?;

    if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
        println!("Aborted.");
        return Ok(());
    }

    fs::remove_dir_all(&base).map_err(|e| xmtp::XmtpError::Io(format!("clear: {e}")))?;
    println!("All profiles deleted.");
    Ok(())
}

/// Show or set the default profile.
pub(crate) fn default(name: Option<&str>, json: bool) -> xmtp::Result<()> {
    if let Some(name) = name {
        if !config::profile_dir(name).exists() {
            return Err(xmtp::XmtpError::InvalidArgument(format!(
                "profile '{name}' does not exist"
            )));
        }
        config::set_default(name)?;
        if json {
            println!("{}", serde_json::json!({"default": name}));
        } else {
            println!("Default profile set to '{name}'.");
        }
    } else {
        let d = config::default_profile();
        if json {
            println!("{}", serde_json::json!({"default": d}));
        } else {
            println!("{d}");
        }
    }
    Ok(())
}

/// Decode a hex string and write as identity.key (0600).
fn import_hex_key(hex_str: &str, path: &std::path::Path) -> xmtp::Result<()> {
    let key = config::parse_hex32(hex_str)?;
    config::write_secret(path, &key)
}

/// Load an existing key file or generate a new random key (0600).
fn load_or_create_key(path: &std::path::Path) -> xmtp::Result<AlloySigner> {
    let key = if path.exists() {
        let key = config::read_key32(path)?;
        config::chmod(path, 0o600)?;
        key
    } else {
        let key = config::random_key32()?;
        config::write_secret(path, &key)?;
        key
    };
    AlloySigner::from_bytes(&key)
}
