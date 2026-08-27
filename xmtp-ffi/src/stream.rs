//! Callback-based streaming for conversations and messages.
//!
//! # Ownership model
//! - Conversation / message callbacks transfer ownership (`*mut`) — caller must free.
//! - Consent / preference / deletion callbacks lend data (`*const`) — valid only during callback.
//! - `on_close` receives a borrowed error string (null = normal close).
//!
//! # Lifecycle
//! `xmtp_stream_end(handle)` → signal stop.
//! `xmtp_stream_join(handle)` → wait for the worker (timeout leaks the task).
//! `xmtp_stream_free(handle)` → release handle memory; does not wait.

use std::ffi::{c_char, c_void};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use xmtp_common::StreamHandle;
use xmtp_mls::Client as MlsClient;
use xmtp_mls::groups::MlsGroup;

use crate::ffi::*;

const JOIN_TIMEOUT: Duration = Duration::from_millis(if cfg!(test) { 200 } else { 5_000 });

/// Nullable on-close callback passed to stream functions.
///
/// This MUST be written inline (not as `Option<FnOnCloseCallback>`) in every
/// `extern "C"` signature. cbindgen cannot represent `Option<TypeAlias>` where
/// the alias is a function pointer — it emits an opaque zero-sized struct
/// instead of a nullable function pointer, causing an ABI mismatch:
///
/// - **System V ABI (Linux/macOS)**: ZSTs are skipped entirely, shifting all
///   subsequent parameters (`context`, `out`) and corrupting the call.
/// - **Windows MSVC ABI**: every argument occupies an 8-byte slot regardless
///   of size, so the ZST happens to not shift parameters — but the value
///   read for `on_close` is still garbage.
///
/// Using this alias *inside Rust* (helpers, closures) is fine; only the
/// `extern "C"` boundary requires the inline form.
type OnCloseCb = Option<FnOnCloseCallback>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a conversation type filter from an int.
fn parse_conv_type(v: i32) -> Option<xmtp_db::group::ConversationType> {
    match v {
        0 => Some(xmtp_db::group::ConversationType::Dm),
        1 => Some(xmtp_db::group::ConversationType::Group),
        2 => Some(xmtp_db::group::ConversationType::Sync),
        3 => Some(xmtp_db::group::ConversationType::Oneshot),
        _ => None,
    }
}

fn take_join_task(h: &FfiStreamHandle) -> Option<FfiJoinHandle> {
    h.join
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

fn leak_join_task(task: FfiJoinHandle) {
    let _leaked = ManuallyDrop::new(task);
}

/// `true` if the wait hit `JOIN_TIMEOUT`.
fn stream_join_timed_out(task: &mut FfiJoinHandle) -> bool {
    let fut = async { tokio::time::timeout(JOIN_TIMEOUT, task.end_and_wait()).await };
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => runtime().block_on(fut),
    };
    result.is_err()
}

/// Guard ensuring `on_close` is called at most once across data-error and close paths.
type OnCloseGuard = Arc<AtomicBool>;

/// Create a fresh guard (shared between data-callback and close-callback closures).
fn new_on_close_guard() -> OnCloseGuard {
    Arc::new(AtomicBool::new(false))
}

/// Invoke the on_close callback with a null error (normal close).
/// No-op if already called.
fn invoke_on_close_ok(on_close: OnCloseCb, ctx: usize, guard: &OnCloseGuard) {
    if guard.swap(true, Ordering::AcqRel) {
        return; // already fired
    }
    if let Some(cb) = on_close {
        unsafe { cb(std::ptr::null(), ctx as *mut c_void) };
    }
}

/// Invoke the on_close callback with an error message.
/// No-op if already called.
fn invoke_on_close_err(on_close: OnCloseCb, ctx: usize, err: &str, guard: &OnCloseGuard) {
    if guard.swap(true, Ordering::AcqRel) {
        return; // already fired
    }
    if let Some(cb) = on_close {
        let c_err = std::ffi::CString::new(err).unwrap_or_default();
        unsafe { cb(c_err.as_ptr(), ctx as *mut c_void) };
    }
}

