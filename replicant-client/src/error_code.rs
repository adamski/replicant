//! Stable, structured error codes carried alongside `SyncError` events.
//!
//! Consumers must be able to react to a sync error by the *action* it calls for
//! (clear the credential? retry? refuse to sync?) without substring-matching a
//! free-form message. Every `SyncError` event carries a [`ReplicantErrorCode`]
//! whose numeric value is stable and exported to C.

/// Structured error code carried by every `SyncError` event.
///
/// The numeric values are STABLE and exported to C via cbindgen. They are
/// banded by the action a consumer should take:
///
/// - `0` — unknown / uncategorized.
/// - `1xxx` — **credential rejected**: the stored credential is bad. The
///   consumer should clear it and re-enroll. See [`is_credential_rejection`].
/// - `2xxx` — **transient**: retry later; NEVER clear credentials. This band
///   includes the timestamp reasons (`2101`, `2102`), which are client/server
///   clock skew — not a bad credential — and so must never trigger a clear.
/// - `3xxx` — **protocol**: the exchange was malformed or violated the contract.
/// - `4xxx` — **identity drift**: the local identity diverged from the account;
///   refuse to sync, but do NOT clear credentials.
/// - `5xxx` — **unresolved divergence**: a local edit could not be reconciled
///   with the server's copy. Retrying cannot help; surface it to the user.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicantErrorCode {
    /// Unknown or uncategorized error.
    Unknown = 0,

    // 1xxx — credential rejected (clear the stored credential and re-enroll)
    /// The API key is not recognized by the server.
    InvalidApiKey = 1001,
    /// The HMAC signature did not verify.
    InvalidSignature = 1002,
    /// The credential authenticated but is not bound to an enrolled user.
    CredentialNotEnrolled = 1003,

    // 2xxx — transient (retry; never clear credentials)
    /// The socket/transport failed to connect.
    ConnectionFailed = 2001,
    /// A join or call timed out.
    Timeout = 2002,
    /// The signed timestamp was outside the server's acceptance window
    /// (client/server clock skew, not a bad credential).
    TimestampExpired = 2101,
    /// The timestamp field was malformed or unparseable (treated as clock skew).
    InvalidTimestamp = 2102,

    // 3xxx — protocol
    /// A required join parameter was missing.
    MissingParams = 3001,
    /// The join topic's user id did not match the credential's user.
    TopicUserMismatch = 3002,
    /// Generic malformed or unexpected server reply.
    ProtocolError = 3003,

    // 4xxx — identity drift (refuse to sync; do NOT clear credentials)
    /// The server-reported user id diverged from the local identity.
    IdentityDrift = 4001,

    // 5xxx — unresolved divergence (surface to the user; do NOT retry)
    /// A local edit could not be rebased onto the server's current content.
    /// The server's copy is now local truth and the edit was discarded.
    UpdateConflict = 5001,
}

/// True iff `code` is in the credential-rejection band (`1xxx`).
///
/// A `true` result means the consumer should clear the stored credential and
/// re-enroll. This is the single source of truth for the band check so bindings
/// do not re-derive the range.
pub fn is_credential_rejection(code: ReplicantErrorCode) -> bool {
    (1000..2000).contains(&(code as i32))
}

/// Map a server join-rejection reason string to its [`ReplicantErrorCode`].
///
/// The reasons are the atoms the phoenix server sends in `{:error, %{reason:
/// "<atom>"}}` (see `replicant_server` `Sync.Channel`/`Auth`). A reason that is
/// present but not recognized is treated as a protocol contract mismatch
/// ([`ReplicantErrorCode::ProtocolError`]); the *absent*-reason case is handled
/// by the caller ([`crate::websocket::error_code_for_join_reject`]).
pub fn error_code_for_reason(reason: &str) -> ReplicantErrorCode {
    match reason {
        "invalid_api_key" => ReplicantErrorCode::InvalidApiKey,
        "invalid_signature" => ReplicantErrorCode::InvalidSignature,
        "credential_not_enrolled" => ReplicantErrorCode::CredentialNotEnrolled,
        "timestamp_expired" => ReplicantErrorCode::TimestampExpired,
        "invalid_timestamp" => ReplicantErrorCode::InvalidTimestamp,
        "missing_params" => ReplicantErrorCode::MissingParams,
        "topic_user_mismatch" => ReplicantErrorCode::TopicUserMismatch,
        "invalid_topic" => ReplicantErrorCode::ProtocolError,
        _ => ReplicantErrorCode::ProtocolError,
    }
}

