//! Core FFI infrastructure: error handling, runtime, memory helpers, type aliases.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::sync::OnceLock;

use tokio::runtime::Runtime;

// ---------------------------------------------------------------------------
// Concrete type aliases (resolve all libxmtp generics once)
// ---------------------------------------------------------------------------

/// Fully resolved client type.
pub type InnerClient = xmtp_mls::Client<xmtp_mls::MlsContext>;

/// Fully resolved group type.
pub type InnerGroup = xmtp_mls::groups::MlsGroup<xmtp_mls::MlsContext>;

// ---------------------------------------------------------------------------
// Opaque handles exposed to C
// ---------------------------------------------------------------------------

/// Opaque client handle.
pub struct FfiClient {
    pub(crate) inner: std::sync::Arc<InnerClient>,
    /// The account identifier used to create this client.
    pub(crate) account_identifier: String,
    /// The application version string passed during creation.
    pub(crate) app_version: String,
}

/// Opaque conversation handle.
pub struct FfiConversation {
    pub(crate) inner: InnerGroup,
}

/// Opaque signature request handle.
pub struct FfiSignatureRequest {
    pub(crate) request:
        std::sync::Arc<tokio::sync::Mutex<xmtp_id::associations::builder::SignatureRequest>>,
    pub(crate) scw_verifier:
        std::sync::Arc<Box<dyn xmtp_id::scw_verifier::SmartContractSignatureVerifier>>,
}

/// Joinable stream task. `xmtp_stream_free` does not wait on this.
pub(crate) type FfiJoinHandle = Box<
    dyn xmtp_common::StreamHandle<
            StreamOutput = std::result::Result<(), xmtp_mls::subscriptions::SubscribeError>,
        > + Send,
>;

/// Opaque stream handle.
pub struct FfiStreamHandle {
    pub(crate) abort: std::sync::Arc<Box<dyn xmtp_common::AbortHandle>>,
    /// Joinable task; `xmtp_stream_free` drops it without waiting.
    #[allow(dead_code, reason = "stored for join; free does not wait")]
    pub(crate) join: std::sync::Mutex<Option<FfiJoinHandle>>,
}

// ---------------------------------------------------------------------------
// Callback types
//
// # Thread Safety
//
// Stream callbacks and `on_close` may be invoked on **any** tokio worker
// thread, not necessarily the thread that created the stream.  The `context`
// pointer passed to callbacks must therefore point to thread-safe data
// (or the C caller must provide its own synchronisation).
// ---------------------------------------------------------------------------

/// Callback for conversation stream events.
pub type FnConversationCallback =
    unsafe extern "C" fn(conversation: *mut FfiConversation, context: *mut std::ffi::c_void);

/// Callback for message stream events.
pub type FnMessageCallback =
    unsafe extern "C" fn(message: *mut FfiMessage, context: *mut std::ffi::c_void);

/// Callback invoked when a stream closes.
/// `error` is null on normal close, or a borrowed error string on failure.
pub type FnOnCloseCallback =
    unsafe extern "C" fn(error: *const c_char, context: *mut std::ffi::c_void);

/// Callback for consent stream events.
/// `records` is borrowed — valid only during the callback invocation.
pub type FnConsentCallback = unsafe extern "C" fn(
    records: *const FfiConsentRecord,
    count: i32,
    context: *mut std::ffi::c_void,
);

/// Callback for message deletion stream events.
/// `message_id` is a borrowed hex string — valid only during the callback.
pub type FnMessageDeletionCallback =
    unsafe extern "C" fn(message_id: *const c_char, context: *mut std::ffi::c_void);

/// Callback for preference stream events.
/// `updates` is borrowed — valid only during the callback invocation.
pub type FnPreferenceCallback = unsafe extern "C" fn(
    updates: *const FfiPreferenceUpdate,
    count: i32,
    context: *mut std::ffi::c_void,
);

// ---------------------------------------------------------------------------
// C-compatible enums (replace magic numbers)
// All enums are exported via cbindgen; suppress unused warnings.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Consent state for an entity.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiConsentState {
    Unknown = 0,
    Allowed = 1,
    Denied = 2,
}

/// Consent entity type.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiConsentEntityType {
    GroupId = 0,
    InboxId = 1,
}

/// Conversation type.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiConversationType {
    Dm = 0,
    Group = 1,
    Sync = 2,
    Oneshot = 3,
}

/// Identifier kind for account identifiers.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiIdentifierKind {
    Ethereum = 0,
    Passkey = 1,
}

/// Group member permission level.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiPermissionLevel {
    Member = 0,
    Admin = 1,
    SuperAdmin = 2,
}

/// Message kind.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiMessageKind {
    Application = 0,
    MembershipChange = 1,
}

/// Message delivery status.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiDeliveryStatus {
    Unpublished = 0,
    Published = 1,
    Failed = 2,
}

/// Group membership state.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiGroupMembershipState {
    Allowed = 0,
    Rejected = 1,
    Pending = 2,
    Restored = 3,
    PendingRemove = 4,
}

/// Permission policy value.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiPermissionPolicy {
    Allow = 0,
    Deny = 1,
    Admin = 2,
    SuperAdmin = 3,
    DoesNotExist = 4,
    Other = 5,
}

/// Group permissions preset.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FfiGroupPermissionsPreset {
    AllMembers = 0,
    AdminOnly = 1,
    Custom = 2,
}

