//! Multi-client sync tests: broadcasts, real-time updates
//!
//! Ported from the original Rust server integration tests.

use super::{
    connect_subject, open_offline_subject, remove_temp_db, serial, skip_if_no_server, temp_db_path,
    TestClient, TEST_EMAIL,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use replicant_client::ClientDatabase;
use replicant_core::models::SyncStatus;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn test_two_clients_same_user() {
    if skip_if_no_server() {
        return;
    }

    // Connect two clients as the same user
    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Client A creates a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Shared Document", "owner": "client_a"});
    let create_result = client_a.create_document_with_id(doc_id, content).await;
    assert!(
        create_result.is_ok(),
        "Create failed: {:?}",
        create_result.err()
    );

    // Give broadcasts time to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client B should see the document via full sync
    let sync_result = client_b.request_full_sync().await.unwrap();
    let documents = sync_result
        .get("documents")
        .and_then(|v| v.as_array())
        .unwrap();
    let found = documents.iter().any(|d| {
        d.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == doc_id.to_string())
            .unwrap_or(false)
    });
    assert!(found, "Client B should see document created by Client A");
}

#[tokio::test]
#[serial]
async fn test_update_propagates_to_other_client() {
    if skip_if_no_server() {
        return;
    }

    // Connect two clients
    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Client A creates a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "Original", "version": 1});
    let create_result = client_a
        .create_document_with_id(doc_id, content)
        .await
        .unwrap();
    let content_hash = create_result
        .get("content_hash")
        .and_then(|v| v.as_str())
        .unwrap();

    // Client A updates the document
    let patch = json!([{"op": "replace", "path": "/title", "value": "Updated by A"}]);
    client_a
        .update_document(doc_id, patch, content_hash)
        .await
        .unwrap();

    // Give broadcasts time to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client B fetches the document via full sync
    let sync_result = client_b.request_full_sync().await.unwrap();
    let documents = sync_result
        .get("documents")
        .and_then(|v| v.as_array())
        .unwrap();
    let doc = documents.iter().find(|d| {
        d.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == doc_id.to_string())
            .unwrap_or(false)
    });

    assert!(doc.is_some(), "Client B should see the document");
    let doc = doc.unwrap();
    let title = doc
        .get("content")
        .and_then(|c| c.get("title"))
        .and_then(|v| v.as_str());
    assert_eq!(
        title,
        Some("Updated by A"),
        "Client B should see updated title"
    );
}

#[tokio::test]
#[serial]
async fn test_delete_propagates_to_other_client() {
    if skip_if_no_server() {
        return;
    }

    // Connect two clients
    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Client A creates a document
    let doc_id = Uuid::new_v4();
    let content = json!({"title": "To Be Deleted"});
    client_a
        .create_document_with_id(doc_id, content)
        .await
        .unwrap();

    // Verify Client B can see it
    let sync_result = client_b.request_full_sync().await.unwrap();
    let documents = sync_result
        .get("documents")
        .and_then(|v| v.as_array())
        .unwrap();
    let found_before = documents.iter().any(|d| {
        d.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == doc_id.to_string())
            .unwrap_or(false)
    });
    assert!(found_before, "Client B should initially see the document");

    // Client A deletes the document
    client_a.delete_document(doc_id).await.unwrap();

    // Give broadcasts time to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client B should no longer see it
    let sync_result = client_b.request_full_sync().await.unwrap();
    let documents = sync_result
        .get("documents")
        .and_then(|v| v.as_array())
        .unwrap();
    let found_after = documents.iter().any(|d| {
        d.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == doc_id.to_string())
            .unwrap_or(false)
    });
    assert!(!found_after, "Client B should not see deleted document");
}

#[tokio::test]
#[serial]
async fn test_incremental_sync_across_clients() {
    if skip_if_no_server() {
        return;
    }

    // Connect two clients
    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Get Client B's current sequence
    let sync_result = client_b.request_full_sync().await.unwrap();
    let sequence_before = sync_result
        .get("latest_sequence")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Client A creates a document
    let content = json!({"title": "Incremental Sync Test"});
    client_a.create_document(content).await.unwrap();

    // Client B gets changes since its last sequence
    let changes_result = client_b.get_changes_since(sequence_before).await.unwrap();
    let events = changes_result
        .get("events")
        .and_then(|v| v.as_array())
        .unwrap();

    assert!(
        !events.is_empty(),
        "Client B should see at least one change event"
    );
}

