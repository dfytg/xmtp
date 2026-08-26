//! SDK types: enumerations, option structs, data structs, and signer trait.

/// XMTP network environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Env {
    /// Local development node.
    Local,
    /// Shared development environment.
    #[default]
    Dev,
    /// Production environment.
    Production,
}

impl Env {
    /// gRPC API URL for this environment.
    #[must_use]
    pub const fn url(self) -> &'static str {
        match self {
            Self::Local => "http://localhost:5556",
            Self::Dev => "https://grpc.dev.xmtp.network:443",
            Self::Production => "https://grpc.production.xmtp.network:443",
        }
    }

    /// History-sync (device-sync) server URL.
    ///
    /// Copied from libxmtp v1.11.0 `DeviceSyncUrls`. Used by
    /// [`Client::send_sync_request`](crate::Client::send_sync_request) when no
    /// override is stored. Not applied automatically on
    /// [`ClientBuilder::build`](crate::ClientBuilder::build).
    #[must_use]
    pub const fn history_sync_url(self) -> &'static str {
        match self {
            Self::Local => "http://0.0.0.0:5558",
            Self::Dev => "https://message-history.dev.ephemera.network",
            Self::Production => "https://message-history.ephemera.network",
        }
    }

    /// Documented d14n gateway URL for this environment.
    ///
    /// Unused unless [`ClientBuilder::gateway_host`](crate::ClientBuilder::gateway_host)
    /// is set — d14n stays off by default. `Env::Dev` maps to the staging payer
    /// (`GrpcUrls::GATEWAY` under libxmtp `if_dev!`), not `GrpcUrlsDev::GATEWAY`.
    #[must_use]
    pub const fn gateway_url(self) -> &'static str {
        match self {
            Self::Local => "http://localhost:5052",
            Self::Dev => "https://payer.testnet-staging.xmtp.network:443",
            Self::Production => "https://payer.testnet.xmtp.network:443",
        }
    }

    /// Whether this environment uses TLS.
    #[must_use]
    pub const fn is_secure(self) -> bool {
        !matches!(self, Self::Local)
    }
}

