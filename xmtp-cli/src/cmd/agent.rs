//! Agent-friendly subcommands with structured JSON output.
//!
//! These commands are designed for AI agents and scripts. They support `--json`
//! for structured output and `stream` always emits NDJSON.

use std::io::{self, Write};
use std::path::Path;

use serde_json::{Value, json};
use xmtp::{
    ArchiveOptions, ConsentState, ConversationOrderBy, ConversationType, CreateGroupOptions,
    DeliveryStatus, ListConversationsOptions, ListMessagesOptions, MessageKind, Recipient,
    SendOptions, SortDirection, stream,
};

use super::ArchiveCommand;
use super::config;
use crate::decode;

enum StreamEvent {
    Message {
        msg_id: String,
    },
    Conversation {
        id: String,
        conv_type: Option<ConversationType>,
        name: Option<String>,
    },
}

/// Write a JSON value as a single line to stdout, flushing immediately.
fn emit(value: &Value) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    drop(serde_json::to_writer(&mut out, value));
    drop(writeln!(out));
    drop(out.flush());
}

const fn conv_type_str(t: Option<ConversationType>) -> &'static str {
    match t {
        Some(ConversationType::Dm) => "dm",
        Some(ConversationType::Group) => "group",
        _ => "unknown",
    }
}

const fn delivery_status_str(s: DeliveryStatus) -> &'static str {
    match s {
        DeliveryStatus::Published => "published",
        DeliveryStatus::Unpublished => "unpublished",
        DeliveryStatus::Failed => "failed",
    }
}

const fn consent_state_str(s: ConsentState) -> &'static str {
    match s {
        ConsentState::Allowed => "allowed",
        ConsentState::Denied => "denied",
        ConsentState::Unknown => "unknown",
    }
}

fn parse_consent(s: &str) -> Option<ConsentState> {
    match s.to_ascii_lowercase().as_str() {
        "allowed" | "accepted" => Some(ConsentState::Allowed),
        "denied" | "hidden" | "blocked" => Some(ConsentState::Denied),
        "unknown" | "pending" | "requested" => Some(ConsentState::Unknown),
        _ => None,
    }
}

/// `xmtp conversations [--consent STATE] [--json]`
pub(crate) fn conversations(profile: &str, consent: Option<&str>, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    drop(client.sync_welcomes());
    drop(client.sync_all(&[]));

    let consent_states = match consent {
        Some(s) => {
            let state = parse_consent(s).ok_or_else(|| {
                xmtp::XmtpError::InvalidArgument(format!(
                    "invalid consent state: {s} (expected: allowed, denied, unknown)"
                ))
            })?;
            vec![state]
        }
        None => vec![],
    };

    let opts = ListConversationsOptions {
        consent_states,
        order_by: ConversationOrderBy::LastActivity,
        ..Default::default()
    };
    let convs = client.list_conversations(&opts)?;

    if json {
        let items: Vec<Value> = convs
            .iter()
            .map(|c| {
                let last = c.last_message().ok().flatten();
                json!({
                    "id": c.id(),
                    "type": conv_type_str(c.conversation_type()),
                    "name": c.name(),
                    "last_message": last.as_ref().map(decode::text),
                    "last_message_ns": last.as_ref().map(|m| m.sent_at_ns),
                })
            })
            .collect();
        emit(&json!({"conversations": items}));
    } else {
        if convs.is_empty() {
            println!("No conversations.");
            return Ok(());
        }
        for c in &convs {
            let kind = conv_type_str(c.conversation_type());
            let name = c.name().unwrap_or_default();
            let id = c.id();
            println!("  [{kind}] {id}  {name}");
        }
    }
    Ok(())
}

/// `xmtp messages <conv_id> [--limit N] [--json]`
pub(crate) fn messages(
    profile: &str,
    conv_id: &str,
    limit: Option<usize>,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    drop(client.sync_all(&[]));

    let conv = client.conversation(conv_id)?.ok_or_else(|| {
        xmtp::XmtpError::InvalidArgument(format!("conversation not found: {conv_id}"))
    })?;

    let opts = ListMessagesOptions {
        direction: Some(SortDirection::Ascending),
        #[allow(clippy::cast_possible_wrap, reason = "CLI limit value fits in i64")]
        limit: limit.map_or(0, |l| l as i64),
        ..Default::default()
    };

    let msgs = conv.list_messages(&opts)?;

    if json {
        let items: Vec<Value> = msgs
            .iter()
            .filter(|m| m.kind == MessageKind::Application)
            .map(|m| {
                json!({
                    "id": m.id,
                    "conversation_id": m.conversation_id,
                    "sender_inbox_id": m.sender_inbox_id,
                    "sent_at_ns": m.sent_at_ns,
                    "delivery_status": delivery_status_str(m.delivery_status),
                    "content": decode::content_json(m),
                })
            })
            .collect();
        emit(&json!({"messages": items, "conversation_id": conv_id}));
    } else {
        if msgs.is_empty() {
            println!("No messages.");
            return Ok(());
        }
        for m in &msgs {
            if m.kind != MessageKind::Application {
                continue;
            }
            let text = decode::text(m);
            let sender = &m.sender_inbox_id;
            let status = delivery_status_str(m.delivery_status);
            println!("  [{status}] {sender}: {text}");
        }
    }
    Ok(())
}