/// Preference update kind.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiPreferenceUpdateKind {
    Consent = 0,
    HmacKey = 1,
}

// ---------------------------------------------------------------------------
// Data transfer types (flat, repr(C))
// ---------------------------------------------------------------------------

/// A stored message exposed to C.
#[allow(dead_code)]
pub struct FfiMessage {
    pub(crate) inner: xmtp_db::group_message::StoredGroupMessage,
}

/// A list of messages.
pub struct FfiMessageList {
    pub(crate) items: Vec<xmtp_db::group_message::StoredGroupMessage>,
}

/// Internal storage for a conversation list item (preserves last_message + fork status).
pub(crate) struct FfiConversationListItemInner {
    pub(crate) group: InnerGroup,
    pub(crate) last_message: Option<xmtp_db::group_message::StoredGroupMessage>,
    pub(crate) is_commit_log_forked: Option<bool>,
}

/// A list of conversations returned from queries.
pub struct FfiConversationList {
    pub(crate) items: Vec<FfiConversationListItemInner>,
}

/// A single group member.
#[repr(C)]
pub struct FfiGroupMember {
    pub(crate) inbox_id: *mut c_char,
    pub(crate) permission_level: FfiPermissionLevel,
    pub(crate) consent_state: FfiConsentState,
    /// Null-terminated array of account identifier strings. Each must be freed.
    pub(crate) account_identifiers: *mut *mut c_char,
    pub(crate) account_identifiers_count: i32,
    /// Null-terminated array of hex-encoded installation ID strings. Each must be freed.
    pub(crate) installation_ids: *mut *mut c_char,
    pub(crate) installation_ids_count: i32,
}

/// A list of group members.
pub struct FfiGroupMemberList {
    pub(crate) items: Vec<FfiGroupMember>,
}

impl Drop for FfiGroupMember {
    fn drop(&mut self) {
        if !self.inbox_id.is_null() {
            drop(unsafe { CString::from_raw(self.inbox_id) });
            self.inbox_id = std::ptr::null_mut();
        }
        free_c_string_array(self.account_identifiers, self.account_identifiers_count);
        self.account_identifiers = std::ptr::null_mut();
        self.account_identifiers_count = 0;
        free_c_string_array(self.installation_ids, self.installation_ids_count);
        self.installation_ids = std::ptr::null_mut();
        self.installation_ids_count = 0;
    }
}

/// A single inbox state entry (batch query result).
#[repr(C)]
pub struct FfiInboxStateItem {
    pub(crate) inbox_id: *mut c_char,
    pub(crate) recovery_identifier: *mut c_char,
    pub(crate) identifiers: *mut *mut c_char,
    pub(crate) identifiers_count: i32,
    pub(crate) installation_ids: *mut *mut c_char,
    pub(crate) installation_ids_count: i32,
    /// Parallel array of client_timestamp_ns per installation (0 = unknown).
    pub(crate) installation_client_timestamps: *mut i64,
}

/// A list of inbox states.
pub struct FfiInboxStateList {
    pub(crate) items: Vec<FfiInboxStateItem>,
}

/// A consent record exposed to C.
#[repr(C)]
pub struct FfiConsentRecord {
    pub entity_type: FfiConsentEntityType,
    pub state: FfiConsentState,
    /// Entity identifier string.
    pub entity: *mut c_char,
}

/// A preference update exposed to C.
#[repr(C)]
pub struct FfiPreferenceUpdate {
    pub kind: FfiPreferenceUpdateKind,
    /// For Consent: the consent record. For HmacKey: zeroed.
    pub consent: FfiConsentRecord,
    /// For HmacKey: the key bytes. For Consent: null/0.
    pub hmac_key: *mut u8,
    pub hmac_key_len: i32,
}

/// Options for sending a message.
#[repr(C)]
pub struct FfiSendOpts {
    /// Whether to send a push notification. 1 = yes (default), 0 = no.
    pub should_push: i32,
}

/// MLS API call statistics (request counts).
#[repr(C)]
pub struct FfiApiStats {
    pub upload_key_package: i64,
    pub fetch_key_package: i64,
    pub send_group_messages: i64,
    pub send_welcome_messages: i64,
    pub query_group_messages: i64,
    pub query_welcome_messages: i64,
    pub subscribe_messages: i64,
    pub subscribe_welcomes: i64,
    pub publish_commit_log: i64,
    pub query_commit_log: i64,
    pub get_newest_group_message: i64,
}

/// Identity API call statistics (request counts).
#[repr(C)]
pub struct FfiIdentityStats {
    pub publish_identity_update: i64,
    pub get_identity_updates_v2: i64,
    pub get_inbox_ids: i64,
    pub verify_smart_contract_wallet_signature: i64,
}

/// A single cursor entry (originator + sequence).
#[repr(C)]
pub struct FfiCursor {
    pub originator_id: u32,
    pub sequence_id: u64,
}

/// Conversation debug info (epoch, fork status, commit logs, cursors).
#[repr(C)]
pub struct FfiConversationDebugInfo {
    pub epoch: u64,
    pub maybe_forked: i32,
    pub fork_details: *mut c_char,
    /// -1 = unknown, 0 = no, 1 = yes
    pub is_commit_log_forked: i32,
    pub local_commit_log: *mut c_char,
    pub remote_commit_log: *mut c_char,
    /// Heap-allocated cursor array. Free with the debug_info_free function.
    pub cursors: *mut FfiCursor,
    pub cursors_count: i32,
}

