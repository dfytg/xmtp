//! Profile configuration persistence and shared infrastructure.

use std::path::{Path, PathBuf};
use std::{fmt, fs};

use xmtp::{AlloySigner, Client, EnsResolver, Env, IdentifierKind, LedgerSigner, Signer};

/// Base data directory for all profiles.
pub(crate) fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xmtp-cli")
}

/// Data directory for a specific profile.
pub(crate) fn profile_dir(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// Read the default profile name (falls back to `"default"`).
pub(crate) fn default_profile() -> String {
    let path = data_dir().join(".default");
    fs::read_to_string(path).map_or_else(|_| "default".into(), |s| s.trim().to_owned())
}

/// Persist the default profile name.
pub(crate) fn set_default(name: &str) -> xmtp::Result<()> {
    let base = data_dir();
    mkdir_secret(&base)?;
    fs::write(base.join(".default"), name).map_err(|e| xmtp::XmtpError::Io(format!("write: {e}")))
}

/// Create `dir` (and parents) and set mode 0700 on Unix.
pub(crate) fn mkdir_secret(dir: &Path) -> xmtp::Result<()> {
    fs::create_dir_all(dir).map_err(|e| xmtp::XmtpError::Io(format!("mkdir: {e}")))?;
    chmod(dir, 0o700)
}

/// Write `bytes` and set mode 0600 on Unix.
pub(crate) fn write_secret(path: &Path, bytes: &[u8]) -> xmtp::Result<()> {
    fs::write(path, bytes)
        .map_err(|e| xmtp::XmtpError::Io(format!("write {}: {e}", path.display())))?;
    chmod(path, 0o600)
}

/// Set Unix file mode. No-op on non-Unix.
pub(crate) fn chmod(path: &Path, mode: u32) -> xmtp::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| xmtp::XmtpError::Io(format!("chmod {}: {e}", path.display())))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

/// 32 cryptographically random bytes.
pub(crate) fn random_key32() -> xmtp::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| xmtp::XmtpError::Io(format!("rng: {e}")))?;
    Ok(key)
}

/// Decode a 32-byte key from hex (`0x` prefix optional).
pub(crate) fn parse_hex32(s: &str) -> xmtp::Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s)
        .map_err(|e| xmtp::XmtpError::InvalidArgument(format!("invalid hex: {e}")))?;
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| xmtp::XmtpError::InvalidArgument(format!("key must be 32 bytes, got {len}")))
}

/// Read a 32-byte key file.
pub(crate) fn read_key32(path: &Path) -> xmtp::Result<[u8; 32]> {
    let bytes =
        fs::read(path).map_err(|e| xmtp::XmtpError::Io(format!("read {}: {e}", path.display())))?;
    let len = bytes.len();
    bytes.try_into().map_err(|_| {
        xmtp::XmtpError::InvalidArgument(format!(
            "key file {} must be 32 bytes, got {len}",
            path.display()
        ))
    })
}

/// Load `db.key` for a profile. Missing file is a hard error (no plaintext DBs).
pub(crate) fn load_db_key(profile: &str) -> xmtp::Result<[u8; 32]> {
    let path = profile_dir(profile).join("db.key");
    if !path.exists() {
        return Err(xmtp::XmtpError::InvalidArgument(format!(
            "profile '{profile}' is missing db.key; unencrypted profiles are not supported"
        )));
    }
    read_key32(&path)
}

/// Create a profile directory (0700) and generate `db.key` (0600).
pub(crate) fn init_profile_dir(dir: &Path) -> xmtp::Result<[u8; 32]> {
    mkdir_secret(dir)?;
    let key = random_key32()?;
    write_secret(&dir.join("db.key"), &key)?;
    Ok(key)
}

/// How a profile signs messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignerKind {
    /// Local key file (`identity.key`).
    File,
    /// Ledger hardware wallet with account index.
    Ledger(usize),
}

impl fmt::Display for SignerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File => f.write_str("file"),
            Self::Ledger(i) => write!(f, "ledger:{i}"),
        }
    }
}