/// `xmtp send <conv_id> <text> [--push] [--json]`
pub(crate) fn send(
    profile: &str,
    conv_id: &str,
    text: &str,
    push: bool,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;

    let conv = client.conversation(conv_id)?.ok_or_else(|| {
        xmtp::XmtpError::InvalidArgument(format!("conversation not found: {conv_id}"))
    })?;

    let opts = SendOptions { should_push: push };
    let msg_id = conv.send_text_with(text, opts)?;

    if json {
        emit(&json!({"ok": true, "message_id": msg_id, "conversation_id": conv_id}));
    } else {
        println!("Sent (message_id: {msg_id})");
    }
    Ok(())
}

/// `xmtp dm <address> [--json]`
pub(crate) fn dm(profile: &str, address: &str, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    drop(client.sync_welcomes());

    let recipient = Recipient::parse(address);
    let conv = client.dm(&recipient)?;
    let id = conv.id();

    if json {
        emit(&json!({
            "conversation_id": id,
            "type": "dm",
            "peer": address,
        }));
    } else {
        println!("DM conversation: {id}");
    }
    Ok(())
}

/// `xmtp group <members...> [--name NAME] [--json]`
pub(crate) fn create_group(
    profile: &str,
    member_addrs: &[String],
    name: Option<&str>,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    drop(client.sync_welcomes());

    let members: Vec<Recipient> = member_addrs.iter().map(|s| Recipient::parse(s)).collect();
    let opts = CreateGroupOptions {
        name: name.map(String::from),
        ..Default::default()
    };

    let conv = client.group(&members, &opts)?;
    let id = conv.id();

    if json {
        emit(&json!({
            "conversation_id": id,
            "type": "group",
            "name": name,
            "members": member_addrs,
        }));
    } else {
        println!("Group created: {id}");
    }
    Ok(())
}

/// `xmtp members <conv_id> [--json]`
pub(crate) fn members(profile: &str, conv_id: &str, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;

    let conv = client.conversation(conv_id)?.ok_or_else(|| {
        xmtp::XmtpError::InvalidArgument(format!("conversation not found: {conv_id}"))
    })?;

    let members = conv.members()?;

    if json {
        let items: Vec<Value> = members
            .iter()
            .map(|m| {
                json!({
                    "inbox_id": m.inbox_id,
                    "addresses": m.account_identifiers,
                    "permission": format!("{:?}", m.permission_level).to_lowercase(),
                    "consent": consent_state_str(m.consent_state),
                })
            })
            .collect();
        emit(&json!({"members": items, "conversation_id": conv_id}));
    } else {
        if members.is_empty() {
            println!("No members.");
            return Ok(());
        }
        for m in &members {
            let addr = m.account_identifiers.first().map_or("—", String::as_str);
            println!("  {} ({addr}) [{:?}]", m.inbox_id, m.permission_level);
        }
    }
    Ok(())
}

/// `xmtp can-message <addresses...> [--json]`
pub(crate) fn can_message(profile: &str, addresses: &[String], json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;

    let recipients: Vec<Recipient> = addresses.iter().map(|s| Recipient::parse(s)).collect();
    let refs: Vec<&Recipient> = recipients.iter().collect();
    let results = client.can_message_recipients(&refs)?;

    if json {
        let items: Vec<Value> = addresses
            .iter()
            .zip(&results)
            .map(|(addr, ok)| json!({"address": addr, "can_message": ok}))
            .collect();
        emit(&json!({"results": items}));
    } else {
        for (addr, ok) in addresses.iter().zip(&results) {
            let status = if *ok { "yes" } else { "no" };
            println!("  {addr}: {status}");
        }
    }
    Ok(())
}