/// Finalize a stream handle: wait_for_ready, keep the joinable task, write to output.
fn finalize_stream<H>(
    mut handle: H,
    out: *mut *mut FfiStreamHandle,
) -> Result<(), Box<dyn std::error::Error>>
where
    H: StreamHandle<StreamOutput = Result<(), xmtp_mls::subscriptions::SubscribeError>>
        + Send
        + 'static,
{
    runtime().block_on(handle.wait_for_ready());
    let abort = handle.abort_handle();
    unsafe {
        write_out(
            out,
            FfiStreamHandle {
                abort: Arc::new(abort),
                join: std::sync::Mutex::new(Some(Box::new(handle))),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Stream conversations
// ---------------------------------------------------------------------------

/// Stream new conversations. Callback receives owned `*mut FfiConversation` (caller must free).
/// `on_close(error, ctx)`: null error = normal close; non-null = borrowed error string.
/// Caller must `xmtp_stream_end`, `xmtp_stream_join`, then `xmtp_stream_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_conversations(
    client: *const FfiClient,
    conversation_type: i32,
    callback: FnConversationCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let conv_type = parse_conv_type(conversation_type);
        let ctx = context as usize;
        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsClient::stream_conversations_with_callback(
            c.inner.clone(),
            conv_type,
            move |result| match result {
                Ok(group) => {
                    let ptr = into_raw(FfiConversation { inner: group });
                    unsafe { callback(ptr, ctx as *mut c_void) };
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
            false,
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream all messages
// ---------------------------------------------------------------------------

/// Stream all messages across conversations. Callback receives owned `*mut FfiMessage`.
/// `consent_states` / `consent_states_count`: optional filter (null/0 = all).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_all_messages(
    client: *const FfiClient,
    conversation_type: i32,
    consent_states: *const i32,
    consent_states_count: i32,
    callback: FnMessageCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let conv_type = parse_conv_type(conversation_type);
        let consents = parse_consent_states(consent_states, consent_states_count)?;
        let ctx = context as usize;
        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsClient::stream_all_messages_with_callback(
            c.inner.context.clone(),
            conv_type,
            consents,
            move |result| match result {
                Ok(msg) => {
                    let ptr = into_raw(FfiMessage { inner: msg });
                    unsafe { callback(ptr, ctx as *mut c_void) };
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream single conversation messages
// ---------------------------------------------------------------------------

/// Stream messages for a single conversation. Callback receives owned `*mut FfiMessage`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_conversation_stream_messages(
    conv: *const FfiConversation,
    callback: FnMessageCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(conv)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ctx = context as usize;
        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsGroup::stream_with_callback(
            c.inner.context.clone(),
            c.inner.group_id.clone(),
            move |result| match result {
                Ok(msg) => {
                    let ptr = into_raw(FfiMessage { inner: msg });
                    unsafe { callback(ptr, ctx as *mut c_void) };
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream consent updates
// ---------------------------------------------------------------------------

/// Stream consent state changes. Callback receives a borrowed array of consent records
/// (`*const FfiConsentRecord`) — valid only during the callback invocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_consent(
    client: *const FfiClient,
    callback: FnConsentCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ctx = context as usize;

        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsClient::stream_consent_with_callback(
            c.inner.clone(),
            move |result| match result {
                Ok(records) => {
                    let c_records: Vec<FfiConsentRecord> =
                        records.iter().map(consent_record_to_c).collect();
                    unsafe {
                        callback(
                            c_records.as_ptr(),
                            c_records.len() as i32,
                            ctx as *mut c_void,
                        )
                    };
                    // Free inner allocations that FfiConsentRecord doesn't Drop
                    for r in &c_records {
                        if !r.entity.is_null() {
                            drop(unsafe { std::ffi::CString::from_raw(r.entity) });
                        }
                    }
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream preference updates
// ---------------------------------------------------------------------------

/// Stream preference updates (consent changes + HMAC key rotations).
/// Callback receives a borrowed array (`*const FfiPreferenceUpdate`) — valid only during callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_preferences(
    client: *const FfiClient,
    callback: FnPreferenceCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ctx = context as usize;

        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsClient::stream_preferences_with_callback(
            c.inner.clone(),
            move |result| match result {
                Ok(updates) => {
                    use xmtp_mls::worker::device_sync::preference_sync::PreferenceUpdate;
                    let c_updates: Vec<FfiPreferenceUpdate> = updates
                        .into_iter()
                        .map(|u| match u {
                            PreferenceUpdate::Consent(r) => FfiPreferenceUpdate {
                                kind: FfiPreferenceUpdateKind::Consent,
                                consent: consent_record_to_c(&r),
                                hmac_key: std::ptr::null_mut(),
                                hmac_key_len: 0,
                            },
                            PreferenceUpdate::Hmac { key, .. } => {
                                let len = key.len() as i32;
                                let boxed = key.into_boxed_slice();
                                let ptr = Box::into_raw(boxed) as *mut u8;
                                FfiPreferenceUpdate {
                                    kind: FfiPreferenceUpdateKind::HmacKey,
                                    consent: FfiConsentRecord {
                                        entity_type: FfiConsentEntityType::GroupId,
                                        state: FfiConsentState::Unknown,
                                        entity: std::ptr::null_mut(),
                                    },
                                    hmac_key: ptr,
                                    hmac_key_len: len,
                                }
                            }
                        })
                        .collect();
                    unsafe {
                        callback(
                            c_updates.as_ptr(),
                            c_updates.len() as i32,
                            ctx as *mut c_void,
                        )
                    };
                    // Free inner allocations that FfiPreferenceUpdate doesn't Drop
                    for u in &c_updates {
                        if !u.consent.entity.is_null() {
                            drop(unsafe { std::ffi::CString::from_raw(u.consent.entity) });
                        }
                        if !u.hmac_key.is_null() && u.hmac_key_len > 0 {
                            drop(unsafe {
                                Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                                    u.hmac_key,
                                    u.hmac_key_len as usize,
                                ))
                            });
                        }
                    }
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream message deletions
// ---------------------------------------------------------------------------

/// Stream message deletion events. Callback receives a borrowed hex message ID
/// (`*const c_char`) — valid only during the callback invocation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_message_deletions(
    client: *const FfiClient,
    callback: FnMessageDeletionCallback,
    on_close: Option<unsafe extern "C" fn(*const c_char, *mut c_void)>,
    context: *mut c_void,
    out: *mut *mut FfiStreamHandle,
) -> i32 {
    catch(|| {
        let _rt = runtime().enter();
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ctx = context as usize;

        let guard = new_on_close_guard();
        let g1 = guard.clone();
        let g2 = guard;

        let handle = MlsClient::stream_message_deletions_with_callback(
            c.inner.clone(),
            move |result| match result {
                Ok(decoded) => {
                    let id_hex = hex::encode(&decoded.metadata.id);
                    let c_str = std::ffi::CString::new(id_hex).unwrap_or_default();
                    unsafe { callback(c_str.as_ptr(), ctx as *mut c_void) };
                }
                Err(e) => invoke_on_close_err(on_close, ctx, &e.to_string(), &g1),
            },
            move || invoke_on_close_ok(on_close, ctx, &g2),
        );
        finalize_stream(handle, out)
    })
}

// ---------------------------------------------------------------------------
// Stream lifecycle
// ---------------------------------------------------------------------------

/// Signal a stream to stop. Does NOT free the handle — call `xmtp_stream_free` afterwards.
/// Safe to call multiple times.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_end(handle: *const FfiStreamHandle) {
    if let Ok(h) = unsafe { ref_from(handle) } {
        h.abort.end();
    }
}

/// Check if a stream has finished. Returns 1 if closed, 0 if active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_is_closed(handle: *const FfiStreamHandle) -> i32 {
    match unsafe { ref_from(handle) } {
        Ok(h) => i32::from(h.abort.is_finished()),
        Err(_) => 1,
    }
}

/// Wait for the stream worker. Does not free the handle.
///
/// Takes the joinable task out of `handle`. Returns 0 if the worker finished
/// (or was already joined). On timeout, leaks the remaining task so callbacks
/// may still run; does not free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_join(handle: *mut FfiStreamHandle) -> i32 {
    catch(|| {
        let h = unsafe { mut_from(handle)? };
        let Some(mut task) = take_join_task(h) else {
            return Ok(());
        };
        if stream_join_timed_out(&mut task) {
            leak_join_task(task);
            return Err("stream join timed out".into());
        }
        Ok(())
    })
}

/// Free a stream handle. Must be called after `xmtp_stream_end`.
/// Calling this on an active (non-ended) stream will also end it.
/// Does not wait. If join was skipped, the remaining task is leaked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_stream_free(handle: *mut FfiStreamHandle) {
    if handle.is_null() {
        return;
    }
    let h = unsafe { Box::from_raw(handle) };
    h.abort.end();
    if let Some(task) = take_join_task(&h) {
        leak_join_task(task);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use xmtp_common::{AbortHandle, StreamHandle, StreamHandleError};

    use super::*;

    type Out = Result<(), xmtp_mls::subscriptions::SubscribeError>;

    struct DummyAbort {
        finished: bool,
    }

    impl AbortHandle for DummyAbort {
        fn end(&self) {}
        fn is_finished(&self) -> bool {
            self.finished
        }
    }

    struct SleepHandle {
        delay: Duration,
        done: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl StreamHandle for SleepHandle {
        type StreamOutput = Out;
        async fn wait_for_ready(&mut self) {}
        fn end(&self) {}
        async fn join(self) -> Result<Self::StreamOutput, StreamHandleError> {
            tokio::time::sleep(self.delay).await;
            self.done.store(true, Ordering::SeqCst);
            Ok(Ok(()))
        }
        async fn end_and_wait(&mut self) -> Result<Self::StreamOutput, StreamHandleError> {
            tokio::time::sleep(self.delay).await;
            self.done.store(true, Ordering::SeqCst);
            Ok(Ok(()))
        }
        fn abort_handle(&self) -> Box<dyn AbortHandle> {
            Box::new(DummyAbort { finished: false })
        }
    }

    struct HangHandle;

    #[async_trait::async_trait]
    impl StreamHandle for HangHandle {
        type StreamOutput = Out;
        async fn wait_for_ready(&mut self) {}
        fn end(&self) {}
        async fn join(self) -> Result<Self::StreamOutput, StreamHandleError> {
            std::future::pending().await
        }
        async fn end_and_wait(&mut self) -> Result<Self::StreamOutput, StreamHandleError> {
            std::future::pending().await
        }
        fn abort_handle(&self) -> Box<dyn AbortHandle> {
            Box::new(DummyAbort { finished: false })
        }
    }

    struct FlagThenHang {
        flag: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl StreamHandle for FlagThenHang {
        type StreamOutput = Out;
        async fn wait_for_ready(&mut self) {}
        fn end(&self) {}
        async fn join(self) -> Result<Self::StreamOutput, StreamHandleError> {
            tokio::time::sleep(Duration::from_millis(10)).await;
            self.flag.store(true, Ordering::SeqCst);
            std::future::pending().await
        }
        async fn end_and_wait(&mut self) -> Result<Self::StreamOutput, StreamHandleError> {
            tokio::time::sleep(Duration::from_millis(10)).await;
            self.flag.store(true, Ordering::SeqCst);
            std::future::pending().await
        }
        fn abort_handle(&self) -> Box<dyn AbortHandle> {
            Box::new(DummyAbort { finished: false })
        }
    }

    fn wrap(task: FfiJoinHandle) -> *mut FfiStreamHandle {
        into_raw(FfiStreamHandle {
            abort: Arc::new(Box::new(DummyAbort { finished: false })),
            join: Mutex::new(Some(task)),
        })
    }

    #[test]
    fn parse_conv_type_oneshot() {
        assert_eq!(
            parse_conv_type(3),
            Some(xmtp_db::group::ConversationType::Oneshot)
        );
        assert_eq!(
            parse_conv_type(0),
            Some(xmtp_db::group::ConversationType::Dm)
        );
        assert!(parse_conv_type(99).is_none());
    }

    #[test]
    fn stream_join_null_is_error() {
        let rc = unsafe { xmtp_stream_join(std::ptr::null_mut()) };
        assert_eq!(rc, -1);
        assert!(xmtp_last_error_length() > 0);
    }

    #[test]
    fn stream_join_already_taken_ok() {
        let ptr = into_raw(FfiStreamHandle {
            abort: Arc::new(Box::new(DummyAbort { finished: true })),
            join: Mutex::new(None),
        });
        let rc = unsafe { xmtp_stream_join(ptr) };
        assert_eq!(rc, 0);
        unsafe { xmtp_stream_free(ptr) };
    }

    #[test]
    fn stream_join_waits_for_callback() {
        let done = Arc::new(AtomicBool::new(false));
        let ptr = wrap(Box::new(SleepHandle {
            delay: Duration::from_millis(50),
            done: done.clone(),
        }));
        unsafe { xmtp_stream_end(ptr) };
        let rc = unsafe { xmtp_stream_join(ptr) };
        assert_eq!(rc, 0);
        assert!(done.load(Ordering::SeqCst));
        unsafe { xmtp_stream_free(ptr) };
    }

    #[test]
    fn stream_join_timeout_leaks_and_callback_still_runs() {
        let flag = Arc::new(AtomicBool::new(false));
        let ptr = wrap(Box::new(FlagThenHang { flag: flag.clone() }));
        let rc = unsafe { xmtp_stream_join(ptr) };
        assert_eq!(rc, -1);
        assert!(xmtp_last_error_length() > 0);
        let start = Instant::now();
        while !flag.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(1) {
            std::thread::yield_now();
        }
        assert!(flag.load(Ordering::SeqCst));
        unsafe { xmtp_stream_free(ptr) };
    }

    #[test]
    fn stream_free_does_not_wait() {
        let ptr = wrap(Box::new(HangHandle));
        let start = Instant::now();
        unsafe { xmtp_stream_free(ptr) };
        assert!(start.elapsed() < Duration::from_millis(100));
    }
}