macro_rules! ffi_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($(#[$vm:meta])* $variant:ident = $val:expr),* $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(i32)]
        $vis enum $name { $($(#[$vm])* $variant = $val),* }

        impl $name {
            /// Convert from FFI `i32`. Returns `None` for unknown values.
            #[must_use]
            pub const fn from_ffi(v: i32) -> Option<Self> {
                match v { $($val => Some(Self::$variant),)* _ => None }
            }
        }
    };
}

ffi_enum! {
    /// Identifier kind.
    pub enum IdentifierKind {
        /// Externally-owned Ethereum account.
        Ethereum = 0,
        /// Passkey.
        Passkey = 1,
    }
}

ffi_enum! {
    /// Conversation type.
    pub enum ConversationType {
        /// Direct message.
        Dm = 0,
        /// Group conversation.
        Group = 1,
        /// Internal sync group.
        Sync = 2,
        /// One-shot conversation.
        Oneshot = 3,
    }
}

ffi_enum! {
    /// Consent state.
    pub enum ConsentState {
        /// Not yet determined.
        Unknown = 0,
        /// Explicitly allowed.
        Allowed = 1,
        /// Explicitly denied.
        Denied = 2,
    }
}

ffi_enum! {
    /// Consent entity type.
    pub enum ConsentEntityType {
        /// Group ID.
        GroupId = 0,
        /// Inbox ID.
        InboxId = 1,
    }
}

ffi_enum! {
    /// Message kind.
    pub enum MessageKind {
        /// Application-level content.
        Application = 0,
        /// MLS membership change.
        MembershipChange = 1,
    }
}

ffi_enum! {
    /// Message delivery status.
    pub enum DeliveryStatus {
        /// Not yet published.
        Unpublished = 0,
        /// Published to the network.
        Published = 1,
        /// Failed to publish.
        Failed = 2,
    }
}

ffi_enum! {
    /// Group member permission level.
    pub enum PermissionLevel {
        /// Regular member.
        Member = 0,
        /// Administrator.
        Admin = 1,
        /// Super administrator.
        SuperAdmin = 2,
    }
}

ffi_enum! {
    /// Group permissions preset.
    pub enum GroupPermissionsPreset {
        /// All members have equal permissions.
        AllMembers = 0,
        /// Only admins can modify the group.
        AdminOnly = 1,
        /// Custom permission set.
        Custom = 2,
    }
}

ffi_enum! {
    /// Group membership state.
    pub enum MembershipState {
        /// Allowed (active member).
        Allowed = 0,
        /// Rejected.
        Rejected = 1,
        /// Pending approval.
        Pending = 2,
        /// Restored after removal.
        Restored = 3,
        /// Pending removal.
        PendingRemove = 4,
    }
}

ffi_enum! {
    /// Sort direction for message listing.
    pub enum SortDirection {
        /// Ascending (oldest first).
        Ascending = 0,
        /// Descending (newest first).
        Descending = 1,
    }
}

ffi_enum! {
    /// Permission policy values (0-based, matching C read struct).
    ///
    /// **Note:** The FFI *write* function uses 1-based values.
    /// Use [`PermissionPolicy::to_write_i32`] when calling write APIs.
    pub enum PermissionPolicy {
        /// Allow all.
        Allow = 0,
        /// Deny all.
        Deny = 1,
        /// Admin only.
        AdminOnly = 2,
        /// Super admin only.
        SuperAdminOnly = 3,
        /// Policy does not exist (read-only, set by protocol).
        DoesNotExist = 4,
        /// Other / unknown policy (read-only, set by protocol).
        Other = 5,
    }
}

impl PermissionPolicy {
    /// Convert to the 1-based i32 value expected by FFI write functions.
    #[must_use]
    pub const fn to_write_i32(self) -> i32 {
        self as i32 + 1
    }
}

ffi_enum! {
    /// Permission update type.
    pub enum PermissionUpdateType {
        /// Add member.
        AddMember = 1,
        /// Remove member.
        RemoveMember = 2,
        /// Add admin.
        AddAdmin = 3,
        /// Remove admin.
        RemoveAdmin = 4,
        /// Update metadata.
        UpdateMetadata = 5,
    }
}

ffi_enum! {
    /// Kind of preference update from a preference stream.
    pub enum PreferenceKind {
        /// Consent preference change.
        Consent = 0,
        /// HMAC key update.
        HmacKey = 1,
    }
}

/// Metadata field names for [`PermissionUpdateType::UpdateMetadata`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetadataField {
    /// Group name.
    GroupName,
    /// Group description.
    Description,
    /// Group image URL.
    ImageUrl,
    /// Group pinned frame URL.
    PinnedFrameUrl,
    /// Custom app data.
    AppData,
    /// Message disappearing settings.
    MessageDisappearing,
}

impl MetadataField {
    /// Returns the FFI string key for this metadata field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GroupName => "group_name",
            Self::Description => "description",
            Self::ImageUrl => "group_image_url_square",
            Self::PinnedFrameUrl => "group_pinned_frame_url",
            Self::AppData => "app_data",
            Self::MessageDisappearing => "message_disappearing",
        }
    }
}

/// An account identifier (address + kind).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountIdentifier {
    /// The account address or public key.
    pub address: String,
    /// The kind of identifier.
    pub kind: IdentifierKind,
}

/// Options for creating a group conversation.
#[derive(Debug, Clone, Default)]
pub struct CreateGroupOptions {
    /// Permission preset.
    pub permissions: Option<GroupPermissionsPreset>,
    /// Group name.
    pub name: Option<String>,
    /// Group description.
    pub description: Option<String>,
    /// Group image URL.
    pub image_url: Option<String>,
    /// Custom app data (max 8192 bytes).
    pub app_data: Option<String>,
    /// Disappearing message settings. `None` = disabled.
    pub disappearing: Option<DisappearingSettings>,
}

/// Options for creating a DM conversation.
#[derive(Debug, Clone, Copy, Default)]
pub struct CreateDmOptions {
    /// Disappearing message settings. `None` = disabled.
    pub disappearing: Option<DisappearingSettings>,
}

/// Options for listing messages.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListMessagesOptions {
    /// Only messages sent after this timestamp (ns).
    pub sent_after_ns: i64,
    /// Only messages sent before this timestamp (ns).
    pub sent_before_ns: i64,
    /// Maximum number of messages to return.
    pub limit: i64,
    /// Sort direction.
    pub direction: Option<SortDirection>,
    /// Filter by delivery status. `None` = all.
    pub delivery_status: Option<DeliveryStatus>,
    /// Filter by message kind. `None` = all.
    pub kind: Option<MessageKind>,
}

/// Sort order for listing conversations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ConversationOrderBy {
    /// Order by creation timestamp (default).
    #[default]
    CreatedAt = 0,
    /// Order by last activity timestamp.
    LastActivity = 1,
}

