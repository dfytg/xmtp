#![allow(
    unsafe_code,
    reason = "Streaming callbacks require unsafe for FFI trampoline functions and raw pointer casts"
)]
//! Channel-based streaming for real-time event subscriptions.
//!
//! Each function returns a [`Subscription<T>`] that yields typed events via
//! an internal channel. Implements [`Iterator`] for idiomatic consumption.
//! The stream stops when the subscription is dropped.

use std::ffi::{CStr, c_void};
use std::sync::mpsc;
use std::{fmt, ptr};

use crate::client::Client;
use crate::conversation::Conversation;
use crate::error::{self, Result};
use crate::ffi::{OwnedHandle, to_ffi_len};
use crate::types::{ConsentEntityType, ConsentState, ConversationType, PreferenceKind};

/// A real-time event subscription backed by an internal channel.
///
/// Yields events of type `T` via [`recv`](Self::recv),
/// [`try_recv`](Self::try_recv), or [`Iterator`] consumption.
/// The underlying FFI stream is stopped when this value is dropped.
pub struct Subscription<T> {
    // Declared first so it is dropped last (after the stream handle).
    _ctx: Option<Box<dyn std::any::Any + Send>>,
    rx: mpsc::Receiver<T>,
    handle: OwnedHandle<xmtp_sys::XmtpFfiStreamHandle>,
}

impl<T> Subscription<T> {
    /// Block until the next event, or `None` if the stream ended.
    #[must_use]
    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok()
    }

    /// Non-blocking receive. Returns `None` if no event is ready.
    #[must_use]
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Signal the stream to stop. Safe to call multiple times.
    pub fn close(&self) {
        // SAFETY: `self.handle` is a valid stream handle; safe to call multiple times.
        unsafe { xmtp_sys::xmtp_stream_end(self.handle.as_ptr()) };
    }

    /// Whether the stream has finished.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        // SAFETY: `self.handle` is a valid stream handle.
        unsafe { xmtp_sys::xmtp_stream_is_closed(self.handle.as_ptr()) == 1 }
    }
}

impl<T> Iterator for Subscription<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.recv().ok()
    }
}

impl<T> Drop for Subscription<T> {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a valid stream handle; safe to call multiple times.
        unsafe { xmtp_sys::xmtp_stream_end(self.handle.as_ptr()) };

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.is_closed() && std::time::Instant::now() < deadline {
            #[allow(
                clippy::disallowed_methods,
                reason = "sync Drop cannot await; bounded poll before leak-or-free"
            )]
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        if !self.is_closed() {
            // `_ctx` is named for drop-order; take it so a live worker cannot UAF.
            #[allow(
                clippy::used_underscore_binding,
                reason = "field is owned for lifetime; take on timeout to leak"
            )]
            if let Some(ctx) = self._ctx.take() {
                let _leaked = std::mem::ManuallyDrop::new(ctx);
            }
        }
    }
}

impl<T> fmt::Debug for Subscription<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Subscription")
            .field("is_closed", &self.is_closed())
            .finish()
    }
}

/// A new-message event from a message stream.
#[derive(Debug, Clone)]
pub struct MessageEvent {
    /// Hex-encoded message ID.
    pub message_id: String,
    /// Hex-encoded conversation (group) ID.
    pub conversation_id: String,
}

/// A consent state change event.
#[derive(Debug, Clone)]
pub struct ConsentUpdate {
    /// Entity type (group ID or inbox ID).
    pub entity_type: ConsentEntityType,
    /// The consent state.
    pub state: ConsentState,
    /// The entity identifier.
    pub entity: String,
}

/// A user preference update event.
#[derive(Debug, Clone)]
pub struct PreferenceUpdate {
    /// The kind of preference change.
    pub kind: PreferenceKind,
    /// For Consent updates: the consent change details.
    pub consent: Option<ConsentUpdate>,
}

