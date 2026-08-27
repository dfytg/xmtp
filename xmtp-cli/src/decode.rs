//! Message content decoding and display utilities.
//!
//! Shared between the TUI renderer, the worker thread, and the agent
//! subcommands — every consumer of [`xmtp::Message`] content goes through
//! this module instead of duplicating decode logic.

use serde_json::{Value, json};
use xmtp::content::Content;
use xmtp::{DeliveryStatus, Message, MessageKind};

// ── Text extraction ───────────────────────────────────────────────

/// Extract text from an inner [`EncodedContent`](xmtp::content::EncodedContent)
/// (e.g. a reply body).
pub(crate) fn reply_text(ec: &xmtp::content::EncodedContent) -> String {
    let is_text = ec
        .r#type
        .as_ref()
        .is_some_and(|t| t.type_id == "text" || t.type_id == "markdown");
    if is_text {
        String::from_utf8(ec.content.clone()).unwrap_or_default()
    } else {
        ec.fallback.clone().unwrap_or_else(|| "[reply]".into())
    }
}

// ── String truncation ─────────────────────────────────────────────

/// Truncate a string, appending `…` if it exceeds `max` characters.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

/// Truncate an identifier for display (e.g. `0x1a2b…c3d4`).
pub(crate) fn truncate_id(id: &str, max: usize) -> String {
    if id.len() <= max {
        id.to_owned()
    } else {
        let half = max.saturating_sub(1) / 2;
        format!("{}…{}", &id[..half], &id[id.len() - half..])
    }
}

// ── TUI display ───────────────────────────────────────────────────

/// Decode a message to a short preview string for the sidebar.
pub(crate) fn preview(msg: &Message) -> String {
    if msg.kind != MessageKind::Application {
        return String::new();
    }
    match msg.decode() {
        Ok(Content::Text(s) | Content::Markdown(s)) => truncate(&s, 28),
        Ok(Content::Reaction(r)) => truncate(&r.content, 28),
        Ok(Content::ReadReceipt) => String::new(),
        Ok(Content::Reply(r)) => truncate(&reply_text(&r.content), 28),
        Ok(Content::Attachment(a)) => {
            format!(
                "[file: {}]",
                truncate(a.filename.as_deref().unwrap_or("file"), 20)
            )
        }
        Ok(Content::RemoteAttachment(_)) => "[attachment]".into(),
        Ok(Content::MultiRemoteAttachment(a)) => format!("[{} attachments]", a.len()),
        Ok(Content::TransactionReference(t)) => truncate(&t.reference, 28),
        Ok(Content::WalletSendCalls(_)) => "[wallet]".into(),
        Ok(Content::Actions(a)) => truncate(&a.description, 28),
        Ok(Content::Intent(i)) => truncate(&i.action_id, 28),
        Ok(Content::LeaveRequest { .. }) => "[leave]".into(),
        Ok(Content::DeleteMessage { .. }) => "[deleted]".into(),
        Ok(Content::Unknown { .. }) | Err(_) => msg.fallback.clone().unwrap_or_default(),
    }
}

/// Decode full message body for the chat view.
pub(crate) fn body(msg: &Message) -> String {
    match msg.decode() {
        Ok(Content::Text(s) | Content::Markdown(s)) => s,
        Ok(Content::Reaction(r)) => format!("[{}]", r.content),
        Ok(Content::ReadReceipt) => "[read]".into(),
        Ok(Content::Reply(r)) => reply_text(&r.content),
        Ok(Content::Attachment(a)) => {
            format!("[file: {}]", a.filename.as_deref().unwrap_or("file"))
        }
        Ok(Content::RemoteAttachment(_)) => "[remote attachment]".into(),
        Ok(Content::MultiRemoteAttachment(a)) => format!("[{} attachments]", a.len()),
        Ok(Content::TransactionReference(t)) => format!("[tx: {}]", t.reference),
        Ok(Content::WalletSendCalls(w)) => format!("[wallet: {} calls]", w.calls.len()),
        Ok(Content::Actions(a)) => a.description,
        Ok(Content::Intent(i)) => format!("[intent: {}]", i.action_id),
        Ok(Content::LeaveRequest { .. }) => "[leave request]".into(),
        Ok(Content::DeleteMessage { message_id }) => format!("[delete: {message_id}]"),
        Ok(Content::Unknown { content_type, .. }) => format!("[unknown: {content_type}]"),
        Err(_) => msg.fallback.clone().unwrap_or_default(),
    }
}

/// Delivery status indicator for TUI display.
pub(crate) const fn delivery_icon(status: DeliveryStatus) -> &'static str {
    match status {
        DeliveryStatus::Published => "✓",
        DeliveryStatus::Unpublished => "○",
        DeliveryStatus::Failed => "✗",
    }
}

// ── Agent / JSON output ───────────────────────────────────────────

/// Plain text extraction for agent output (no truncation).
pub(crate) fn text(msg: &Message) -> String {
    if msg.kind != MessageKind::Application {
        return String::new();
    }
    match msg.decode() {
        Ok(Content::Text(s) | Content::Markdown(s)) => s,
        Ok(Content::Reaction(r)) => r.content,
        Ok(Content::Reply(r)) => reply_text(&r.content),
        _ => String::new(),
    }
}

/// Build a JSON representation of message content.
pub(crate) fn content_json(msg: &Message) -> Value {
    if msg.kind != MessageKind::Application {
        return json!({"type": "system"});
    }
    match msg.decode() {
        Ok(Content::Text(s)) => json!({"type": "text", "text": s}),
        Ok(Content::Markdown(s)) => json!({"type": "markdown", "text": s}),
        Ok(Content::Reaction(r)) => json!({
            "type": "reaction",
            "emoji": r.content,
            "reference_message_id": r.reference,
        }),
        Ok(Content::Reply(r)) => json!({
            "type": "reply",
            "reference_message_id": r.reference,
            "text": reply_text(&r.content),
        }),
        Ok(Content::ReadReceipt) => json!({"type": "read_receipt"}),
        Ok(Content::Attachment(a)) => json!({
            "type": "attachment",
            "filename": a.filename,
            "mime_type": a.mime_type,
            "size": a.data.len(),
        }),
        Ok(Content::RemoteAttachment(a)) => json!({
            "type": "remote_attachment",
            "url": a.url,
            "filename": a.filename,
        }),
        Ok(Content::MultiRemoteAttachment(a)) => json!({
            "type": "multi_remote_attachment",
            "count": a.len(),
        }),
        Ok(Content::TransactionReference(t)) => json!({
            "type": "transaction_reference",
            "network_id": t.network_id,
            "reference": t.reference,
        }),
        Ok(Content::WalletSendCalls(w)) => json!({
            "type": "wallet_send_calls",
            "chain_id": w.chain_id,
            "from": w.from,
            "calls": w.calls.len(),
        }),
        Ok(Content::Actions(a)) => json!({
            "type": "actions",
            "id": a.id,
            "description": a.description,
        }),
        Ok(Content::Intent(i)) => json!({
            "type": "intent",
            "id": i.id,
            "action_id": i.action_id,
        }),
        Ok(Content::LeaveRequest { .. }) => json!({"type": "leave_request"}),
        Ok(Content::DeleteMessage { message_id }) => json!({
            "type": "delete_message",
            "message_id": message_id,
        }),
        Ok(Content::Unknown { content_type, .. }) => json!({
            "type": "unknown",
            "content_type": content_type,
        }),
        Err(e) => json!({"type": "error", "error": e.to_string()}),
    }
}