/// A single HMAC key (42-byte key + epoch).
#[repr(C)]
pub struct FfiHmacKey {
    pub key: *mut u8,
    pub key_len: i32,
    pub epoch: i64,
}

/// A list of HMAC keys for one conversation.
#[repr(C)]
pub struct FfiHmacKeyEntry {
    /// Hex-encoded group ID.
    pub group_id: *mut c_char,
    pub keys: *mut FfiHmacKey,
    pub keys_count: i32,
}

/// A map of conversation ID → HMAC keys.
pub struct FfiHmacKeyMap {
    pub(crate) entries: Vec<FfiHmacKeyEntry>,
}

impl Drop for FfiHmacKey {
    fn drop(&mut self) {
        if !self.key.is_null() && self.key_len > 0 {
            drop(unsafe {
                Vec::from_raw_parts(self.key, self.key_len as usize, self.key_len as usize)
            });
            self.key = std::ptr::null_mut();
            self.key_len = 0;
        }
    }
}

impl Drop for FfiHmacKeyEntry {
    fn drop(&mut self) {
        if !self.group_id.is_null() {
            drop(unsafe { CString::from_raw(self.group_id) });
            self.group_id = std::ptr::null_mut();
        }
        if !self.keys.is_null() && self.keys_count > 0 {
            drop(unsafe {
                Vec::from_raw_parts(
                    self.keys,
                    self.keys_count as usize,
                    self.keys_count as usize,
                )
            });
            self.keys = std::ptr::null_mut();
            self.keys_count = 0;
        }
    }
}

/// Options for device sync archive operations.
#[repr(C)]
pub struct FfiArchiveOptions {
    /// Bitmask of element selections: bit 0 = Messages, bit 1 = Consent.
    pub elements: i32,
    /// Start timestamp filter (ns). 0 = no filter.
    pub start_ns: i64,
    /// End timestamp filter (ns). 0 = no filter.
    pub end_ns: i64,
    /// Whether to exclude disappearing messages. 0 = include, 1 = exclude.
    pub exclude_disappearing_messages: i32,
}

/// Info about an available archive in the sync group.
#[repr(C)]
pub struct FfiAvailableArchive {
    pub pin: *mut c_char,
    pub backup_version: u16,
    pub exported_at_ns: i64,
    pub sent_by_installation: *mut u8,
    pub sent_by_installation_len: i32,
}

/// A list of available archives.
pub struct FfiAvailableArchiveList {
    pub(crate) items: Vec<FfiAvailableArchive>,
}

/// Opaque handle for gateway authentication credentials.
pub struct FfiAuthHandle {
    pub(crate) inner: xmtp_api_d14n::AuthHandle,
}

/// Key package status for an installation.
#[repr(C)]
pub struct FfiKeyPackageStatus {
    /// Installation ID as hex string (owned).
    pub installation_id: *mut c_char,
    /// 1 if valid, 0 if validation error.
    pub valid: i32,
    /// not_before timestamp (0 if unavailable).
    pub not_before: u64,
    /// not_after timestamp (0 if unavailable).
    pub not_after: u64,
    /// Validation error message (null if no error, owned).
    pub validation_error: *mut c_char,
}

/// A list of key package statuses.
pub struct FfiKeyPackageStatusList {
    pub(crate) items: Vec<FfiKeyPackageStatus>,
}

/// Inbox update count entry (inbox_id → count).
#[repr(C)]
pub struct FfiInboxUpdateCount {
    pub inbox_id: *mut c_char,
    pub count: u32,
}

/// A list of inbox update counts.
pub struct FfiInboxUpdateCountList {
    pub(crate) items: Vec<FfiInboxUpdateCount>,
}

/// Group metadata (creator + conversation type).
#[repr(C)]
pub struct FfiGroupMetadata {
    /// Creator inbox ID (owned string).
    pub creator_inbox_id: *mut c_char,
    pub conversation_type: FfiConversationType,
}

/// Permission policy set for a conversation.
#[repr(C)]
pub struct FfiPermissionPolicySet {
    pub add_member_policy: FfiPermissionPolicy,
    pub remove_member_policy: FfiPermissionPolicy,
    pub add_admin_policy: FfiPermissionPolicy,
    pub remove_admin_policy: FfiPermissionPolicy,
    pub update_group_name_policy: FfiPermissionPolicy,
    pub update_group_description_policy: FfiPermissionPolicy,
    pub update_group_image_url_square_policy: FfiPermissionPolicy,
    pub update_message_disappearing_policy: FfiPermissionPolicy,
    pub update_app_data_policy: FfiPermissionPolicy,
}

/// Group permissions (policy type + policy set).
#[repr(C)]
pub struct FfiGroupPermissions {
    pub policy_type: FfiGroupPermissionsPreset,
    pub policy_set: FfiPermissionPolicySet,
}