/// Start an FFI stream and wire its callback to a channel receiver.
///
/// The callback `F` is a pre-erased trait object (`Box<dyn Fn(…)>`) whose
/// raw pointer is passed to the FFI trampoline. The corresponding trampoline
/// casts the context back to the same type, reading the fat pointer correctly.
fn subscribe<T: Send + 'static, F: Send + 'static>(
    callback: F,
    rx: mpsc::Receiver<T>,
    start: impl FnOnce(*mut c_void, *mut *mut xmtp_sys::XmtpFfiStreamHandle) -> i32,
) -> Result<Subscription<T>> {
    let boxed = Box::new(callback);
    let ctx_ptr = Box::into_raw(boxed).cast::<c_void>();
    let mut out: *mut xmtp_sys::XmtpFfiStreamHandle = ptr::null_mut();
    let rc = start(ctx_ptr, &raw mut out);
    if rc != 0 {
        // Reclaim the context to avoid a leak on error.
        // SAFETY: Reclaiming the context box to avoid a leak; `ctx_ptr` was created via `Box::into_raw`.
        let _ = unsafe { Box::from_raw(ctx_ptr.cast::<F>()) };
        return Err(error::last_ffi_error());
    }
    let handle = OwnedHandle::new(out, xmtp_sys::xmtp_stream_free)?;
    // SAFETY: `ctx_ptr` was created via `Box::into_raw` and the FFI layer no longer owns it.
    let ctx_box = unsafe { Box::from_raw(ctx_ptr.cast::<F>()) };
    Ok(Subscription {
        _ctx: Some(ctx_box),
        rx,
        handle,
    })
}

/// Stream new conversations.
///
/// Pass `None` for `conversation_type` to receive all types.
pub fn conversations(
    client: &Client,
    conversation_type: Option<ConversationType>,
) -> Result<Subscription<Conversation>> {
    let (tx, rx) = mpsc::channel();
    let client_ptr = client.handle.as_ptr();
    let conv_type = conversation_type.map_or(-1, |t| t as i32);
    let cb: Box<dyn Fn(Conversation) + Send> = Box::new(move |conv| {
        drop(tx.send(conv));
    });
    // SAFETY: Valid client pointer and callback context; `out` receives the stream handle.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_stream_conversations(
            client_ptr,
            conv_type,
            Some(conv_trampoline),
            None,
            ctx,
            out,
        )
    })
}

/// Stream all messages across conversations.
///
/// Pass `None` for `conversation_type` to receive from all types.
pub fn messages(
    client: &Client,
    conversation_type: Option<ConversationType>,
    consent_states: &[ConsentState],
) -> Result<Subscription<MessageEvent>> {
    let (tx, rx) = mpsc::channel();
    let client_ptr = client.handle.as_ptr();
    let conv_type = conversation_type.map_or(-1, |t| t as i32);
    let cs: Vec<i32> = consent_states.iter().map(|s| *s as i32).collect();
    let cs_ptr = if cs.is_empty() {
        ptr::null()
    } else {
        cs.as_ptr()
    };
    let cs_len = to_ffi_len(cs.len())?;
    let cb: Box<dyn Fn(String, String) + Send> = Box::new(move |mid, cid| {
        drop(tx.send(MessageEvent {
            message_id: mid,
            conversation_id: cid,
        }));
    });
    // SAFETY: Valid client pointer, consent arrays, and callback context.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_stream_all_messages(
            client_ptr,
            conv_type,
            cs_ptr,
            cs_len,
            Some(msg_trampoline),
            None,
            ctx,
            out,
        )
    })
}

/// Stream messages for a single conversation.
pub fn conversation_messages(conversation: &Conversation) -> Result<Subscription<MessageEvent>> {
    let (tx, rx) = mpsc::channel();
    let conv_ptr = conversation.handle_ptr();
    let cb: Box<dyn Fn(String, String) + Send> = Box::new(move |mid, cid| {
        drop(tx.send(MessageEvent {
            message_id: mid,
            conversation_id: cid,
        }));
    });
    // SAFETY: Valid conversation pointer and callback context.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_conversation_stream_messages(conv_ptr, Some(msg_trampoline), None, ctx, out)
    })
}

