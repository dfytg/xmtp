#![allow(
    unsafe_code,
    reason = "Archive and device-sync operations require unsafe for FFI calls to xmtp_sys"
)]
//! Device-sync archives: request, list, process, file create/import/metadata.

use std::ffi::CStr;
use std::ptr;

use crate::client::Client;
use crate::error::{self, Result};
use crate::ffi::{FfiList, ffi_usize, to_c_string};
use crate::types::SyncResult;

/// Bit 0 of [`xmtp_sys::XmtpFfiArchiveOptions::elements`].
const ELEMENT_MESSAGES: i32 = 1;
/// Bit 1 of [`xmtp_sys::XmtpFfiArchiveOptions::elements`].
const ELEMENT_CONSENT: i32 = 2;
/// Bit 2 of archive metadata `out_elements` (FFI write path ignores this).
const ELEMENT_EVENT: i32 = 4;

/// Options for device-sync archive operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchiveOptions {
    /// Include messages. Default: `true`.
    pub include_messages: bool,
    /// Include consent records. Default: `true`.
    pub include_consent: bool,
    /// Start timestamp filter (ns). `None` = no lower bound.
    pub start_ns: Option<i64>,
    /// End timestamp filter (ns). `None` = no upper bound.
    pub end_ns: Option<i64>,
    /// Exclude disappearing messages.
    pub exclude_disappearing_messages: bool,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            include_messages: true,
            include_consent: true,
            start_ns: None,
            end_ns: None,
            exclude_disappearing_messages: false,
        }
    }
}

impl ArchiveOptions {
    fn to_ffi(self) -> xmtp_sys::XmtpFfiArchiveOptions {
        let mut elements = 0i32;
        if self.include_messages {
            elements |= ELEMENT_MESSAGES;
        }
        if self.include_consent {
            elements |= ELEMENT_CONSENT;
        }
        xmtp_sys::XmtpFfiArchiveOptions {
            elements,
            start_ns: self.start_ns.unwrap_or(0),
            end_ns: self.end_ns.unwrap_or(0),
            exclude_disappearing_messages: i32::from(self.exclude_disappearing_messages),
        }
    }
}

/// An archive advertised in the device-sync group.
///
/// The C ABI only exposes pin and export timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AvailableArchive {
    /// Pin used to process this archive.
    pub pin: String,
    /// Export timestamp (ns).
    pub exported_at_ns: i64,
}

/// Metadata of a local archive file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchiveMetadata {
    /// Backup format version.
    pub backup_version: u16,
    /// Export timestamp (ns).
    pub exported_at_ns: i64,
    /// Archive includes messages.
    pub include_messages: bool,
    /// Archive includes consent records.
    pub include_consent: bool,
    /// Archive includes event records.
    pub include_event: bool,
    /// Start timestamp filter (ns), if set.
    pub start_ns: Option<i64>,
    /// End timestamp filter (ns), if set.
    pub end_ns: Option<i64>,
}

/// Read metadata from an archive file. Does not require a [`Client`].
pub fn archive_metadata(path: &str, key: &[u8; 32]) -> Result<ArchiveMetadata> {
    let c_path = to_c_string(path)?;
    let mut backup_version = 0u16;
    let mut exported_at_ns = 0i64;
    let mut elements = 0i32;
    let mut start_ns = 0i64;
    let mut end_ns = 0i64;
    // SAFETY: `c_path` and `key` are valid for the duration of the call; output
    // pointers are stack locals.
    error::check(unsafe {
        xmtp_sys::xmtp_device_sync_archive_metadata(
            c_path.as_ptr(),
            key.as_ptr(),
            32,
            &raw mut backup_version,
            &raw mut exported_at_ns,
            &raw mut elements,
            &raw mut start_ns,
            &raw mut end_ns,
        )
    })?;
    Ok(ArchiveMetadata {
        backup_version,
        exported_at_ns,
        include_messages: elements & ELEMENT_MESSAGES != 0,
        include_consent: elements & ELEMENT_CONSENT != 0,
        include_event: elements & ELEMENT_EVENT != 0,
        start_ns: (start_ns > 0).then_some(start_ns),
        end_ns: (end_ns > 0).then_some(end_ns),
    })
}

fn i32_count(n: i32) -> u32 {
    u32::try_from(n).unwrap_or(0)
}

fn read_available_archives(
    ptr: *mut xmtp_sys::XmtpFfiAvailableArchiveList,
) -> Result<Vec<AvailableArchive>> {
    let list = FfiList::new(
        ptr,
        xmtp_sys::xmtp_available_archive_list_len,
        xmtp_sys::xmtp_available_archive_list_free,
    );
    let mut archives = Vec::with_capacity(ffi_usize(list.len()));
    for i in 0..list.len() {
        // SAFETY: `list` is a valid FFI list and `i` is within bounds. The pin
        // pointer is borrowed from the list and must not be freed.
        let pin_ptr = unsafe { xmtp_sys::xmtp_available_archive_pin(list.as_ptr(), i) };
        let pin = if pin_ptr.is_null() {
            String::new()
        } else {
            // SAFETY: non-null borrowed NUL-terminated C string owned by `list`.
            unsafe { CStr::from_ptr(pin_ptr) }
                .to_str()
                .map(String::from)
                .map_err(|_| crate::XmtpError::InvalidUtf8)?
        };
        // SAFETY: `list` is a valid FFI list and `i` is within bounds.
        let exported_at_ns =
            unsafe { xmtp_sys::xmtp_available_archive_exported_at_ns(list.as_ptr(), i) };
        archives.push(AvailableArchive {
            pin,
            exported_at_ns,
        });
    }
    Ok(archives)
}