/// An enriched (decoded) message exposed to C.
/// Contains metadata + the original encoded content bytes for upper-layer decoding.
#[repr(C)]
pub struct FfiEnrichedMessage {
    /// Message ID (hex string, owned).
    pub id: *mut c_char,
    /// Group ID (hex string, owned).
    pub group_id: *mut c_char,
    /// Sender inbox ID (owned string).
    pub sender_inbox_id: *mut c_char,
    /// Sender installation ID (hex string, owned).
    pub sender_installation_id: *mut c_char,
    /// Sent timestamp in nanoseconds.
    pub sent_at_ns: i64,
    /// Inserted-into-DB timestamp in nanoseconds.
    pub inserted_at_ns: i64,
    pub kind: FfiMessageKind,
    pub delivery_status: FfiDeliveryStatus,
    /// Content type ID string (e.g. "xmtp.org/text:1.0", owned).
    pub content_type: *mut c_char,
    /// Fallback text (nullable, owned).
    pub fallback_text: *mut c_char,
    /// Expiration timestamp in nanoseconds (0 = no expiration).
    pub expires_at_ns: i64,
    /// Number of reactions.
    pub num_reactions: i32,
    /// Number of replies.
    pub num_replies: i32,
    /// Raw decrypted content bytes (protobuf-encoded EncodedContent, owned).
    pub content_bytes: *mut u8,
    /// Length of `content_bytes`. 0 if unavailable.
    pub content_bytes_len: i32,
}

/// A list of enriched messages.
pub struct FfiEnrichedMessageList {
    pub(crate) items: Vec<FfiEnrichedMessage>,
}

impl Drop for FfiEnrichedMessage {
    fn drop(&mut self) {
        for p in [
            &mut self.id,
            &mut self.group_id,
            &mut self.sender_inbox_id,
            &mut self.sender_installation_id,
            &mut self.content_type,
            &mut self.fallback_text,
        ] {
            if !p.is_null() {
                drop(unsafe { CString::from_raw(*p) });
                *p = std::ptr::null_mut();
            }
        }
        if !self.content_bytes.is_null() && self.content_bytes_len > 0 {
            drop(unsafe {
                Vec::from_raw_parts(
                    self.content_bytes,
                    self.content_bytes_len as usize,
                    self.content_bytes_len as usize,
                )
            });
            self.content_bytes = std::ptr::null_mut();
            self.content_bytes_len = 0;
        }
    }
}

/// Last-read-time entry (inbox_id → timestamp_ns).
#[repr(C)]
pub struct FfiLastReadTimeEntry {
    pub inbox_id: *mut c_char,
    pub timestamp_ns: i64,
}

/// A list of last-read-time entries.
pub struct FfiLastReadTimeList {
    pub(crate) items: Vec<FfiLastReadTimeEntry>,
}

// ---------------------------------------------------------------------------
// Thread-local error
//
// # Thread Safety
//
// Error state is **thread-local** (similar to POSIX `errno`).  The caller
// must read the error on the **same OS thread** that made the failing FFI
// call.  Reading from a different thread will see that thread's independent
// (and likely empty) error state.
// ---------------------------------------------------------------------------

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Store an error message for later retrieval.
pub(crate) fn set_last_error(msg: impl Into<String>) {
    LAST_ERROR.with(|e| *e.borrow_mut() = msg.into());
}

/// Get the length of the last error message (including NUL terminator).
/// Returns 0 if no error.
#[unsafe(no_mangle)]
pub extern "C" fn xmtp_last_error_length() -> i32 {
    LAST_ERROR.with(|e| {
        let s = e.borrow();
        if s.is_empty() { 0 } else { s.len() as i32 + 1 }
    })
}

/// Copy the last error message into `buf`. Returns bytes written (excluding NUL),
/// or -1 if `buf` is null or too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_last_error_message(buf: *mut c_char, buf_len: i32) -> i32 {
    if buf.is_null() || buf_len <= 0 {
        return -1;
    }
    LAST_ERROR.with(|e| {
        let s = e.borrow();
        if s.is_empty() {
            unsafe {
                *buf = 0;
            }
            return 0;
        }
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min((buf_len - 1) as usize);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), copy_len);
            *buf.add(copy_len) = 0;
        }
        copy_len as i32
    })
}

// ---------------------------------------------------------------------------
// Error-catching wrapper
// ---------------------------------------------------------------------------

/// Execute a closure with panic safety, set thread-local error on failure, return code.
/// Catches both `Err` results and unwinding panics to prevent UB at the FFI boundary.
pub(crate) fn catch<F>(f: F) -> i32
where
    F: FnOnce() -> Result<(), Box<dyn std::error::Error>>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => {
            LAST_ERROR.with(|e| e.borrow_mut().clear());
            0
        }
        Ok(Err(e)) => {
            set_last_error(e.to_string());
            -1
        }
        Err(panic) => {
            let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                format!("panic: {s}")
            } else if let Some(s) = panic.downcast_ref::<String>() {
                format!("panic: {s}")
            } else {
                "panic: <unknown>".to_string()
            };
            set_last_error(msg);
            -1
        }
    }
}

/// Execute an async closure, set error on failure, return code.
///
/// Detects whether the current thread is already inside the shared tokio
/// runtime (e.g. when a C caller invokes an FFI function from within a
/// stream callback) and uses [`tokio::task::block_in_place`] to avoid a
/// nested `block_on` panic.
pub(crate) fn catch_async<F, Fut>(f: F) -> i32
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    catch(|| match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f())),
        Err(_) => runtime().block_on(f()),
    })
}

// ---------------------------------------------------------------------------
// Shared tokio runtime
// ---------------------------------------------------------------------------

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get or initialize the global tokio runtime.
pub(crate) fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Convert a C string to an owned Rust `String`. Returns `Err` on null or invalid UTF-8.
pub(crate) unsafe fn c_str_to_string(
    s: *const c_char,
) -> Result<String, Box<dyn std::error::Error>> {
    if s.is_null() {
        return Err("null string pointer".into());
    }
    Ok(unsafe { CStr::from_ptr(s) }.to_str()?.to_owned())
}