/// Test from original suite: test_bidirectional_sync
/// Both clients create documents, both should see all documents
#[tokio::test]
#[serial]
async fn test_bidirectional_sync() {
    if skip_if_no_server() {
        return;
    }

    let client_a = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_b = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Both clients create documents
    let doc_a_id = Uuid::new_v4();
    let doc_b_id = Uuid::new_v4();

    client_a
        .create_document_with_id(doc_a_id, json!({"title": "Doc from Client A"}))
        .await
        .unwrap();
    client_b
        .create_document_with_id(doc_b_id, json!({"title": "Doc from Client B"}))
        .await
        .unwrap();

    // Give time to sync
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Both should see both documents
    let sync_a = client_a.request_full_sync().await.unwrap();
    let sync_b = client_b.request_full_sync().await.unwrap();

    let docs_a = sync_a.get("documents").and_then(|v| v.as_array()).unwrap();
    let docs_b = sync_b.get("documents").and_then(|v| v.as_array()).unwrap();

    let has_doc_a_id = |docs: &[serde_json::Value]| {
        docs.iter().any(|d| {
            d.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == doc_a_id.to_string())
                .unwrap_or(false)
        })
    };

    let has_doc_b_id = |docs: &[serde_json::Value]| {
        docs.iter().any(|d| {
            d.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == doc_b_id.to_string())
                .unwrap_or(false)
        })
    };

    assert!(has_doc_a_id(docs_a), "Client A should see doc_a");
    assert!(has_doc_b_id(docs_a), "Client A should see doc_b");
    assert!(has_doc_a_id(docs_b), "Client B should see doc_a");
    assert!(has_doc_b_id(docs_b), "Client B should see doc_b");
}

/// Test from original suite: test_three_clients_full_crud
/// Three clients perform CRUD operations, all should converge
#[tokio::test]
#[serial]
async fn test_three_clients_full_crud() {
    if skip_if_no_server() {
        return;
    }

    let client_1 = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_2 = TestClient::connect(TEST_EMAIL).await.unwrap();
    let client_3 = TestClient::connect(TEST_EMAIL).await.unwrap();

    // Give clients time to connect
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Client 1 creates a document
    let doc_id = Uuid::new_v4();
    let create_result = client_1
        .create_document_with_id(
            doc_id,
            json!({
                "title": "Shared Task",
                "status": "pending",
                "priority": "high"
            }),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify all clients see the document
    for (name, client) in [
        ("Client 1", &client_1),
        ("Client 2", &client_2),
        ("Client 3", &client_3),
    ] {
        let sync = client.request_full_sync().await.unwrap();
        let docs = sync.get("documents").and_then(|v| v.as_array()).unwrap();
        let found = docs.iter().any(|d| {
            d.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == doc_id.to_string())
                .unwrap_or(false)
        });
        assert!(found, "{} should see the document after create", name);
    }

    // Client 2 updates the document
    let content_hash = create_result
        .get("content_hash")
        .and_then(|v| v.as_str())
        .unwrap();
    client_2
        .update_document(
            doc_id,
            json!([{"op": "replace", "path": "/status", "value": "in_progress"}]),
            content_hash,
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify update propagated
    let sync_3 = client_3.request_full_sync().await.unwrap();
    let docs_3 = sync_3.get("documents").and_then(|v| v.as_array()).unwrap();
    let doc = docs_3.iter().find(|d| {
        d.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s == doc_id.to_string())
            .unwrap_or(false)
    });
    let status = doc
        .and_then(|d| d.get("content"))
        .and_then(|c| c.get("status"))
        .and_then(|s| s.as_str());
    assert_eq!(
        status,
        Some("in_progress"),
        "Client 3 should see updated status"
    );

    // Client 3 deletes the document
    client_3.delete_document(doc_id).await.unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify deletion propagated
    for (name, client) in [("Client 1", &client_1), ("Client 2", &client_2)] {
        let sync = client.request_full_sync().await.unwrap();
        let docs = sync.get("documents").and_then(|v| v.as_array()).unwrap();
        let found = docs.iter().any(|d| {
            d.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == doc_id.to_string())
                .unwrap_or(false)
        });
        assert!(!found, "{} should not see deleted document", name);
    }
}