/// Stream consent state changes.
pub fn consent(client: &Client) -> Result<Subscription<Vec<ConsentUpdate>>> {
    let (tx, rx) = mpsc::channel();
    let client_ptr = client.handle.as_ptr();
    let cb: Box<dyn Fn(Vec<ConsentUpdate>) + Send> = Box::new(move |updates| {
        drop(tx.send(updates));
    });
    // SAFETY: Valid client pointer and callback context.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_stream_consent(client_ptr, Some(consent_trampoline), None, ctx, out)
    })
}

/// Stream preference updates.
pub fn preferences(client: &Client) -> Result<Subscription<Vec<PreferenceUpdate>>> {
    let (tx, rx) = mpsc::channel();
    let client_ptr = client.handle.as_ptr();
    let cb: Box<dyn Fn(Vec<PreferenceUpdate>) + Send> = Box::new(move |updates| {
        drop(tx.send(updates));
    });
    // SAFETY: Valid client pointer and callback context.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_stream_preferences(client_ptr, Some(pref_trampoline), None, ctx, out)
    })
}

/// Stream message deletion events. Each event yields the hex message ID.
pub fn message_deletions(client: &Client) -> Result<Subscription<String>> {
    let (tx, rx) = mpsc::channel();
    let client_ptr = client.handle.as_ptr();
    let cb: Box<dyn Fn(String) + Send> = Box::new(move |id| {
        drop(tx.send(id));
    });
    // SAFETY: Valid client pointer and callback context.
    subscribe(cb, rx, |ctx, out| unsafe {
        xmtp_sys::xmtp_stream_message_deletions(
            client_ptr,
            Some(deletion_trampoline),
            None,
            ctx,
            out,
        )
    })
}

unsafe extern "C" fn conv_trampoline(
    conv: *mut xmtp_sys::XmtpFfiConversation,
    context: *mut c_void,
) {
    if context.is_null() || conv.is_null() {
        return;
    }
    // SAFETY: `context` is a `Box<Box<dyn Fn(Conversation) + Send>>` created by `subscribe`.
    let cb = unsafe { &*context.cast::<Box<dyn Fn(Conversation) + Send>>() };
    if let Ok(c) = Conversation::from_raw(conv) {
        cb(c);
    }
}

unsafe extern "C" fn msg_trampoline(msg: *mut xmtp_sys::XmtpFfiMessage, context: *mut c_void) {
    if context.is_null() || msg.is_null() {
        if !msg.is_null() {
            // SAFETY: `msg` is an FFI-allocated message that must be freed.
            unsafe { xmtp_sys::xmtp_message_free(msg) };
        }
        return;
    }
    // SAFETY: Extract message ID before freeing.
    let id_ptr = unsafe { xmtp_sys::xmtp_single_message_id(msg) };
    // SAFETY: Extract group ID before freeing.
    let gid_ptr = unsafe { xmtp_sys::xmtp_single_message_group_id(msg) };
    // SAFETY: `msg` is an FFI-allocated message that must be freed.
    unsafe { xmtp_sys::xmtp_message_free(msg) };

    // SAFETY: `context` is a `Box<Box<dyn Fn(String, String) + Send>>` created by `subscribe`.
    let cb = unsafe { &*context.cast::<Box<dyn Fn(String, String) + Send>>() };
    let id = if id_ptr.is_null() {
        String::new()
    } else {
        // SAFETY: `id_ptr` is a valid NUL-terminated C string.
        let s = unsafe { CStr::from_ptr(id_ptr) }
            .to_str()
            .unwrap_or_default()
            .to_owned();
        // SAFETY: `id_ptr` was allocated by the FFI layer.
        unsafe { xmtp_sys::xmtp_free_string(id_ptr) };
        s
    };
    let gid = if gid_ptr.is_null() {
        String::new()
    } else {
        // SAFETY: `gid_ptr` is a valid NUL-terminated C string.
        let s = unsafe { CStr::from_ptr(gid_ptr) }
            .to_str()
            .unwrap_or_default()
            .to_owned();
        // SAFETY: `gid_ptr` was allocated by the FFI layer.
        unsafe { xmtp_sys::xmtp_free_string(gid_ptr) };
        s
    };
    cb(id, gid);
}