/// Convert a nullable C string to `Option<String>`.
pub(crate) unsafe fn c_str_to_option(
    s: *const c_char,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if s.is_null() {
        return Ok(None);
    }
    Ok(Some(unsafe { CStr::from_ptr(s) }.to_str()?.to_owned()))
}

/// Untrusted C buffer. Null or negative → Err. `len == 0` → Ok(&[]).
/// Never `as usize` on a negative i32.
pub(crate) fn checked_slice<'a, T>(
    ptr: *const T,
    len: i32,
) -> Result<&'a [T], Box<dyn std::error::Error>> {
    if ptr.is_null() {
        return Err("null pointer".into());
    }
    if len < 0 {
        return Err("negative length".into());
    }
    if len == 0 {
        return Ok(&[]);
    }
    // SAFETY: null/negative rejected; caller must pass `len` valid `T`s.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len as usize) })
}

/// Required payload. Null, negative, or 0 → Err.
pub(crate) fn checked_slice_nonempty<'a, T>(
    ptr: *const T,
    len: i32,
) -> Result<&'a [T], Box<dyn std::error::Error>> {
    let slice = checked_slice(ptr, len)?;
    if slice.is_empty() {
        return Err("empty buffer".into());
    }
    Ok(slice)
}

/// 32-byte DB/archive key. Null or `len != 32` → Err.
pub(crate) fn checked_key32<'a>(
    ptr: *const u8,
    len: i32,
) -> Result<&'a [u8; 32], Box<dyn std::error::Error>> {
    if ptr.is_null() || len != 32 {
        return Err("key must be 32 bytes".into());
    }
    let slice = checked_slice(ptr, 32)?;
    slice.try_into().map_err(|_| "key must be 32 bytes".into())
}

/// Convert to an owned `CString`. Fails on interior NUL without leaking.
pub(crate) fn to_cstring(s: &str) -> Result<CString, Box<dyn std::error::Error>> {
    CString::new(s).map_err(|_| "string contains interior NUL".into())
}

/// Allocate a new C string from a Rust `&str`. Caller must free with [`xmtp_free_string`].
pub(crate) fn to_c_string(s: &str) -> Result<*mut c_char, Box<dyn std::error::Error>> {
    Ok(to_cstring(s)?.into_raw())
}

/// Pointer-returning FFI: record last error on interior NUL instead of swallowing it.
pub(crate) fn to_c_string_or_null(s: &str) -> *mut c_char {
    match to_c_string(s) {
        Ok(p) => p,
        Err(e) => {
            set_last_error(e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Free a string previously returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Free a byte buffer previously returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_free_bytes(ptr: *mut u8, len: i32) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len as usize, len as usize) });
    }
}

// ---------------------------------------------------------------------------
// Handle helpers
// ---------------------------------------------------------------------------

/// Validate a pointer and create a safe reference.
pub(crate) unsafe fn ref_from<'a, T>(ptr: *const T) -> Result<&'a T, Box<dyn std::error::Error>> {
    if ptr.is_null() {
        return Err("null handle".into());
    }
    Ok(unsafe { &*ptr })
}

/// Box a value and return a raw pointer.
pub(crate) fn into_raw<T>(val: T) -> *mut T {
    Box::into_raw(Box::new(val))
}