impl ConversationOrderBy {
    /// Convert from an FFI `i32` value. Returns `None` for unknown values.
    #[must_use]
    pub const fn from_ffi(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::CreatedAt),
            1 => Some(Self::LastActivity),
            _ => None,
        }
    }
}

/// Options for listing conversations.
#[derive(Debug, Clone, Default)]
pub struct ListConversationsOptions {
    /// Filter by conversation type. `None` = all.
    pub conversation_type: Option<ConversationType>,
    /// Maximum number of conversations.
    pub limit: i64,
    /// Only conversations created after this timestamp (ns).
    pub created_after_ns: i64,
    /// Only conversations created before this timestamp (ns).
    pub created_before_ns: i64,
    /// Only conversations with last activity after this timestamp (ns).
    pub last_activity_after_ns: i64,
    /// Only conversations with last activity before this timestamp (ns).
    pub last_activity_before_ns: i64,
    /// Filter by consent states. Empty = all.
    pub consent_states: Vec<ConsentState>,
    /// Sort order for results.
    pub order_by: ConversationOrderBy,
    /// Whether to include duplicate DMs.
    pub include_duplicate_dms: bool,
}

/// Options for sending a message.
#[derive(Debug, Clone, Copy)]
pub struct SendOptions {
    /// Whether to include in push notifications (default: `true`).
    pub should_push: bool,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self { should_push: true }
    }
}

/// Disappearing message settings.
#[derive(Debug, Clone, Copy, Default)]
pub struct DisappearingSettings {
    /// Start timestamp (ns).
    pub from_ns: i64,
    /// Duration (ns).
    pub in_ns: i64,
}

/// Full permission policy set for a conversation.
#[derive(Debug, Clone, Copy)]
pub struct PermissionPolicySet {
    /// Policy for adding members.
    pub add_member: PermissionPolicy,
    /// Policy for removing members.
    pub remove_member: PermissionPolicy,
    /// Policy for adding admins.
    pub add_admin: PermissionPolicy,
    /// Policy for removing admins.
    pub remove_admin: PermissionPolicy,
    /// Policy for updating the group name.
    pub update_group_name: PermissionPolicy,
    /// Policy for updating the group description.
    pub update_group_description: PermissionPolicy,
    /// Policy for updating the group image URL.
    pub update_group_image_url: PermissionPolicy,
    /// Policy for updating disappearing message settings.
    pub update_message_disappearing: PermissionPolicy,
    /// Policy for updating app data.
    pub update_app_data: PermissionPolicy,
}

/// Group permissions (preset + full policy set).
#[derive(Debug, Clone, Copy)]
pub struct Permissions {
    /// The permissions preset used when creating the group.
    pub preset: GroupPermissionsPreset,
    /// The full set of per-action policies.
    pub policies: PermissionPolicySet,
}

/// Conversation metadata (creator + type).
#[derive(Debug, Clone)]
pub struct ConversationMetadata {
    /// The inbox ID of the conversation creator.
    pub creator_inbox_id: String,
    /// The type of conversation.
    pub conversation_type: ConversationType,
}

/// A single cursor entry for debug info.
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    /// Originator node ID.
    pub originator_id: u32,
    /// Sequence number within the originator.
    pub sequence_id: u64,
}

/// Conversation debug information.
#[derive(Debug, Clone)]
pub struct ConversationDebugInfo {
    /// Current MLS epoch.
    pub epoch: u64,
    /// Whether a fork has been detected.
    pub maybe_forked: bool,
    /// Human-readable fork details.
    pub fork_details: Option<String>,
    /// Whether the commit log is forked. `None` = unknown.
    pub is_commit_log_forked: Option<bool>,
    /// Local commit log summary.
    pub local_commit_log: Option<String>,
    /// Remote commit log summary.
    pub remote_commit_log: Option<String>,
    /// Cursor entries for each originator.
    pub cursors: Vec<Cursor>,
}

/// A single HMAC key entry.
#[derive(Debug, Clone)]
pub struct HmacKey {
    /// The raw key bytes.
    pub key: Vec<u8>,
    /// The epoch this key belongs to.
    pub epoch: i64,
}

/// HMAC keys for a conversation group.
#[derive(Debug, Clone)]
pub struct HmacKeyEntry {
    /// Hex-encoded group ID.
    pub group_id: String,
    /// HMAC keys for each epoch.
    pub keys: Vec<HmacKey>,
}