/// Test from original suite: test_no_duplicate_broadcast_to_sender
/// Sender should not receive their own broadcast back
#[tokio::test]
#[serial]
async fn test_no_duplicate_broadcast_to_sender() {
    if skip_if_no_server() {
        return;
    }

    let client = TestClient::connect(TEST_EMAIL).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create a document
    let doc_id = Uuid::new_v4();
    client
        .create_document_with_id(doc_id, json!({"title": "Test No Duplicates"}))
        .await
        .unwrap();

    // Wait for any potential duplicate broadcasts
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify only one document exists
    let sync = client.request_full_sync().await.unwrap();
    let docs = sync.get("documents").and_then(|v| v.as_array()).unwrap();

    // Filter to only docs with our specific ID (avoid interference from other tests)
    let matching_docs: Vec<_> = docs
        .iter()
        .filter(|d| {
            d.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == doc_id.to_string())
                .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        matching_docs.len(),
        1,
        "Should have exactly 1 document with our ID, not duplicates"
    );
}

// ---------------------------------------------------------------------------
// Convergence torture (DEV-1037, Task 7)
// ---------------------------------------------------------------------------

/// Default op stream. Override with `TORTURE_SEED=<u64>` to replay or explore
/// a different interleaving; a failure prints the seed alongside the trace.
const DEFAULT_TORTURE_SEED: u64 = 0xDE1037;
const TORTURE_CLIENTS: usize = 3;
const TORTURE_SEED_DOCS: usize = 4;
const TORTURE_OPS: usize = 30;

/// The op indices at which replica 2 loses and regains its server connection.
const OFFLINE_AT: [usize; 2] = [7, 19];
const ONLINE_AT: [usize; 2] = [13, 25];

/// One simulated device: its own SQLite file, its own `Client`, and a
/// second read-only pool the test uses to inspect the row the client wrote.
struct Replica {
    tag: &'static str,
    db_file: String,
    db_url: String,
    client: Option<replicant_client::Client>,
    db: ClientDatabase,
    online: bool,
}

impl Replica {
    async fn new(tag: &'static str) -> Self {
        let db_file = temp_db_path(tag);
        let db_url = format!("sqlite:{}?mode=rwc", db_file);
        let client = connect_subject(&db_url).await;
        let db = ClientDatabase::new(&db_url).await.unwrap();
        Self {
            tag,
            db_file,
            db_url,
            client: Some(client),
            db,
            online: true,
        }
    }

    fn client(&self) -> &replicant_client::Client {
        self.client.as_ref().expect("replica has a live client")
    }

    /// Drop the connected client before opening its replacement: two `Client`s
    /// on one SQLite file would race each other's writes.
    async fn go_offline(&mut self) {
        self.client = None;
        self.client = Some(open_offline_subject(&self.db_url).await);
        self.online = false;
    }

    async fn go_online(&mut self) {
        self.client = None;
        self.client = Some(connect_subject(&self.db_url).await);
        self.online = true;
    }

    /// Pending documents plus queued patches. There is no public accessor for
    /// the in-flight upload map, so an empty queue held across consecutive
    /// polls stands in for "nothing is still in the air".
    async fn outstanding(&self) -> usize {
        let pending = self
            .client()
            .count_pending_sync()
            .await
            .unwrap_or(usize::MAX);
        let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_queue")
            .fetch_one(&self.db.pool)
            .await
            .unwrap_or(i64::MAX);
        pending + queued as usize
    }
}