/// Write a raw pointer into an output parameter.
pub(crate) unsafe fn write_out<T>(
    out: *mut *mut T,
    val: T,
) -> Result<(), Box<dyn std::error::Error>> {
    if out.is_null() {
        return Err("null output pointer".into());
    }
    unsafe {
        *out = into_raw(val);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Identifier helpers (shared across modules)
// ---------------------------------------------------------------------------

/// Parse a single identifier from a C string + kind.
pub(crate) unsafe fn parse_identifier(
    s: *const c_char,
    kind: i32,
) -> Result<xmtp_id::associations::Identifier, Box<dyn std::error::Error>> {
    let val = unsafe { c_str_to_string(s)? };
    match kind {
        0 => Ok(xmtp_id::associations::Identifier::eth(val)?),
        1 => Ok(xmtp_id::associations::Identifier::passkey_str(&val, None)?),
        _ => Err("invalid identifier kind".into()),
    }
}

/// Collect parallel arrays of identifiers and kinds into `Vec<Identifier>`.
/// `count == 0` is an empty list (pointer may be null). Null with `count > 0` is an error.
pub(crate) unsafe fn collect_identifiers(
    ptrs: *const *const c_char,
    kinds: *const i32,
    count: i32,
) -> Result<Vec<xmtp_id::associations::Identifier>, Box<dyn std::error::Error>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let ptrs = checked_slice_nonempty(ptrs, count)?;
    let kinds = checked_slice_nonempty(kinds, count)?;
    ptrs.iter()
        .zip(kinds.iter())
        .map(|(&p, &kind)| unsafe { parse_identifier(p, kind) })
        .collect()
}

/// Collect an array of C strings into `Vec<String>`.
/// `count == 0` is an empty list (pointer may be null). Null with `count > 0` is an error.
pub(crate) unsafe fn collect_strings(
    ptrs: *const *const c_char,
    count: i32,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let ptrs = checked_slice_nonempty(ptrs, count)?;
    ptrs.iter()
        .map(|&p| unsafe { c_str_to_string(p) })
        .collect()
}

/// Convert a `Vec<String>` into a heap-allocated C string array.
/// Caller must free each string and the array itself.
pub(crate) fn string_vec_to_c(
    v: Vec<String>,
    out_count: *mut i32,
) -> Result<*mut *mut c_char, Box<dyn std::error::Error>> {
    if out_count.is_null() {
        return Err("null output pointer".into());
    }
    let owned: Vec<CString> = v.iter().map(|s| to_cstring(s)).collect::<Result<_, _>>()?;
    let count = owned.len();
    unsafe {
        *out_count = count as i32;
    }
    if owned.is_empty() {
        return Ok(std::ptr::null_mut());
    }
    let ptrs: Vec<*mut c_char> = owned.into_iter().map(CString::into_raw).collect();
    // Use into_boxed_slice to guarantee cap == len, avoiding UB in from_raw_parts
    let boxed = ptrs.into_boxed_slice();
    Ok(Box::into_raw(boxed) as *mut *mut c_char)
}

// ---------------------------------------------------------------------------
// Array free helpers
// ---------------------------------------------------------------------------

/// Free a heap-allocated array of C strings.
pub(crate) fn free_c_string_array(arr: *mut *mut c_char, count: i32) {
    if arr.is_null() || count <= 0 {
        return;
    }
    for i in 0..count as usize {
        let s = unsafe { *arr.add(i) };
        if !s.is_null() {
            drop(unsafe { CString::from_raw(s) });
        }
    }
    drop(unsafe { Vec::from_raw_parts(arr, count as usize, count as usize) });
}

/// Free a string array returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_free_string_array(arr: *mut *mut c_char, count: i32) {
    free_c_string_array(arr, count);
}

// ---------------------------------------------------------------------------
// DRY macros for FFI list types
// ---------------------------------------------------------------------------

/// Generate a `_len` function for a list type with an `items` field.
macro_rules! ffi_list_len {
    ($fn_name:ident, $list_ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(list: *const $list_ty) -> i32 {
            match unsafe { $crate::ffi::ref_from(list) } {
                Ok(l) => l.items.len().min(i32::MAX as usize) as i32,
                Err(_) => 0,
            }
        }
    };
}

/// Generate a `_get` function returning `*const Item` by index.
macro_rules! ffi_list_get {
    ($fn_name:ident, $list_ty:ty, $item_ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(list: *const $list_ty, index: i32) -> *const $item_ty {
            match unsafe { $crate::ffi::ref_from(list) } {
                Ok(l) => l
                    .items
                    .get(index as usize)
                    .map_or(std::ptr::null(), |item| item as *const _),
                Err(_) => std::ptr::null(),
            }
        }
    };
}

/// Generate a `_free` function for a boxed type (no inner cleanup needed).
macro_rules! free_opaque {
    ($fn_name:ident, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(ptr: *mut $ty) {
            if !ptr.is_null() {
                drop(unsafe { Box::from_raw(ptr) });
            }
        }
    };
}

/// Free multiple nullable `*mut c_char` fields on a struct expression.
macro_rules! free_c_strings {
    ($item:expr, $($field:ident),+ $(,)?) => {
        $(
            if !$item.$field.is_null() {
                drop(unsafe { std::ffi::CString::from_raw($item.$field) });
            }
        )+
    };
}

/// Generate a free function for a list type whose items only contain owned C strings.
macro_rules! ffi_list_free {
    ($fn_name:ident, $list_ty:ty, [$($field:ident),+ $(,)?]) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(list: *mut $list_ty) {
            if list.is_null() {
                return;
            }
            let l = unsafe { Box::from_raw(list) };
            for item in &l.items {
                $crate::ffi::free_c_strings!(item, $($field),+);
            }
        }
    };
}

/// Generate a conversation string property getter (returns `*mut c_char`).
/// Inner method must return `Result<String, _>`.
macro_rules! conv_string_getter {
    ($fn_name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            conv: *const $crate::ffi::FfiConversation,
        ) -> *mut std::ffi::c_char {
            match unsafe { $crate::ffi::ref_from(conv) } {
                Ok(c) => match c.inner.$method() {
                    Ok(s) => $crate::ffi::to_c_string_or_null(&s),
                    Err(_) => std::ptr::null_mut(),
                },
                Err(_) => std::ptr::null_mut(),
            }
        }
    };
}

/// Generate an async conversation string property setter (returns `i32`).
/// Inner method must be `async fn(&self, String) -> Result<(), _>`.
macro_rules! conv_string_setter {
    ($fn_name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            conv: *const $crate::ffi::FfiConversation,
            value: *const std::ffi::c_char,
        ) -> i32 {
            $crate::ffi::catch_async(|| async {
                let c = unsafe { $crate::ffi::ref_from(conv)? };
                let v = unsafe { $crate::ffi::c_str_to_string(value)? };
                c.inner.$method(v).await?;
                Ok(())
            })
        }
    };
}