unsafe extern "C" fn consent_trampoline(
    records: *const xmtp_sys::XmtpFfiConsentRecord,
    count: i32,
    context: *mut c_void,
) {
    if context.is_null() || records.is_null() || count <= 0 {
        return;
    }
    // SAFETY: `context` is a `Box<Box<dyn Fn(Vec<ConsentUpdate>) + Send>>` created by `subscribe`.
    let cb = unsafe { &*context.cast::<Box<dyn Fn(Vec<ConsentUpdate>) + Send>>() };
    // SAFETY: `records` points to `count` valid consent records.
    let slice = unsafe { std::slice::from_raw_parts(records, count.unsigned_abs() as usize) };
    let updates: Vec<ConsentUpdate> = slice
        .iter()
        .filter_map(|r| {
            let entity_type = ConsentEntityType::from_ffi(r.entity_type as i32)?;
            let state = ConsentState::from_ffi(r.state as i32)?;
            // SAFETY: `r.entity` is a valid NUL-terminated C string.
            let entity = unsafe { CStr::from_ptr(r.entity) }
                .to_str()
                .ok()?
                .to_owned();
            Some(ConsentUpdate {
                entity_type,
                state,
                entity,
            })
        })
        .collect();
    if !updates.is_empty() {
        cb(updates);
    }
}

unsafe extern "C" fn pref_trampoline(
    updates: *const xmtp_sys::XmtpFfiPreferenceUpdate,
    count: i32,
    context: *mut c_void,
) {
    if context.is_null() || updates.is_null() || count <= 0 {
        return;
    }
    // SAFETY: `context` is a `Box<Box<dyn Fn(Vec<PreferenceUpdate>) + Send>>` created by `subscribe`.
    let cb = unsafe { &*context.cast::<Box<dyn Fn(Vec<PreferenceUpdate>) + Send>>() };
    // SAFETY: `updates` points to `count` valid preference records.
    let slice = unsafe { std::slice::from_raw_parts(updates, count.unsigned_abs() as usize) };
    let items: Vec<PreferenceUpdate> = slice
        .iter()
        .filter_map(|u| {
            let kind = PreferenceKind::from_ffi(u.kind as i32)?;
            let consent = if kind == PreferenceKind::Consent {
                let r = &u.consent;
                let et = ConsentEntityType::from_ffi(r.entity_type as i32);
                let st = ConsentState::from_ffi(r.state as i32);
                let entity = if r.entity.is_null() {
                    String::new()
                } else {
                    // SAFETY: `r.entity` is a valid NUL-terminated C string.
                    unsafe { CStr::from_ptr(r.entity) }
                        .to_str()
                        .unwrap_or_default()
                        .to_owned()
                };
                et.zip(st).map(|(entity_type, state)| ConsentUpdate {
                    entity_type,
                    state,
                    entity,
                })
            } else {
                None
            };
            Some(PreferenceUpdate { kind, consent })
        })
        .collect();
    if !items.is_empty() {
        cb(items);
    }
}

unsafe extern "C" fn deletion_trampoline(
    message_id: *const std::ffi::c_char,
    context: *mut c_void,
) {
    if context.is_null() || message_id.is_null() {
        return;
    }
    // SAFETY: `context` is a `Box<Box<dyn Fn(String) + Send>>` created by `subscribe`.
    let cb = unsafe { &*context.cast::<Box<dyn Fn(String) + Send>>() };
    // SAFETY: `message_id` is a valid NUL-terminated C string.
    if let Ok(id) = unsafe { CStr::from_ptr(message_id) }.to_str() {
        cb(id.to_owned());
    }
}