/// Per-inbox last-read timestamp.
#[derive(Debug, Clone)]
pub struct LastReadTime {
    /// The inbox ID.
    pub inbox_id: String,
    /// Last-read timestamp in nanoseconds.
    pub timestamp_ns: i64,
}

/// MLS API call statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiStats {
    /// Number of `upload_key_package` calls.
    pub upload_key_package: i64,
    /// Number of `fetch_key_package` calls.
    pub fetch_key_package: i64,
    /// Number of `send_group_messages` calls.
    pub send_group_messages: i64,
    /// Number of `send_welcome_messages` calls.
    pub send_welcome_messages: i64,
    /// Number of `query_group_messages` calls.
    pub query_group_messages: i64,
    /// Number of `query_welcome_messages` calls.
    pub query_welcome_messages: i64,
    /// Number of `subscribe_messages` calls.
    pub subscribe_messages: i64,
    /// Number of `subscribe_welcomes` calls.
    pub subscribe_welcomes: i64,
    /// Number of `publish_commit_log` calls.
    pub publish_commit_log: i64,
    /// Number of `query_commit_log` calls.
    pub query_commit_log: i64,
    /// Number of `get_newest_group_message` calls.
    pub get_newest_group_message: i64,
}

/// Identity API call statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityStats {
    /// Number of `publish_identity_update` calls.
    pub publish_identity_update: i64,
    /// Number of `get_identity_updates_v2` calls.
    pub get_identity_updates_v2: i64,
    /// Number of `get_inbox_ids` calls.
    pub get_inbox_ids: i64,
    /// Number of `verify_smart_contract_wallet_signature` calls.
    pub verify_smart_contract_wallet_signature: i64,
}

/// Key package status for an installation.
#[derive(Debug, Clone)]
pub struct KeyPackageStatus {
    /// Hex-encoded installation ID.
    pub installation_id: String,
    /// Whether the key package is valid.
    pub valid: bool,
    /// `not_before` timestamp (0 if unavailable).
    pub not_before: u64,
    /// `not_after` timestamp (0 if unavailable).
    pub not_after: u64,
    /// Validation error message, if any.
    pub validation_error: Option<String>,
}

/// Result of a sync operation.
#[derive(Debug, Clone, Copy)]
pub struct SyncResult {
    /// Number of conversations successfully synced.
    pub synced: u32,
    /// Number of conversations eligible for sync.
    pub eligible: u32,
}

/// Snapshot of an inbox's identity state.
#[derive(Debug, Clone)]
pub struct InboxState {
    /// The inbox ID.
    pub inbox_id: String,
    /// Recovery identifier.
    pub recovery_identifier: String,
    /// Associated account identifiers.
    pub identifiers: Vec<String>,
    /// Installation IDs (hex).
    pub installation_ids: Vec<String>,
}

/// Trait for signing messages during XMTP identity operations.
pub trait Signer: Send + Sync {
    /// The account identifier for this signer.
    fn identifier(&self) -> AccountIdentifier;

    /// Sign the given text and return raw signature bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if signing fails (e.g. key unavailable or hardware error).
    fn sign(&self, text: &str) -> crate::error::Result<Vec<u8>>;

    /// Whether this is a smart contract wallet (ERC-1271). Default: `false`.
    fn is_smart_wallet(&self) -> bool {
        false
    }

    /// EVM chain ID for SCW verification.
    fn chain_id(&self) -> u64 {
        1
    }