/// Generate a conversation `Vec<String>` property getter (admin lists, etc.).
/// Inner method must return `Result<Vec<String>, _>`.
macro_rules! conv_string_list_getter {
    ($fn_name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            conv: *const $crate::ffi::FfiConversation,
            out_count: *mut i32,
        ) -> *mut *mut std::ffi::c_char {
            if out_count.is_null() {
                return std::ptr::null_mut();
            }
            match unsafe { $crate::ffi::ref_from(conv) } {
                Ok(c) => match c.inner.$method() {
                    Ok(list) => match $crate::ffi::string_vec_to_c(list, out_count) {
                        Ok(p) => p,
                        Err(e) => {
                            $crate::ffi::set_last_error(e.to_string());
                            unsafe { *out_count = 0 };
                            std::ptr::null_mut()
                        }
                    },
                    Err(_) => {
                        unsafe { *out_count = 0 };
                        std::ptr::null_mut()
                    }
                },
                Err(_) => {
                    unsafe { *out_count = 0 };
                    std::ptr::null_mut()
                }
            }
        }
    };
}

/// Generate a conversation inbox-ID-in-list check (is_admin / is_super_admin).
/// Inner method must return `Result<Vec<String>, _>`.
macro_rules! conv_inbox_check {
    ($fn_name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            conv: *const $crate::ffi::FfiConversation,
            inbox_id: *const std::ffi::c_char,
        ) -> i32 {
            let c = match unsafe { $crate::ffi::ref_from(conv) } {
                Ok(c) => c,
                Err(_) => return -1,
            };
            let id = match unsafe { $crate::ffi::c_str_to_string(inbox_id) } {
                Ok(s) => s,
                Err(_) => return -1,
            };
            match c.inner.$method() {
                Ok(list) => i32::from(list.contains(&id)),
                Err(_) => -1,
            }
        }
    };
}

pub(crate) use conv_inbox_check;
pub(crate) use conv_string_getter;
pub(crate) use conv_string_list_getter;
pub(crate) use conv_string_setter;
pub(crate) use ffi_list_free;
pub(crate) use ffi_list_get;
pub(crate) use ffi_list_len;
pub(crate) use free_c_strings;
pub(crate) use free_opaque;

// ---------------------------------------------------------------------------
// Enum mapping helpers (shared across modules)
// ---------------------------------------------------------------------------

/// Map `ConsentState` → `FfiConsentState`.
pub(crate) fn consent_state_to_ffi(s: xmtp_db::consent_record::ConsentState) -> FfiConsentState {
    match s {
        xmtp_db::consent_record::ConsentState::Unknown => FfiConsentState::Unknown,
        xmtp_db::consent_record::ConsentState::Allowed => FfiConsentState::Allowed,
        xmtp_db::consent_record::ConsentState::Denied => FfiConsentState::Denied,
    }
}

/// Map i32 → `ConsentState`. Returns `Err` on invalid value.
pub(crate) fn i32_to_consent_state(
    v: i32,
) -> Result<xmtp_db::consent_record::ConsentState, Box<dyn std::error::Error>> {
    match v {
        0 => Ok(xmtp_db::consent_record::ConsentState::Unknown),
        1 => Ok(xmtp_db::consent_record::ConsentState::Allowed),
        2 => Ok(xmtp_db::consent_record::ConsentState::Denied),
        _ => Err("invalid consent state".into()),
    }
}

/// Map `ConsentType` → `FfiConsentEntityType`.
pub(crate) fn consent_type_to_ffi(t: xmtp_db::consent_record::ConsentType) -> FfiConsentEntityType {
    match t {
        xmtp_db::consent_record::ConsentType::ConversationId => FfiConsentEntityType::GroupId,
        xmtp_db::consent_record::ConsentType::InboxId => FfiConsentEntityType::InboxId,
    }
}

/// Map i32 → `ConsentType`. Returns `Err` on invalid value.
pub(crate) fn i32_to_consent_type(
    v: i32,
) -> Result<xmtp_db::consent_record::ConsentType, Box<dyn std::error::Error>> {
    match v {
        0 => Ok(xmtp_db::consent_record::ConsentType::ConversationId),
        1 => Ok(xmtp_db::consent_record::ConsentType::InboxId),
        _ => Err("invalid entity type".into()),
    }
}

/// Convert a `StoredConsentRecord` to a C `FfiConsentRecord`.
pub(crate) fn consent_record_to_c(
    r: &xmtp_db::consent_record::StoredConsentRecord,
) -> FfiConsentRecord {
    FfiConsentRecord {
        entity_type: consent_type_to_ffi(r.entity_type),
        state: consent_state_to_ffi(r.state),
        entity: to_c_string_or_null(&r.entity),
    }
}

/// Map i32 → `ContentType`. Returns `None` on invalid value.
pub(crate) fn i32_to_content_type(v: i32) -> Option<xmtp_db::group_message::ContentType> {
    use xmtp_db::group_message::ContentType;
    match v {
        0 => Some(ContentType::Unknown),
        1 => Some(ContentType::Text),
        2 => Some(ContentType::GroupMembershipChange),
        3 => Some(ContentType::GroupUpdated),
        4 => Some(ContentType::Reaction),
        5 => Some(ContentType::ReadReceipt),
        6 => Some(ContentType::Reply),
        7 => Some(ContentType::Attachment),
        8 => Some(ContentType::RemoteAttachment),
        9 => Some(ContentType::TransactionReference),
        10 => Some(ContentType::WalletSendCalls),
        11 => Some(ContentType::LeaveRequest),
        12 => Some(ContentType::Markdown),
        13 => Some(ContentType::Actions),
        14 => Some(ContentType::Intent),
        15 => Some(ContentType::MultiRemoteAttachment),
        16 => Some(ContentType::DeleteMessage),
        _ => None,
    }
}

