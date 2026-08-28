//! Pins the Rust client's `calculate_checksum` to the server's `compute_hash`
//! so every later corruption-guard task can rely on byte-identical hashes.

use super::{serial, skip_if_no_server, TestClient, TEST_EMAIL};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn client_and_server_agree_on_content_hash() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();
    let content = json!({
        "type": "tuning", "title": "Interop", "zeta": 1,
        "alpha": {"nested_z": [1, 2, 3], "nested_a": "x"},
        "pitches": ["1/1", "9/8", "5/4"], "referenceFrequency": 261.626
    });

    let resp = client
        .create_document_with_id(Uuid::new_v4(), content.clone())
        .await
        .unwrap();
    let server_hash = resp.get("content_hash").unwrap().as_str().unwrap();

    assert_eq!(
        server_hash,
        replicant_core::patches::calculate_checksum(&content)
    );
}