impl Client {
    /// Send a device-sync request using the URL stored on this client
    /// ([`ClientBuilder::history_sync_url`](crate::ClientBuilder::history_sync_url)
    /// or [`Env::history_sync_url`](crate::Env::history_sync_url)).
    ///
    /// Replaces the removed `request_device_sync`.
    pub fn send_sync_request(&self, opts: ArchiveOptions) -> Result<()> {
        self.send_sync_request_to(&self.history_sync_url, opts)
    }

    /// Send a device-sync request to an explicit server URL (one-shot override).
    pub fn send_sync_request_to(&self, server_url: &str, opts: ArchiveOptions) -> Result<()> {
        let c_url = to_c_string(server_url)?;
        let ffi_opts = opts.to_ffi();
        // SAFETY: valid client handle; `ffi_opts` and `c_url` outlive the call.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_send_request(
                self.handle.as_ptr(),
                &raw const ffi_opts,
                c_url.as_ptr(),
            )
        })
    }

    /// Upload a sync archive for the given pin, using the stored history-sync URL.
    pub fn send_sync_archive(&self, pin: &str, opts: ArchiveOptions) -> Result<()> {
        let c_url = to_c_string(&self.history_sync_url)?;
        let c_pin = to_c_string(pin)?;
        let ffi_opts = opts.to_ffi();
        // SAFETY: valid client handle; C strings and opts outlive the call.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_send_archive(
                self.handle.as_ptr(),
                &raw const ffi_opts,
                c_url.as_ptr(),
                c_pin.as_ptr(),
            )
        })
    }

    /// Process a sync archive. `None` pin processes the latest archive.
    pub fn process_sync_archive(&self, pin: Option<&str>) -> Result<()> {
        let c_pin = pin.map(to_c_string).transpose()?;
        // SAFETY: valid client handle; `c_pin` is null when `pin` is `None`.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_process_archive(
                self.handle.as_ptr(),
                c_pin.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
            )
        })
    }

    /// List archives available in the sync group.
    ///
    /// `days_cutoff` limits how far back to look.
    pub fn list_available_archives(&self, days_cutoff: u32) -> Result<Vec<AvailableArchive>> {
        let mut out: *mut xmtp_sys::XmtpFfiAvailableArchiveList = ptr::null_mut();
        // SAFETY: valid client handle; `out` receives the list pointer.
        let rc = unsafe {
            xmtp_sys::xmtp_device_sync_list_available_archives(
                self.handle.as_ptr(),
                i64::from(days_cutoff),
                &raw mut out,
            )
        };
        error::check(rc)?;
        read_available_archives(out)
    }

    /// Manually sync all device-sync groups.
    pub fn sync_all_device_sync_groups(&self) -> Result<SyncResult> {
        let mut synced = 0i32;
        let mut eligible = 0i32;
        // SAFETY: valid client handle; output pointers are stack locals.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_sync_all(
                self.handle.as_ptr(),
                &raw mut synced,
                &raw mut eligible,
            )
        })?;
        Ok(SyncResult {
            synced: i32_count(synced),
            eligible: i32_count(eligible),
        })
    }

    /// Export an archive to a local file. `key` is the 32-byte encryption key.
    pub fn create_archive(&self, path: &str, key: &[u8; 32], opts: ArchiveOptions) -> Result<()> {
        let c_path = to_c_string(path)?;
        let ffi_opts = opts.to_ffi();
        // SAFETY: valid handle; path, opts, and 32-byte key outlive the call.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_create_archive(
                self.handle.as_ptr(),
                c_path.as_ptr(),
                &raw const ffi_opts,
                key.as_ptr(),
                32,
            )
        })
    }

    /// Import a previously exported archive from a file.
    pub fn import_archive(&self, path: &str, key: &[u8; 32]) -> Result<()> {
        let c_path = to_c_string(path)?;
        // SAFETY: valid handle; path and 32-byte key outlive the call.
        error::check(unsafe {
            xmtp_sys::xmtp_device_sync_import_archive(
                self.handle.as_ptr(),
                c_path.as_ptr(),
                key.as_ptr(),
                32,
            )
        })
    }

    /// Read metadata from an archive file.
    #[allow(
        clippy::unused_self,
        reason = "FFI has no client handle; method exists to match the rest of the archive API"
    )]
    pub fn archive_metadata(&self, path: &str, key: &[u8; 32]) -> Result<ArchiveMetadata> {
        archive_metadata(path, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_options_default_is_messages_and_consent() {
        let opts = ArchiveOptions::default();
        assert!(opts.include_messages);
        assert!(opts.include_consent);
        assert!(!opts.exclude_disappearing_messages);
        assert_eq!(opts.start_ns, None);
        assert_eq!(opts.end_ns, None);
        let ffi = opts.to_ffi();
        assert_eq!(ffi.elements, ELEMENT_MESSAGES | ELEMENT_CONSENT);
        assert_eq!(ffi.start_ns, 0);
        assert_eq!(ffi.end_ns, 0);
        assert_eq!(ffi.exclude_disappearing_messages, 0);
    }

    #[test]
    fn archive_options_to_ffi_bitmask() {
        let ffi = ArchiveOptions {
            include_messages: true,
            include_consent: false,
            start_ns: Some(10),
            end_ns: Some(20),
            exclude_disappearing_messages: true,
        }
        .to_ffi();
        assert_eq!(ffi.elements, ELEMENT_MESSAGES);
        assert_eq!(ffi.start_ns, 10);
        assert_eq!(ffi.end_ns, 20);
        assert_eq!(ffi.exclude_disappearing_messages, 1);

        let empty = ArchiveOptions {
            include_messages: false,
            include_consent: false,
            start_ns: None,
            end_ns: None,
            exclude_disappearing_messages: false,
        }
        .to_ffi();
        assert_eq!(empty.elements, 0);
    }
}