/// Parse a consent state filter from a raw int array. Shared by conversations and streams.
///
/// `null` or `count == 0` → `Ok(None)` (no filter). Negative `count` → `Err`.
pub(crate) fn parse_consent_states(
    states: *const i32,
    count: i32,
) -> Result<Option<Vec<xmtp_db::consent_record::ConsentState>>, Box<dyn std::error::Error>> {
    if states.is_null() || count == 0 {
        return Ok(None);
    }
    let slice = checked_slice(states, count)?;
    let mut result = Vec::with_capacity(slice.len());
    for &s in slice {
        result.push(match s {
            0 => xmtp_db::consent_record::ConsentState::Unknown,
            1 => xmtp_db::consent_record::ConsentState::Allowed,
            2 => xmtp_db::consent_record::ConsentState::Denied,
            _ => continue,
        });
    }
    if result.is_empty() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

/// Map `ConversationType` → `FfiConversationType`.
pub(crate) fn conversation_type_to_ffi(t: xmtp_db::group::ConversationType) -> FfiConversationType {
    match t {
        xmtp_db::group::ConversationType::Dm => FfiConversationType::Dm,
        xmtp_db::group::ConversationType::Group => FfiConversationType::Group,
        xmtp_db::group::ConversationType::Sync => FfiConversationType::Sync,
        xmtp_db::group::ConversationType::Oneshot => FfiConversationType::Oneshot,
    }
}

// ---------------------------------------------------------------------------
// Logger initialization
// ---------------------------------------------------------------------------

static LOGGER_INIT: OnceLock<()> = OnceLock::new();

/// Initialize the tracing logger. Call at most once. `level` is a C string like
/// "debug", "info", "warn", "error", or "off". Pass null for default ("info").
/// Returns 0 on success.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_init_logger(level: *const c_char) -> i32 {
    catch(|| {
        use tracing_subscriber::{EnvFilter, fmt, prelude::*};
        LOGGER_INIT.get_or_init(|| {
            let filter_str = if level.is_null() {
                "info".to_string()
            } else {
                unsafe { CStr::from_ptr(level) }
                    .to_str()
                    .unwrap_or("info")
                    .to_string()
            };
            let filter = EnvFilter::builder().parse_lossy(&filter_str);
            tracing_subscriber::registry()
                .with(fmt::layer())
                .with(filter)
                .init();
        });
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_slice_len_zero_ok_empty() {
        let p = std::ptr::NonNull::<u8>::dangling().as_ptr();
        let s = checked_slice::<u8>(p, 0).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn checked_slice_null_or_negative_err() {
        assert!(checked_slice::<u8>(std::ptr::null(), 0).is_err());
        assert!(checked_slice::<u8>(std::ptr::null(), 4).is_err());
        let buf = [1u8, 2];
        assert!(checked_slice(buf.as_ptr(), -1).is_err());
        assert_eq!(checked_slice(buf.as_ptr(), 2).unwrap(), &buf);
    }

    #[test]
    fn checked_slice_nonempty_rejects_zero() {
        let p = std::ptr::NonNull::<u8>::dangling().as_ptr();
        assert!(checked_slice_nonempty::<u8>(p, 0).is_err());
        let buf = [1u8];
        assert_eq!(checked_slice_nonempty(buf.as_ptr(), 1).unwrap(), &[1]);
    }

    #[test]
    fn checked_key32_len_not_32_err() {
        let short = [0u8; 16];
        assert!(checked_key32(short.as_ptr(), 16).is_err());
        assert!(checked_key32(std::ptr::null(), 32).is_err());
        assert!(checked_key32(short.as_ptr(), -1).is_err());
        let key = [7u8; 32];
        assert_eq!(checked_key32(key.as_ptr(), 32).unwrap(), &key);
    }

    #[test]
    fn parse_consent_states_empty_is_none_not_err() {
        assert!(parse_consent_states(std::ptr::null(), 0).unwrap().is_none());
        let empty: [i32; 0] = [];
        assert!(parse_consent_states(empty.as_ptr(), 0).unwrap().is_none());
    }

    #[test]
    fn parse_consent_states_negative_is_err() {
        let s = [1i32];
        assert!(parse_consent_states(s.as_ptr(), -1).is_err());
    }

    #[test]
    fn parse_consent_states_values() {
        let s = [1i32, 2, 99];
        let got = parse_consent_states(s.as_ptr(), 3).unwrap().unwrap();
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn to_c_string_rejects_interior_nul() {
        let p = to_c_string("ok").unwrap();
        unsafe { xmtp_free_string(p) };
        assert!(to_c_string("bad\0nul").is_err());
        assert!(to_c_string_or_null("bad\0nul").is_null());
        assert!(xmtp_last_error_length() > 0);
    }

    #[test]
    fn collect_strings_count_zero_is_empty_ok() {
        let got = unsafe { collect_strings(std::ptr::null(), 0) }.unwrap();
        assert!(got.is_empty());
        let p = std::ptr::NonNull::<*const c_char>::dangling().as_ptr();
        let got = unsafe { collect_strings(p, 0) }.unwrap();
        assert!(got.is_empty());
        assert!(unsafe { collect_strings(std::ptr::null(), 2) }.is_err());
    }
}