/// Persistent per-profile configuration stored as `profile.conf`.
#[derive(Debug, Clone)]
pub(crate) struct ProfileConfig {
    pub env: Env,
    pub rpc_url: String,
    pub signer: SignerKind,
    /// Cached wallet address (avoids needing signer just to read address).
    pub address: String,
}

impl ProfileConfig {
    /// Load from `<profile_dir>/profile.conf`.
    pub(crate) fn load(profile: &str) -> xmtp::Result<Self> {
        let path = profile_dir(profile).join("profile.conf");
        let text = fs::read_to_string(&path)
            .map_err(|e| xmtp::XmtpError::Io(format!("load config: {e}")))?;

        let mut env = Env::Dev;
        let mut rpc_url = String::from("https://eth.llamarpc.com");
        let mut signer = SignerKind::File;
        let mut address = String::new();

        for line in text.lines() {
            let Some((k, v)) = line.trim().split_once('=') else {
                continue;
            };
            match k.trim() {
                "env" => env = super::parse_env(v.trim()).map_err(xmtp::XmtpError::Ffi)?,
                "rpc_url" => v.trim().clone_into(&mut rpc_url),
                "signer" => signer = parse_signer(v.trim()),
                "address" => v.trim().clone_into(&mut address),
                _ => {}
            }
        }

        Ok(Self {
            env,
            rpc_url,
            signer,
            address,
        })
    }

    /// Save to `<profile_dir>/profile.conf`.
    pub(crate) fn save(&self, profile: &str) -> xmtp::Result<()> {
        let dir = profile_dir(profile);
        mkdir_secret(&dir)?;
        let content = format!(
            "env={}\nrpc_url={}\nsigner={}\naddress={}\n",
            env_name(self.env),
            self.rpc_url,
            self.signer,
            self.address,
        );
        fs::write(dir.join("profile.conf"), content)
            .map_err(|e| xmtp::XmtpError::Io(format!("write config: {e}")))
    }
}

/// Open a profile without a signer (for TUI and info — no signing needed).
pub(crate) fn open_client(profile: &str) -> xmtp::Result<(ProfileConfig, Client)> {
    open_client_with(profile, None)
}

/// Open a profile, optionally overriding the history-sync URL.
pub(crate) fn open_client_with(
    profile: &str,
    history_url: Option<&str>,
) -> xmtp::Result<(ProfileConfig, Client)> {
    let cfg = ProfileConfig::load(profile)?;
    let db = profile_dir(profile).join("messages.db3");
    let key = load_db_key(profile)?;
    let client = build_client(&cfg, &db.to_string_lossy(), None, &key, history_url)?;
    Ok((cfg, client))
}

/// Open a profile with a signer (for operations that need signing, e.g. revoke).
pub(crate) fn open_with_signer(
    profile: &str,
) -> xmtp::Result<(ProfileConfig, Box<dyn Signer>, Client)> {
    let cfg = ProfileConfig::load(profile)?;
    let dir = profile_dir(profile);

    let signer: Box<dyn Signer> = match cfg.signer {
        SignerKind::File => {
            let key = read_key32(&dir.join("identity.key"))?;
            chmod(&dir.join("identity.key"), 0o600)?;
            Box::new(AlloySigner::from_bytes(&key)?)
        }
        SignerKind::Ledger(index) => {
            eprintln!("Connecting to Ledger (index {index})...");
            Box::new(LedgerSigner::new(index)?)
        }
    };

    let db = dir.join("messages.db3");
    let key = load_db_key(profile)?;
    let client = build_client(
        &cfg,
        &db.to_string_lossy(),
        Some(signer.as_ref()),
        &key,
        None,
    )?;
    Ok((cfg, signer, client))
}

