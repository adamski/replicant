//! Pins the Rust client's `calculate_checksum` to the server's `compute_hash`
//! so every later corruption-guard task can rely on byte-identical hashes.

use super::{serial, skip_if_no_server, TestClient, TEST_EMAIL};
use serde_json::{json, Map, Value};
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

/// Erlang maps beyond 32 keys switch from a sorted small-map representation
/// to an unordered HAMT, so this exercises the server's canonical-encode
/// path directly rather than relying on incidental map iteration order. It
/// also covers float magnitudes near serde_json's (ryu's) fixed/scientific
/// notation thresholds — Jason's own formatter disagrees with serde_json at
/// these values (e.g. `1.0e10`/`10000000000.0`, `1.0e-7`/`1e-7`) — plus a
/// non-ASCII key/value pair.
#[tokio::test]
#[serial]
async fn client_and_server_agree_on_content_hash_for_large_object() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    let mut fields = Map::new();
    for i in 1..=35 {
        fields.insert(format!("field_{:02}", i), json!(i));
    }
    fields.insert("whole".to_string(), json!(1.0)); // must not collapse to `1`
    fields.insert("large".to_string(), json!(1.0e10)); // near ryu's large-magnitude threshold
    fields.insert("small".to_string(), json!(1.0e-7)); // near ryu's small-magnitude threshold
    fields.insert("tenth".to_string(), json!(0.1));
    fields.insert("negative".to_string(), json!(-2.5));
    fields.insert("unicode_key_🎵".to_string(), json!("café résumé 音楽"));
    fields.insert(
        "nested".to_string(),
        json!({"z": [3, 2, 1], "a": "x", "deep": {"tags": ["b", "a", "c"]}}),
    );
    fields.insert(
        "items".to_string(),
        Value::Array(
            (0..10)
                .map(|i| json!({"idx": i, "name": format!("item{}", i)}))
                .collect(),
        ),
    );
    let content = Value::Object(fields);

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

/// Exercises canonical_json's recursion into a nested >32-key (HAMT-backed)
/// object sitting inside an otherwise normal-sized (small-map) parent —
/// distinct from the top-level-object case above, which never recurses
/// through a small map to reach the HAMT.
#[tokio::test]
#[serial]
async fn client_and_server_agree_on_content_hash_for_nested_large_object() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();

    let mut child = Map::new();
    for i in 1..=40 {
        child.insert(format!("child_field_{:02}", i), json!(i));
    }
    let content = json!({
        "title": "Parent Doc",
        "count": 3,
        "child": Value::Object(child)
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