/// `xmtp request <conv_id> accept|deny [--json]`
pub(crate) fn request(profile: &str, conv_id: &str, action: &str, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;

    let conv = client.conversation(conv_id)?.ok_or_else(|| {
        xmtp::XmtpError::InvalidArgument(format!("conversation not found: {conv_id}"))
    })?;

    let state = match action.to_ascii_lowercase().as_str() {
        "accept" | "allow" => ConsentState::Allowed,
        "deny" | "block" | "hide" => ConsentState::Denied,
        _ => {
            return Err(xmtp::XmtpError::InvalidArgument(format!(
                "invalid action: {action} (expected: accept, deny)"
            )));
        }
    };

    conv.set_consent(state)?;

    if json {
        emit(&json!({
            "ok": true,
            "conversation_id": conv_id,
            "consent": consent_state_str(state),
        }));
    } else {
        println!("Conversation {conv_id}: {}", consent_state_str(state));
    }
    Ok(())
}

/// `xmtp stream [messages|conversations|all]`
///
/// Outputs NDJSON events to stdout. Runs until interrupted.
pub(crate) fn stream_events(profile: &str, kind: &str) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    drop(client.sync_welcomes());
    drop(client.sync_all(&[]));

    let (tx, rx) = std::sync::mpsc::channel::<StreamEvent>();
    let stream_msgs = kind == "messages" || kind == "all";
    let stream_convs = kind == "conversations" || kind == "all";

    if !stream_msgs && !stream_convs {
        return Err(xmtp::XmtpError::InvalidArgument(format!(
            "invalid stream type: {kind} (expected: messages, conversations, all)"
        )));
    }

    if stream_msgs {
        let sub = stream::messages(&client, None, &[])?;
        let tx_msg = tx.clone();
        std::thread::spawn(move || pipe_messages(sub, &tx_msg));
    }

    if stream_convs {
        let sub = stream::conversations(&client, None)?;
        let tx_conv = tx.clone();
        std::thread::spawn(move || pipe_conversations(sub, &tx_conv));
    }

    drop(tx);

    emit(&json!({"type": "ready", "stream": kind}));

    while let Ok(event) = rx.recv() {
        match event {
            StreamEvent::Message { msg_id } => {
                if let Ok(Some(msg)) = client.message_by_id(&msg_id) {
                    emit(&json!({
                        "type": "message",
                        "message_id": msg.id,
                        "conversation_id": msg.conversation_id,
                        "sender_inbox_id": msg.sender_inbox_id,
                        "sent_at_ns": msg.sent_at_ns,
                        "delivery_status": delivery_status_str(msg.delivery_status),
                        "content": decode::content_json(&msg),
                    }));
                }
            }
            StreamEvent::Conversation {
                id,
                conv_type,
                name,
            } => {
                drop(client.sync_welcomes());
                emit(&json!({
                    "type": "conversation",
                    "conversation_id": id,
                    "conversation_type": conv_type_str(conv_type),
                    "name": name,
                }));
            }
        }
    }
    Ok(())
}

fn pipe_messages(
    sub: stream::Subscription<stream::MessageEvent>,
    tx: &std::sync::mpsc::Sender<StreamEvent>,
) {
    for ev in sub {
        if tx
            .send(StreamEvent::Message {
                msg_id: ev.message_id,
            })
            .is_err()
        {
            break;
        }
    }
}

fn pipe_conversations(
    sub: stream::Subscription<xmtp::Conversation>,
    tx: &std::sync::mpsc::Sender<StreamEvent>,
) {
    for conv in sub {
        if tx
            .send(StreamEvent::Conversation {
                id: conv.id(),
                conv_type: conv.conversation_type(),
                name: conv.name(),
            })
            .is_err()
        {
            break;
        }
    }
}

fn profile_name(explicit: Option<&String>) -> String {
    explicit.cloned().unwrap_or_else(config::default_profile)
}

fn path_as_str(path: &Path) -> xmtp::Result<&str> {
    path.to_str().ok_or_else(|| {
        xmtp::XmtpError::InvalidArgument(format!("path is not valid UTF-8: {}", path.display()))
    })
}

fn archive_key(profile: &str, key_hex: Option<&str>) -> xmtp::Result<[u8; 32]> {
    key_hex.map_or_else(|| config::load_db_key(profile), config::parse_hex32)
}

/// `xmtp sync [--history-url URL] [--json]`
pub(crate) fn sync(profile: &str, history_url: Option<&str>, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client_with(profile, history_url)?;
    client.send_sync_request(ArchiveOptions::default())?;
    let url = client.history_sync_url();
    if json {
        emit(&json!({"ok": true, "history_url": url}));
    } else {
        println!("Sync request sent ({url})");
    }
    Ok(())
}

