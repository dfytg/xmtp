//! Signature request creation, application, and installation key signing.

use std::ffi::c_char;
use std::sync::Arc;

use crate::ffi::*;

// ---------------------------------------------------------------------------
// Signature request creation
// ---------------------------------------------------------------------------

/// Create an inbox registration signature request (if needed).
/// Returns null via `out` if no signature is needed.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_create_inbox_signature_request(
    client: *const FfiClient,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch(|| {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let Some(sig_req) = c.inner.identity().signature_request() else {
            unsafe {
                *out = std::ptr::null_mut();
            }
            return Ok(());
        };
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

/// Create a signature request to add a new identifier.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_add_identifier_signature_request(
    client: *const FfiClient,
    identifier: *const c_char,
    identifier_kind: i32,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ident = unsafe { parse_identifier(identifier, identifier_kind)? };
        let sig_req = c.inner.identity_updates().associate_identity(ident).await?;
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

/// Create a signature request to revoke an identifier.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_revoke_identifier_signature_request(
    client: *const FfiClient,
    identifier: *const c_char,
    identifier_kind: i32,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ident = unsafe { parse_identifier(identifier, identifier_kind)? };
        let sig_req = c
            .inner
            .identity_updates()
            .revoke_identities(vec![ident])
            .await?;
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

/// Create a signature request to revoke all other installations.
/// Returns null via `out` if there are no other installations.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_revoke_all_other_installations(
    client: *const FfiClient,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let my_id = c.inner.installation_public_key();
        let inbox_state = c.inner.inbox_state(true).await?;
        let other_ids: Vec<Vec<u8>> = inbox_state
            .installation_ids()
            .into_iter()
            .filter(|id| id != my_id)
            .collect();
        if other_ids.is_empty() {
            unsafe {
                *out = std::ptr::null_mut();
            }
            return Ok(());
        }
        let sig_req = c
            .inner
            .identity_updates()
            .revoke_installations(other_ids)
            .await?;
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Signature request operations
// ---------------------------------------------------------------------------

/// Get the human-readable signature text. Caller must free with [`xmtp_free_string`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_signature_request_text(
    req: *const FfiSignatureRequest,
) -> *mut c_char {
    match unsafe { ref_from(req) } {
        Ok(r) => {
            let text = runtime().block_on(async { r.request.lock().await.signature_text() });
            to_c_string_or_null(&text)
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Add an ECDSA signature to the request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_signature_request_add_ecdsa(
    req: *const FfiSignatureRequest,
    signature_bytes: *const u8,
    signature_len: i32,
) -> i32 {
    catch_async(|| async {
        let r = unsafe { ref_from(req)? };
        let sig = checked_slice_nonempty(signature_bytes, signature_len)?;
        let signature =
            xmtp_id::associations::unverified::UnverifiedSignature::new_recoverable_ecdsa(
                sig.to_vec(),
            );
        let mut req_lock = r.request.lock().await;
        req_lock.add_signature(signature, &r.scw_verifier).await?;
        Ok(())
    })
}

/// Add a passkey signature to the request.
/// All four byte arrays are required and must not be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_signature_request_add_passkey(
    req: *const FfiSignatureRequest,
    public_key: *const u8,
    public_key_len: i32,
    signature: *const u8,
    signature_len: i32,
    authenticator_data: *const u8,
    authenticator_data_len: i32,
    client_data_json: *const u8,
    client_data_json_len: i32,
) -> i32 {
    catch_async(|| async {
        let r = unsafe { ref_from(req)? };
        let to_vec = |p: *const u8, len: i32| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
            Ok(checked_slice_nonempty(p, len)?.to_vec())
        };
        let sig = xmtp_id::associations::unverified::UnverifiedSignature::new_passkey(
            to_vec(public_key, public_key_len)?,
            to_vec(signature, signature_len)?,
            to_vec(authenticator_data, authenticator_data_len)?,
            to_vec(client_data_json, client_data_json_len)?,
        );
        let mut req_lock = r.request.lock().await;
        req_lock.add_signature(sig, &r.scw_verifier).await?;
        Ok(())
    })
}

/// Add a smart contract wallet (SCW) signature to the request.
/// `account_address` is the EVM account address (hex string).
/// `chain_id` is the EVM chain ID (e.g. 1 for mainnet).
/// `block_number` is optional; pass 0 to omit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_signature_request_add_scw(
    req: *const FfiSignatureRequest,
    account_address: *const c_char,
    signature_bytes: *const u8,
    signature_len: i32,
    chain_id: u64,
    block_number: u64,
) -> i32 {
    catch_async(|| async {
        let r = unsafe { ref_from(req)? };
        let addr = unsafe { c_str_to_string(account_address)? };
        let sig = checked_slice_nonempty(signature_bytes, signature_len)?.to_vec();
        let account_id = xmtp_id::associations::AccountId::new_evm(chain_id, addr);
        let bn = if block_number == 0 {
            None
        } else {
            Some(block_number)
        };
        let scw_sig =
            xmtp_id::associations::unverified::NewUnverifiedSmartContractWalletSignature::new(
                sig, account_id, bn,
            );
        let mut req_lock = r.request.lock().await;
        req_lock
            .add_new_unverified_smart_contract_signature(scw_sig, &*r.scw_verifier)
            .await?;
        Ok(())
    })
}

/// Apply a signature request to the client.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_apply_signature_request(
    client: *const FfiClient,
    req: *const FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        let r = unsafe { ref_from(req)? };
        let req_clone = r.request.lock().await.clone();
        c.inner
            .identity_updates()
            .apply_signature_request(req_clone)
            .await?;
        Ok(())
    })
}

