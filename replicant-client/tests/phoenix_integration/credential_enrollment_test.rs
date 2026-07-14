//! Negative enrollment guard: a credential with no bound user (the legacy
//! shared-style secret minted via `ReplicantServer.Auth.create_credential/1`,
//! which leaves `user_id` nil) must be REJECTED at channel join.
//!
//! Post-#6 the server resolves identity from the credential's `user_id`; a nil
//! user cannot resolve identity, so join is refused with `credential_not_enrolled`
//! (server: `Sync.Channel.require_enrolled/1`, locked by f4d37db) before any
//! topic check. This asserts that guard end-to-end.
//!
//! Gated behind `RUN_INTEGRATION_TESTS`. The harness seeds the legacy credential
//! and exports `REPLICANT_LEGACY_API_KEY` / `REPLICANT_LEGACY_API_SECRET`.

use super::{raw_user_join_error, serial, skip_if_no_server, TestClient, TEST_EMAIL};
use replicant_client::{error_code_for_join_reject, ReplicantErrorCode};

fn legacy_credentials() -> Option<(String, String)> {
    let key = std::env::var("REPLICANT_LEGACY_API_KEY").ok()?;
    let secret = std::env::var("REPLICANT_LEGACY_API_SECRET").ok()?;
    Some((key, secret))
}

#[tokio::test]
#[serial]
async fn test_unenrolled_credential_rejected_at_join() {
    if skip_if_no_server() {
        return;
    }

    let Some((legacy_key, legacy_secret)) = legacy_credentials() else {
        panic!(
            "RUN_INTEGRATION_TESTS is set but REPLICANT_LEGACY_API_KEY/SECRET are missing; \
             the harness must seed a nil-user credential and export them"
        );
    };

    let result =
        TestClient::connect_with_credentials(TEST_EMAIL, &legacy_key, &legacy_secret).await;

    let err = result
        .err()
        .expect("join with an unenrolled (nil-user) credential must be rejected, but it connected");
    assert!(
        err.contains("credential_not_enrolled"),
        "expected credential_not_enrolled rejection, got: {}",
        err
    );

    // Assert the STRUCTURED code the client derives from the real rejection
    // payload, exercising the production mapping (not just the message string).
    let join_error = raw_user_join_error(TEST_EMAIL, &legacy_key, &legacy_secret)
        .await
        .expect("unenrolled credential must produce a join error");
    assert_eq!(
        error_code_for_join_reject(&join_error),
        ReplicantErrorCode::CredentialNotEnrolled,
        "expected CredentialNotEnrolled (1003), got: {:?} from {:?}",
        error_code_for_join_reject(&join_error),
        join_error
    );
}