/// Band check exposed over FFI: `true` iff `code` is a credential rejection.
///
/// Bindings should treat a `true` result as "clear the stored credential and
/// re-enroll". Implemented once here so consumers do not re-implement the band
/// logic against the raw numeric values.
///
/// # Safety
/// This function is pure and takes the code by value; it is always safe to call.
#[no_mangle]
pub extern "C" fn replicant_error_is_credential_rejection(code: i32) -> bool {
    (1000..2000).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_server_reason_maps_to_its_code() {
        assert_eq!(
            error_code_for_reason("invalid_api_key"),
            ReplicantErrorCode::InvalidApiKey
        );
        assert_eq!(
            error_code_for_reason("invalid_signature"),
            ReplicantErrorCode::InvalidSignature
        );
        assert_eq!(
            error_code_for_reason("credential_not_enrolled"),
            ReplicantErrorCode::CredentialNotEnrolled
        );
        assert_eq!(
            error_code_for_reason("timestamp_expired"),
            ReplicantErrorCode::TimestampExpired
        );
        assert_eq!(
            error_code_for_reason("invalid_timestamp"),
            ReplicantErrorCode::InvalidTimestamp
        );
        assert_eq!(
            error_code_for_reason("missing_params"),
            ReplicantErrorCode::MissingParams
        );
        assert_eq!(
            error_code_for_reason("topic_user_mismatch"),
            ReplicantErrorCode::TopicUserMismatch
        );
        assert_eq!(
            error_code_for_reason("invalid_topic"),
            ReplicantErrorCode::ProtocolError
        );
    }

    #[test]
    fn unrecognized_reason_is_protocol_error() {
        assert_eq!(
            error_code_for_reason("something_new"),
            ReplicantErrorCode::ProtocolError
        );
    }

    #[test]
    fn credential_rejection_band_is_1xxx_only() {
        // 1xxx band: true
        assert!(is_credential_rejection(ReplicantErrorCode::InvalidApiKey));
        assert!(is_credential_rejection(
            ReplicantErrorCode::InvalidSignature
        ));
        assert!(is_credential_rejection(
            ReplicantErrorCode::CredentialNotEnrolled
        ));

        // every other band: false
        assert!(!is_credential_rejection(ReplicantErrorCode::Unknown));
        assert!(!is_credential_rejection(
            ReplicantErrorCode::ConnectionFailed
        ));
        assert!(!is_credential_rejection(ReplicantErrorCode::Timeout));
        assert!(!is_credential_rejection(
            ReplicantErrorCode::TimestampExpired
        ));
        assert!(!is_credential_rejection(
            ReplicantErrorCode::InvalidTimestamp
        ));
        assert!(!is_credential_rejection(ReplicantErrorCode::MissingParams));
        assert!(!is_credential_rejection(
            ReplicantErrorCode::TopicUserMismatch
        ));
        assert!(!is_credential_rejection(ReplicantErrorCode::ProtocolError));
        assert!(!is_credential_rejection(ReplicantErrorCode::IdentityDrift));
        assert!(!is_credential_rejection(ReplicantErrorCode::UpdateConflict));
    }

    #[test]
    fn ffi_band_check_matches_rust_helper() {
        assert!(replicant_error_is_credential_rejection(
            ReplicantErrorCode::CredentialNotEnrolled as i32
        ));
        assert!(!replicant_error_is_credential_rejection(
            ReplicantErrorCode::Timeout as i32
        ));
        assert!(!replicant_error_is_credential_rejection(
            ReplicantErrorCode::IdentityDrift as i32
        ));
    }
}