    /// Block number for SCW verification. 0 = latest.
    fn block_number(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_url_exact_values() {
        assert_eq!(Env::Local.url(), "http://localhost:5556");
        assert_eq!(Env::Dev.url(), "https://grpc.dev.xmtp.network:443");
        assert_eq!(
            Env::Production.url(),
            "https://grpc.production.xmtp.network:443"
        );
    }

    #[test]
    fn env_history_sync_url_copies_device_sync_urls_v1_11_0() {
        assert_eq!(Env::Local.history_sync_url(), "http://0.0.0.0:5558");
        assert_eq!(
            Env::Dev.history_sync_url(),
            "https://message-history.dev.ephemera.network"
        );
        assert_eq!(
            Env::Production.history_sync_url(),
            "https://message-history.ephemera.network"
        );
    }

    #[test]
    fn env_gateway_url_is_documented_staging_for_dev() {
        assert_eq!(Env::Local.gateway_url(), "http://localhost:5052");
        assert_eq!(
            Env::Dev.gateway_url(),
            "https://payer.testnet-staging.xmtp.network:443"
        );
        assert_eq!(
            Env::Production.gateway_url(),
            "https://payer.testnet.xmtp.network:443"
        );
    }

    #[test]
    fn env_local_is_insecure_others_secure() {
        assert!(!Env::Local.is_secure());
        assert!(Env::Dev.is_secure());
        assert!(Env::Production.is_secure());
    }

    /// FFI discriminant contract: identity & conversation enums.
    #[test]
    fn ffi_discriminants_identity_and_conversation() {
        assert_eq!(IdentifierKind::from_ffi(0), Some(IdentifierKind::Ethereum));
        assert_eq!(IdentifierKind::from_ffi(1), Some(IdentifierKind::Passkey));
        assert_eq!(IdentifierKind::from_ffi(2), None);

        assert_eq!(ConversationType::from_ffi(0), Some(ConversationType::Dm));
        assert_eq!(ConversationType::from_ffi(1), Some(ConversationType::Group));
        assert_eq!(ConversationType::from_ffi(2), Some(ConversationType::Sync));
        assert_eq!(
            ConversationType::from_ffi(3),
            Some(ConversationType::Oneshot)
        );
        assert_eq!(ConversationType::from_ffi(4), None);

        assert_eq!(ConsentState::from_ffi(0), Some(ConsentState::Unknown));
        assert_eq!(ConsentState::from_ffi(1), Some(ConsentState::Allowed));
        assert_eq!(ConsentState::from_ffi(2), Some(ConsentState::Denied));
        assert_eq!(ConsentState::from_ffi(-1), None);

        assert_eq!(
            ConversationOrderBy::from_ffi(0),
            Some(ConversationOrderBy::CreatedAt)
        );
        assert_eq!(
            ConversationOrderBy::from_ffi(1),
            Some(ConversationOrderBy::LastActivity)
        );
        assert_eq!(ConversationOrderBy::from_ffi(-1), None);
    }

    /// FFI discriminant contract: message & preference enums.
    #[test]
    fn ffi_discriminants_message_and_preference() {
        assert_eq!(MessageKind::from_ffi(0), Some(MessageKind::Application));
        assert_eq!(
            MessageKind::from_ffi(1),
            Some(MessageKind::MembershipChange)
        );
        assert_eq!(MessageKind::from_ffi(2), None);

        assert_eq!(
            DeliveryStatus::from_ffi(0),
            Some(DeliveryStatus::Unpublished)
        );
        assert_eq!(DeliveryStatus::from_ffi(1), Some(DeliveryStatus::Published));
        assert_eq!(DeliveryStatus::from_ffi(2), Some(DeliveryStatus::Failed));
        assert_eq!(DeliveryStatus::from_ffi(3), None);

        assert_eq!(SortDirection::from_ffi(0), Some(SortDirection::Ascending));
        assert_eq!(SortDirection::from_ffi(1), Some(SortDirection::Descending));
        assert_eq!(SortDirection::from_ffi(2), None);

        assert_eq!(PreferenceKind::from_ffi(0), Some(PreferenceKind::Consent));
        assert_eq!(PreferenceKind::from_ffi(1), Some(PreferenceKind::HmacKey));
        assert_eq!(PreferenceKind::from_ffi(2), None);
    }

    /// `PermissionPolicy` discriminants + the `+1` write-offset logic.
    #[test]
    fn permission_policy_discriminants_and_write_offset() {
        assert_eq!(PermissionPolicy::from_ffi(0), Some(PermissionPolicy::Allow));
        assert_eq!(PermissionPolicy::from_ffi(1), Some(PermissionPolicy::Deny));
        assert_eq!(
            PermissionPolicy::from_ffi(2),
            Some(PermissionPolicy::AdminOnly)
        );
        assert_eq!(
            PermissionPolicy::from_ffi(3),
            Some(PermissionPolicy::SuperAdminOnly)
        );
        assert_eq!(
            PermissionPolicy::from_ffi(4),
            Some(PermissionPolicy::DoesNotExist)
        );
        assert_eq!(PermissionPolicy::from_ffi(5), Some(PermissionPolicy::Other));
        assert_eq!(PermissionPolicy::from_ffi(99), None);

        assert_eq!(PermissionPolicy::Allow.to_write_i32(), 1);
        assert_eq!(PermissionPolicy::Deny.to_write_i32(), 2);
        assert_eq!(PermissionPolicy::SuperAdminOnly.to_write_i32(), 4);
    }
}