/// `xmtp archive …`
pub(crate) fn archive(cmd: &ArchiveCommand) -> xmtp::Result<()> {
    match cmd {
        ArchiveCommand::Create {
            path,
            key,
            profile,
            output,
        } => archive_create(
            &profile_name(profile.as_ref()),
            path,
            key.as_deref(),
            output.json,
        ),
        ArchiveCommand::Import {
            path,
            key,
            profile,
            output,
        } => archive_import(
            &profile_name(profile.as_ref()),
            path,
            key.as_deref(),
            output.json,
        ),
        ArchiveCommand::Metadata {
            path,
            key,
            profile,
            output,
        } => archive_metadata(
            &profile_name(profile.as_ref()),
            path,
            key.as_deref(),
            output.json,
        ),
        ArchiveCommand::List {
            days,
            profile,
            output,
        } => archive_list(&profile_name(profile.as_ref()), *days, output.json),
        ArchiveCommand::Process {
            pin,
            profile,
            output,
        } => archive_process(&profile_name(profile.as_ref()), pin.as_deref(), output.json),
        ArchiveCommand::Send {
            pin,
            history_url,
            profile,
            output,
        } => archive_send(
            &profile_name(profile.as_ref()),
            pin,
            history_url.as_deref(),
            output.json,
        ),
    }
}

fn archive_create(
    profile: &str,
    path: &Path,
    key_hex: Option<&str>,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    let key = archive_key(profile, key_hex)?;
    let path_str = path_as_str(path)?;
    client.create_archive(path_str, &key, ArchiveOptions::default())?;
    if json {
        emit(&json!({"ok": true, "path": path_str}));
    } else {
        println!("Archive written to {path_str}");
    }
    Ok(())
}

fn archive_import(
    profile: &str,
    path: &Path,
    key_hex: Option<&str>,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    let key = archive_key(profile, key_hex)?;
    let path_str = path_as_str(path)?;
    client.import_archive(path_str, &key)?;
    if json {
        emit(&json!({"ok": true, "path": path_str}));
    } else {
        println!("Archive imported from {path_str}");
    }
    Ok(())
}

fn archive_metadata(
    profile: &str,
    path: &Path,
    key_hex: Option<&str>,
    json: bool,
) -> xmtp::Result<()> {
    let key = archive_key(profile, key_hex)?;
    let path_str = path_as_str(path)?;
    let meta = xmtp::archive_metadata(path_str, &key)?;
    if json {
        emit(&json!({
            "backup_version": meta.backup_version,
            "exported_at_ns": meta.exported_at_ns,
            "include_messages": meta.include_messages,
            "include_consent": meta.include_consent,
            "include_event": meta.include_event,
            "start_ns": meta.start_ns,
            "end_ns": meta.end_ns,
        }));
    } else {
        println!("Backup version: {}", meta.backup_version);
        println!("Exported at:    {} ns", meta.exported_at_ns);
        println!("Messages:       {}", meta.include_messages);
        println!("Consent:        {}", meta.include_consent);
        println!("Events:         {}", meta.include_event);
    }
    Ok(())
}

fn archive_list(profile: &str, days: u32, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    let archives = client.list_available_archives(days)?;
    if json {
        let items: Vec<Value> = archives
            .iter()
            .map(|a| json!({"pin": a.pin, "exported_at_ns": a.exported_at_ns}))
            .collect();
        emit(&json!({"archives": items}));
    } else if archives.is_empty() {
        println!("No archives.");
    } else {
        for a in &archives {
            println!("  {}  {}", a.pin, a.exported_at_ns);
        }
    }
    Ok(())
}

fn archive_process(profile: &str, pin: Option<&str>, json: bool) -> xmtp::Result<()> {
    let (_, client) = config::open_client(profile)?;
    client.process_sync_archive(pin)?;
    if json {
        emit(&json!({"ok": true, "pin": pin}));
    } else {
        match pin {
            Some(p) => println!("Processed archive {p}"),
            None => println!("Processed latest archive"),
        }
    }
    Ok(())
}

fn archive_send(
    profile: &str,
    pin: &str,
    history_url: Option<&str>,
    json: bool,
) -> xmtp::Result<()> {
    let (_, client) = config::open_client_with(profile, history_url)?;
    client.send_sync_archive(pin, ArchiveOptions::default())?;
    if json {
        emit(&json!({"ok": true, "pin": pin, "history_url": client.history_sync_url()}));
    } else {
        println!("Archive sent for pin {pin}");
    }
    Ok(())
}