/// Build an XMTP client with automatic stale-DB recovery.
///
/// Local DB is always encrypted with `encryption_key` (32 bytes from `db.key`).
/// When `signer` is `Some`, uses `build(signer)` which may register.
/// When `None`, uses `build_existing()` with the stored address (no signing).
pub(crate) fn build_client(
    cfg: &ProfileConfig,
    db_path: &str,
    signer: Option<&dyn Signer>,
    encryption_key: &[u8; 32],
    history_url: Option<&str>,
) -> xmtp::Result<Client> {
    let build = |path: &str| {
        let mut b = Client::builder().env(cfg.env).db_path(path);
        b = b.encryption_key(*encryption_key)?;
        if let Some(url) = history_url {
            b = b.history_sync_url(url);
        }
        if let Ok(r) = EnsResolver::new(&cfg.rpc_url) {
            b = b.resolver(r);
        }
        match signer {
            Some(s) => b.build(s),
            None => b.build_existing(&cfg.address, IdentifierKind::Ethereum),
        }
    };

    match build(db_path) {
        Ok(c) => Ok(c),
        Err(e) if e.to_string().contains("does not match the stored InboxId") => {
            for ext in ["", "-shm", "-wal"] {
                drop(fs::remove_file(format!("{db_path}{ext}")));
            }
            build(db_path)
        }
        Err(e) => Err(e),
    }
}

fn parse_signer(value: &str) -> SignerKind {
    if value.starts_with("ledger") {
        let idx = value
            .strip_prefix("ledger:")
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        SignerKind::Ledger(idx)
    } else {
        SignerKind::File
    }
}

/// Human-readable environment name.
pub(crate) const fn env_name(env: Env) -> &'static str {
    match env {
        Env::Dev => "dev",
        Env::Production => "production",
        Env::Local => "local",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            let mut suffix = [0u8; 8];
            getrandom::fill(&mut suffix).expect("rng");
            let dir = std::env::temp_dir().join(format!("xmtp-cli-test-{}", hex::encode(suffix)));
            fs::create_dir_all(&dir).expect("mkdir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.0));
        }
    }

    #[test]
    fn init_profile_dir_writes_db_key_0600_dir_0700() {
        let tmp = Tmp::new();
        let dir = tmp.path().join("alice");
        let key = init_profile_dir(&dir).expect("init");
        let db_key = dir.join("db.key");
        assert_eq!(
            fs::metadata(&db_key).expect("db.key meta").len(),
            32,
            "db.key length"
        );
        let got = read_key32(&db_key).expect("read db.key");
        assert_eq!(got, key, "db.key bytes");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir_mode = fs::metadata(&dir).expect("dir meta").permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700, "profile dir mode");
            let key_mode = fs::metadata(dir.join("db.key"))
                .expect("key meta")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(key_mode, 0o600, "db.key mode");
        }
    }

    #[test]
    fn write_secret_identity_key_is_0600() {
        let tmp = Tmp::new();
        let path = tmp.path().join("identity.key");
        let key = random_key32().expect("rng");
        write_secret(&path, &key).expect("write");
        assert_eq!(read_key32(&path).expect("read"), key, "identity.key bytes");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "identity.key mode");
        }
    }

    #[test]
    fn missing_db_key_is_hard_error() {
        let err = load_db_key("___xmtp_cli_no_such_profile___").expect_err("missing");
        let msg = err.to_string();
        assert!(msg.contains("db.key"), "{msg}");
        assert!(msg.contains("unencrypted"), "{msg}");
    }

    #[test]
    fn missing_db_key_file_on_disk_errors() {
        let tmp = Tmp::new();
        let path = tmp.path().join("db.key");
        let err = read_key32(&path).expect_err("missing file");
        assert!(err.to_string().contains("db.key"), "{}", err.to_string());
    }

    #[test]
    fn db_key_wrong_length_is_error() {
        let tmp = Tmp::new();
        let path = tmp.path().join("db.key");
        write_secret(&path, &[0u8; 16]).expect("write");
        let err = read_key32(&path).expect_err("len");
        let msg = err.to_string();
        assert!(msg.contains("32 bytes"), "{msg}");
    }

    #[test]
    fn parse_hex32_accepts_64_hex_and_0x_prefix() {
        let hex64 = "ab".repeat(32);
        let a = parse_hex32(&hex64).expect("hex");
        let b = parse_hex32(&format!("0x{hex64}")).expect("0x");
        assert_eq!(a, b, "prefix");
        assert!(parse_hex32("aa").is_err(), "short");
        assert!(parse_hex32("zz").is_err(), "not hex");
    }
}