free_opaque!(xmtp_signature_request_free, FfiSignatureRequest);

// ---------------------------------------------------------------------------
// Revoke specific installations
// ---------------------------------------------------------------------------

/// Create a signature request to revoke specific installations by their IDs.
/// `installation_ids` is an array of byte arrays, each `id_len` bytes long.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_revoke_installations_signature_request(
    client: *const FfiClient,
    installation_ids: *const *const u8,
    id_lengths: *const i32,
    count: i32,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let installation_ids = checked_slice_nonempty(installation_ids, count)?;
        let id_lengths = checked_slice_nonempty(id_lengths, count)?;
        let mut ids = Vec::with_capacity(installation_ids.len());
        for i in 0..installation_ids.len() {
            ids.push(checked_slice_nonempty(installation_ids[i], id_lengths[i])?.to_vec());
        }
        let sig_req = c.inner.identity_updates().revoke_installations(ids).await?;
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

/// Create a signature request to change the recovery identifier.
/// Caller must free with [`xmtp_signature_request_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_change_recovery_identifier_signature_request(
    client: *const FfiClient,
    new_identifier: *const c_char,
    identifier_kind: i32,
    out: *mut *mut FfiSignatureRequest,
) -> i32 {
    catch_async(|| async {
        let c = unsafe { ref_from(client)? };
        if out.is_null() {
            return Err("null output pointer".into());
        }
        let ident = unsafe { parse_identifier(new_identifier, identifier_kind)? };
        let sig_req = c
            .inner
            .identity_updates()
            .change_recovery_identifier(ident)
            .await?;
        let handle = FfiSignatureRequest {
            request: Arc::new(tokio::sync::Mutex::new(sig_req)),
            scw_verifier: c.inner.scw_verifier().clone(),
        };
        unsafe { write_out(out, handle)? };
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Installation key signing
// ---------------------------------------------------------------------------

/// Sign text with the client's installation key.
/// Writes signature bytes to `out` and length to `out_len`.
/// Caller must free `out` with [`xmtp_free_bytes`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_client_sign_with_installation_key(
    client: *const FfiClient,
    text: *const c_char,
    out: *mut *mut u8,
    out_len: *mut i32,
) -> i32 {
    catch(|| {
        let c = unsafe { ref_from(client)? };
        if out.is_null() || out_len.is_null() {
            return Err("null output pointer".into());
        }
        let text = unsafe { c_str_to_string(text)? };
        let sig = c.inner.context.sign_with_public_context(text)?;
        // Use into_boxed_slice to guarantee cap == len for safe deallocation
        let boxed = sig.into_boxed_slice();
        let len = boxed.len();
        let ptr = Box::into_raw(boxed) as *mut u8;
        unsafe {
            *out = ptr;
            *out_len = len as i32;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Standalone verification
// ---------------------------------------------------------------------------

/// Verify a signature produced by `sign_with_installation_key` using an
/// arbitrary public key. Does not require a client handle.
/// `signature_bytes` must be exactly 64 bytes, `public_key` must be exactly 32 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xmtp_verify_signed_with_public_key(
    signature_text: *const c_char,
    signature_bytes: *const u8,
    signature_len: i32,
    public_key: *const u8,
    public_key_len: i32,
) -> i32 {
    catch(|| {
        let text = unsafe { c_str_to_string(signature_text)? };
        let sig: [u8; 64] = checked_slice_nonempty(signature_bytes, signature_len)?
            .try_into()
            .map_err(|_| "signature_bytes must be exactly 64 bytes")?;
        let key = *checked_key32(public_key, public_key_len)?;
        xmtp_id::associations::verify_signed_with_public_context(text, &sig, &key)?;
        Ok(())
    })
}