/// Wait until every *connected* replica reports nothing outstanding on three
/// consecutive polls, then let the final acks and broadcasts land. Returns
/// false if `max_polls` passes first.
///
/// Offline replicas are excluded by construction: their queue cannot drain
/// while they have nowhere to send it, so waiting on them would only ever
/// burn the budget.
async fn quiesce(replicas: &[Replica], max_polls: usize) -> bool {
    let mut settled_polls = 0;
    for _ in 0..max_polls {
        let mut total = 0;
        for replica in replicas.iter().filter(|r| r.online) {
            total += replica.outstanding().await;
        }
        settled_polls = if total == 0 { settled_polls + 1 } else { 0 };
        if settled_polls >= 3 {
            tokio::time::sleep(Duration::from_millis(2000)).await;
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

fn render_trace(trace: &[String]) -> String {
    let mut out = String::from("--- torture op trace ---\n");
    for line in trace {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Rewrite one key of the replica's *current local* content, so every edit is
/// built on whatever that replica believes the document is — which, mid-run, is
/// routinely stale relative to the server.
async fn edit_locally(replica: &Replica, doc_id: Uuid, step: usize) -> Option<Value> {
    let current = replica.db.get_document(&doc_id).await.ok()?;
    let mut object = current.content.as_object().cloned()?;
    object.insert(format!("k_{}", replica.tag), json!(step));
    let next = Value::Object(object);
    replica
        .client()
        .update_document(doc_id, next.clone())
        .await
        .ok()?;
    Some(next)
}

/// Read the server's live documents as `id -> (content, sync_revision)`.
async fn server_state(driver: &TestClient) -> BTreeMap<Uuid, (Value, i64)> {
    let sync = driver.request_full_sync().await.unwrap();
    let documents = sync.get("documents").and_then(|v| v.as_array()).unwrap();
    documents
        .iter()
        .filter_map(|d| {
            let id = Uuid::parse_str(d.get("id")?.as_str()?).ok()?;
            let content = d.get("content")?.clone();
            let revision = d.get("sync_revision")?.as_i64()?;
            Some((id, (content, revision)))
        })
        .collect()
}

/// Three devices of one user, hammering an overlapping document set with
/// interleaved edits, creates and deletes while one of them drops offline and
/// back twice. Afterwards every replica and the server must hold byte-identical
/// content at the same revision — the whole point of the DEV-1037 work.
///
/// The server still echoes a sender's own broadcasts back to it (DEV-1038), so
/// this run exercises the client's echo-deferral path rather than assuming a
/// quiet channel.
#[tokio::test]
#[serial]
async fn three_client_convergence_torture() {
    if skip_if_no_server() {
        return;
    }

    let seed = std::env::var("TORTURE_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TORTURE_SEED);
    let mut rng = StdRng::seed_from_u64(seed);
    let mut trace: Vec<String> = vec![format!("seed={}", seed)];

    std::fs::create_dir_all("databases").ok();
    let driver = TestClient::connect(TEST_EMAIL).await.unwrap();

    let mut replicas = Vec::with_capacity(TORTURE_CLIENTS);
    for tag in ["r0", "r1", "r2"] {
        replicas.push(Replica::new(tag).await);
    }

    // Seed the shared document set, one creator per document in turn.
    let mut live: Vec<Uuid> = Vec::new();
    let mut retired: Vec<Uuid> = Vec::new();
    let mut applied_edits = 0usize;
    for n in 0..TORTURE_SEED_DOCS {
        let doc_id = Uuid::new_v4();
        let creator = n % TORTURE_CLIENTS;
        replicas[creator]
            .client()
            .create_document_with_id(doc_id, json!({"title": format!("doc-{}", n), "n": n}))
            .await
            .unwrap();
        live.push(doc_id);
        trace.push(format!("seed-create r{} {}", creator, doc_id));
    }
    assert!(
        quiesce(&replicas, 600).await,
        "the seed documents should settle before the run starts (seed={})",
        seed
    );

    for step in 0..TORTURE_OPS {
        if let Some(slot) = OFFLINE_AT.iter().position(|&at| at == step) {
            replicas[2].go_offline().await;
            trace.push(format!("op{:02} r2 OFFLINE (#{})", step, slot));
        }
        if let Some(slot) = ONLINE_AT.iter().position(|&at| at == step) {
            replicas[2].go_online().await;
            trace.push(format!("op{:02} r2 ONLINE (#{})", step, slot));
        }

        let roll = rng.gen_range(0..100);
        if roll < 76 && !live.is_empty() {
            // Edit, weighted toward two replicas hitting the same document with
            // no settle in between.
            let doc_id = live[rng.gen_range(0..live.len())];
            let first = rng.gen_range(0..TORTURE_CLIENTS);
            let applied = edit_locally(&replicas[first], doc_id, step).await;
            applied_edits += applied.is_some() as usize;
            trace.push(format!(
                "op{:02} edit r{} {} -> {}",
                step,
                first,
                doc_id,
                applied.map(|v| v.to_string()).unwrap_or("skipped".into())
            ));

            if rng.gen_bool(0.5) {
                let second = (first + 1 + rng.gen_range(0..TORTURE_CLIENTS - 1)) % TORTURE_CLIENTS;
                let applied = edit_locally(&replicas[second], doc_id, step).await;
                applied_edits += applied.is_some() as usize;
                trace.push(format!(
                    "op{:02} concurrent-edit r{} {} -> {}",
                    step,
                    second,
                    doc_id,
                    applied.map(|v| v.to_string()).unwrap_or("skipped".into())
                ));
            }
        } else if roll < 88 {
            let doc_id = Uuid::new_v4();
            let creator = rng.gen_range(0..TORTURE_CLIENTS);
            replicas[creator]
                .client()
                .create_document_with_id(doc_id, json!({"title": "created mid-run", "at": step}))
                .await
                .unwrap();
            live.push(doc_id);
            trace.push(format!("op{:02} create r{} {}", step, creator, doc_id));
        } else if live.len() > 1 {
            // Retire the document from the edit pool first, then let the
            // connected replicas' edits on it drain: a delete racing an
            // in-flight patch is a server-side not_found, not a convergence
            // question. A replica that is offline may still be holding a
            // queued edit for it, which is the interesting case and is left in.
            let doc_id = live.remove(rng.gen_range(0..live.len()));
            quiesce(&replicas, 100).await;
            let online: Vec<usize> = (0..TORTURE_CLIENTS)
                .filter(|&i| replicas[i].online)
                .collect();
            let deleter = online[rng.gen_range(0..online.len())];
            if replicas[deleter]
                .client()
                .delete_document(doc_id)
                .await
                .is_ok()
            {
                retired.push(doc_id);
                trace.push(format!("op{:02} delete r{} {}", step, deleter, doc_id));
            } else {
                live.push(doc_id);
                trace.push(format!(
                    "op{:02} delete r{} {} FAILED",
                    step, deleter, doc_id
                ));
            }
        }

        tokio::time::sleep(Duration::from_millis(rng.gen_range(20..140))).await;
    }

    if !replicas[2].online {
        replicas[2].go_online().await;
        trace.push("post-run r2 ONLINE".to_string());
    }

    assert!(
        quiesce(&replicas, 600).await,
        "all replicas should drain their queues after the run (seed={})",
        seed
    );

    // A run whose edits all failed to land would converge trivially. Guard the
    // test against silently degenerating into an assertion about nothing.
    assert!(
        applied_edits >= 15,
        "{}\nonly {} edits landed — the run was not a torture (seed={})",
        render_trace(&trace),
        applied_edits,
        seed
    );
    eprintln!(
        "torture seed={} applied {} edits over {} live and {} deleted documents",
        seed,
        applied_edits,
        live.len(),
        retired.len()
    );

    // --- Convergence ------------------------------------------------------
    let server = server_state(&driver).await;
    let mut conflicted: Vec<String> = Vec::new();

    for doc_id in &live {
        let (server_content, server_revision) = server.get(doc_id).unwrap_or_else(|| {
            panic!(
                "{}\nserver lost {} (seed={})",
                render_trace(&trace),
                doc_id,
                seed
            )
        });

        for replica in &replicas {
            let local = replica.db.get_document(doc_id).await.unwrap_or_else(|e| {
                panic!(
                    "{}\n{} lost {} ({e}) (seed={})",
                    render_trace(&trace),
                    replica.tag,
                    doc_id,
                    seed
                )
            });
            assert_eq!(
                &local.content,
                server_content,
                "{}\n{} diverged on {} (seed={})",
                render_trace(&trace),
                replica.tag,
                doc_id,
                seed
            );

            // A document parked in durable Conflict still tracks the server's
            // content (Task 5); only its revision bookkeeping is allowed to
            // lag, so it is reported rather than asserted.
            if replica.db.get_sync_status(doc_id).await.unwrap() == Some(SyncStatus::Conflict) {
                conflicted.push(format!("{} {}", replica.tag, doc_id));
                continue;
            }
            assert_eq!(
                local.sync_revision,
                *server_revision,
                "{}\n{} is at the wrong revision for {} (seed={})",
                render_trace(&trace),
                replica.tag,
                doc_id,
                seed
            );
        }
    }

    for doc_id in &retired {
        assert!(
            !server.contains_key(doc_id),
            "{}\nserver still holds deleted {} (seed={})",
            render_trace(&trace),
            doc_id,
            seed
        );
        for replica in &replicas {
            let ids = replica.client().get_all_document_ids(false).await.unwrap();
            assert!(
                !ids.contains(doc_id),
                "{}\n{} still holds deleted {} (seed={})",
                render_trace(&trace),
                replica.tag,
                doc_id,
                seed
            );
        }
    }

    if !conflicted.is_empty() {
        eprintln!(
            "converged with {} document(s) in durable Conflict: {:?}",
            conflicted.len(),
            conflicted
        );
    }

    for mut replica in replicas {
        replica.client = None;
        remove_temp_db(&replica.db_file);
    }
}
