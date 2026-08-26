//! CLI argument definitions and subcommand routing.

pub(crate) mod agent;
pub(crate) mod config;
pub(crate) mod inspect;
pub(crate) mod profile;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use xmtp::Env;

fn cli_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            let pkg = env!("CARGO_PKG_VERSION");
            xmtp::libxmtp_version()
                .map_or_else(|_| pkg.to_owned(), |lib| format!("{pkg} (libxmtp {lib})"))
        })
        .as_str()
}

/// Interactive XMTP TUI chat client.
///
/// Launch without a subcommand to enter the TUI chat interface.
/// Use subcommands for one-shot operations.
#[derive(Parser)]
#[command(
    name = "xmtp",
    version = cli_version(),
    about,
    args_conflicts_with_subcommands = true
)]
pub(crate) struct Cli {
    /// Profile name for TUI session (uses default if omitted).
    pub profile: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Shared output-format options for commands that support `--json`.
#[derive(clap::Args, Debug, Clone, Copy)]
pub(crate) struct OutputArgs {
    /// Output in JSON format for agent/script consumption.
    #[arg(long)]
    pub json: bool,
}

/// One-shot operations (run and exit).
#[derive(Subcommand)]
pub(crate) enum Command {
    /// Create a new profile and register with the XMTP network.
    New(NewArgs),
    /// List all saved profiles.
    #[command(alias = "ls")]
    List {
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Remove a profile and its data.
    #[command(alias = "rm")]
    Remove {
        /// Profile name to remove.
        name: String,
    },
    /// Remove ALL profiles and data (requires confirmation).
    Clear,
    /// Show or set the default profile.
    Default {
        /// Profile name to set as default. Omit to show current.
        name: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Show profile information and installations.
    Info {
        /// Profile name (uses default if omitted).
        name: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Revoke all installations except the current one.
    Revoke {
        /// Profile name (uses default if omitted).
        name: Option<String>,
    },

    /// List conversations (alias: convs).
    #[command(alias = "convs")]
    Conversations {
        /// Filter by consent state (allowed, denied, unknown).
        #[arg(long)]
        consent: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// List messages in a conversation (alias: msgs).
    #[command(alias = "msgs")]
    Messages {
        /// Conversation ID.
        conv: String,
        /// Maximum number of messages to return.
        #[arg(long)]
        limit: Option<usize>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Send a text message to a conversation.
    Send {
        /// Conversation ID.
        conv: String,
        /// Message text.
        text: String,
        /// Send a push notification to the recipient's device.
        #[arg(long, default_value_t = false)]
        push: bool,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Create or open a DM conversation.
    Dm {
        /// Recipient address, ENS name, or inbox ID.
        address: String,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Create a group conversation.
    #[command(name = "group")]
    CreateGroup {
        /// Member addresses, ENS names, or inbox IDs.
        #[arg(required = true)]
        members: Vec<String>,
        /// Group name.
        #[arg(long)]
        name: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// List members of a conversation.
    #[command(alias = "who")]
    Members {
        /// Conversation ID.
        conv: String,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Check if addresses can receive XMTP messages.
    CanMessage {
        /// Addresses, ENS names, or inbox IDs to check.
        #[arg(required = true)]
        addresses: Vec<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Accept or deny a conversation request.
    Request {
        /// Conversation ID.
        conv: String,
        /// Action: accept or deny.
        action: String,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Stream real-time events as NDJSON (newline-delimited JSON).
    Stream {
        /// What to stream: messages, conversations, or all.
        #[arg(default_value = "all")]
        kind: String,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
    },
    /// Request history transfer from another installation.
    Sync {
        /// Override history-sync server URL (defaults from the profile env).
        #[arg(long)]
        history_url: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Device-sync archive operations.
    #[command(subcommand)]
    Archive(ArchiveCommand),
}

/// Subcommands for `xmtp archive`.
#[derive(Subcommand)]
pub(crate) enum ArchiveCommand {
    /// Export an encrypted archive to a local file.
    Create {
        /// Destination file path.
        path: PathBuf,
        /// 32-byte archive key as hex. Defaults to the profile `db.key`.
        #[arg(long)]
        key: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Import an encrypted archive from a local file.
    Import {
        /// Source file path.
        path: PathBuf,
        /// 32-byte archive key as hex. Defaults to the profile `db.key`.
        #[arg(long)]
        key: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Read metadata from an archive file.
    Metadata {
        /// Archive file path.
        path: PathBuf,
        /// 32-byte archive key as hex. Defaults to the profile `db.key`.
        #[arg(long)]
        key: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// List archives available in the device-sync group.
    List {
        /// How many days back to look.
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Process a sync archive (`--pin` omitted = latest).
    Process {
        /// Pin identifying the archive. Omit to process the latest.
        #[arg(long)]
        pin: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
    /// Upload a sync archive for the given pin.
    Send {
        /// Pin identifying the archive to upload.
        pin: String,
        /// Override history-sync server URL (defaults from the profile env).
        #[arg(long)]
        history_url: Option<String>,
        /// Profile name (uses default if omitted).
        #[arg(long)]
        profile: Option<String>,
        #[command(flatten)]
        output: OutputArgs,
    },
}

impl ArchiveCommand {
    const fn is_json(&self) -> bool {
        match self {
            Self::Create { output, .. }
            | Self::Import { output, .. }
            | Self::Metadata { output, .. }
            | Self::List { output, .. }
            | Self::Process { output, .. }
            | Self::Send { output, .. } => output.json,
        }
    }
}

impl Command {
    /// Whether this command was invoked with `--json`.
    pub(crate) const fn is_json(&self) -> bool {
        match self {
            Self::List { output, .. }
            | Self::Default { output, .. }
            | Self::Info { output, .. }
            | Self::Conversations { output, .. }
            | Self::Messages { output, .. }
            | Self::Send { output, .. }
            | Self::Dm { output, .. }
            | Self::CreateGroup { output, .. }
            | Self::Members { output, .. }
            | Self::CanMessage { output, .. }
            | Self::Request { output, .. }
            | Self::Sync { output, .. } => output.json,
            Self::Archive(cmd) => cmd.is_json(),
            Self::Stream { .. } => true,
            Self::New(_) | Self::Remove { .. } | Self::Clear | Self::Revoke { .. } => false,
        }
    }
}

/// Arguments for the `new` subcommand.
#[derive(clap::Args)]
pub(crate) struct NewArgs {
    /// Profile name.
    pub name: String,

    /// XMTP network environment.
    #[arg(long, default_value = "dev", value_parser = parse_env)]
    pub env: Env,

    /// Ethereum RPC URL for ENS resolution.
    #[arg(long, default_value = xmtp::DEFAULT_RPC)]
    pub rpc_url: String,

    /// Import a hex-encoded private key.
    #[arg(long, conflicts_with_all = ["key", "ledger"])]
    pub import: Option<String>,

    /// Copy a private key file into the profile.
    #[arg(long, conflicts_with_all = ["import", "ledger"])]
    pub key: Option<PathBuf>,

    /// Use a Ledger hardware wallet (optionally specify account index, default 0).
    #[arg(long, num_args = 0..=1, default_missing_value = "0",
          conflicts_with_all = ["import", "key"])]
    pub ledger: Option<usize>,
}

pub(crate) fn parse_env(s: &str) -> Result<Env, String> {
    match s.to_ascii_lowercase().as_str() {
        "dev" | "development" => Ok(Env::Dev),
        "prod" | "production" => Ok(Env::Production),
        "local" | "localhost" => Ok(Env::Local),
        _ => Err(format!(
            "unknown environment: {s} (expected: dev, production, local)"
        )),
    }
}
