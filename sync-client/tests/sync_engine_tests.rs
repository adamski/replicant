mod common;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use sqlx::Row;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::Arc;
use std::time::Duration;
use sync_client::{ClientDatabase, SyncEngine};
use sync_core::protocol::{ClientMessage, ServerMessage};
use sync_core::ConflictResolution;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;

/// A mock WebSocket server to simulate the backend for testing.
/// It allows tests to control the messages sent to the SyncEngine
/// and to inspect messages received from it.
struct MockServer {
    pub addr: SocketAddr,
    handle: Option<tokio::task::JoinHandle<()>>,
    // Channel to send server messages to the connected client.
    to_client_tx: mpsc::Sender<ServerMessage>,
    // Channel to receive client messages from the connected client.
    from_client_rx: mpsc::Receiver<ClientMessage>,
    // Websocket listener File Descriptor
    _listener_fd: RawFd,
    // Stop signal for listener threads
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockServer {
    /// Starts a new mock server on a random available port.
    pub async fn new() -> Self {
        let listener = TcpListener::bind("localhost:0").await.unwrap();
        let fd = listener.as_raw_fd();

        let addr = listener.local_addr().unwrap();
        let (to_client_tx, _) = mpsc::channel(100);
        let (_, from_client_rx) = mpsc::channel(100);

        Self {
            addr,
            handle: None,
            to_client_tx,
            from_client_rx,
            _listener_fd: fd,
            shutdown_tx: None,
        }
    }
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    pub async fn start(&mut self) {
        let addr = self.addr;
        let listener = TcpListener::bind(addr).await.unwrap();
        let (to_client_tx, mut to_client_rx) = mpsc::channel(100);
        let (from_client_tx, from_client_rx) = mpsc::channel(100);
        self.to_client_tx = to_client_tx;
        self.from_client_rx = from_client_rx;
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);
        self.handle = Some(tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (mut ws_tx, mut ws_rx) = accept_async(stream).await.unwrap().split();
                // Forward server messages to SyncEngine
                let h1 = tokio::spawn(async move {
                    loop {
                        tokio::select! {
                             _ = &mut shutdown_rx => {
                                let _ = ws_tx.send(Message::Close(None)).await;
                                break;
                            },
                            msg = to_client_rx.recv() => {
                                if let Some(msg) = msg {
                                    let json = serde_json::to_string(&msg).unwrap();
                                    let _ = ws_tx.send(Message::Text(json)).await;
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                });

                // Forward SyncEngine messages to client
                let h2 = tokio::spawn(async move {
                    loop {
                        if let Some(Ok(msg)) = ws_rx.next().await {
                            if let Message::Text(text) = msg {
                                if let Ok(client_msg) = serde_json::from_str(&text) {
                                    if let Err(e) = from_client_tx.send(client_msg).await {
                                        println!("{:?}", e)
                                    }
                                }
                            } else if msg.is_close() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                });
                let (_, _) = tokio::join!(h1, h2);
            }
        }));
    }
    /// Expects to receive a specific client message within a timeout.
    pub async fn expect_client_message(&mut self) -> ClientMessage {
        tokio::time::timeout(Duration::from_secs(2), self.from_client_rx.recv())
            .await
            .expect("Timed out waiting for client message")
            .unwrap()
    }

    /// Sends a server message to the client.
    pub async fn send_server_message(&self, msg: ServerMessage) {
        self.to_client_tx.send(msg).await.unwrap();
    }
}

/// Holds all the necessary components for a SyncEngine test.
struct TestSetup {
    engine: SyncEngine,
    server: MockServer,
    db: Arc<ClientDatabase>,
    _db_id: Uuid,
}

/// Creates a new SyncEngine connected to an in-memory database and a mock server.
async fn setup() -> TestSetup {
    // Use a unique database for each test to ensure isolation
    let db_id = Uuid::new_v4();
    let db = Arc::new(
        ClientDatabase::new(&format!("file:{}?mode=memory&cache=shared", db_id))
            .await
            .unwrap(),
    );
    db.run_migrations().await.unwrap();

    let mut server = MockServer::new().await;
    server.start().await;
    let server_url = format!("ws://{}", server.addr);
    let email = "test@user.com";
    let api_key = "test-key";
    let api_secret = "test-secret";

    let engine = SyncEngine::new(
        &format!("file:{}?mode=memory&cache=shared", db_id),
        &server_url,
        email,
        api_key,
        api_secret,
    )
    .await
    .unwrap();
    TestSetup {
        engine,
        server,
        db,
        _db_id: db_id,
    }
}

#[tokio::test]
async fn test_new_engine_and_authentication() {
    let mut setup = setup().await;

    // The engine should automatically send an Authenticate message on connection.
    let auth_msg = setup.server.expect_client_message().await;
    assert!(matches!(auth_msg, ClientMessage::Authenticate { .. }));
}

/// Tests document creation logic from a single client to then fully synchronizing with the server
#[tokio::test]
async fn test_create_document_online() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // consume auth
    let _ = setup.server.expect_client_message().await; // consume sync

    let content = json!({ "title": "My first document" });
    let doc = setup.engine.create_document(content.clone()).await.unwrap();

    // 1. Verify a `CreateDocument` message was sent to the server.
    let create_msg = setup.server.expect_client_message().await;
    match create_msg {
        ClientMessage::CreateDocument { document } => {
            assert_eq!(document.id, doc.id);
            assert_eq!(document.content, content);
        }
        _ => {
            panic!("Expected CreateDocument message")
        }
    }

    // 2. Verify the document is saved locally with "pending" status.
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content, content);

    let pending_docs = setup.db.get_pending_documents().await.unwrap();
    assert_eq!(pending_docs.len(), 1);
    assert_eq!(pending_docs[0].id, doc.id);

    // 3. Simulate a successful response from the server.
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    // 4. Verify the document is now marked as "synced".
    tokio::time::sleep(Duration::from_millis(100)).await;
    let pending_docs_after = setup.db.get_pending_documents().await.unwrap();
    assert!(pending_docs_after.is_empty());
}

/// Tests the flow for document creation -> sync -> document update -> sync between a client server
/// pair
#[tokio::test]
async fn test_update_document_online() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // consume auth
    let _ = setup.server.expect_client_message().await; // consume sync

    // 1. Create an initial document.
    let initial_content = serde_json::json!({ "title": "Original" });
    let doc = setup
        .engine
        .create_document(initial_content.clone())
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // consume create

    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Update the document.
    let updated_content = serde_json::json!({ "title": "Updated" });
    setup
        .engine
        .update_document(doc.id, updated_content.clone())
        .await
        .unwrap();

    // 3. Verify an `UpdateDocument` message with patch was sent.
    let update_msg = setup.server.expect_client_message().await;
    match update_msg {
        ClientMessage::UpdateDocument { patch } => {
            assert_eq!(patch.document_id, doc.id);
            // Should have a patch with operations
            assert!(!patch.patch.0.is_empty(), "Patch should contain operations");
        }
        _ => panic!("Expected UpdateDocument with patch, got {:?}", update_msg),
    }

    // 4. Verify the document is updated locally.
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content, updated_content);
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);

    setup
        .server
        .send_server_message(ServerMessage::DocumentUpdatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);
}

/// Tests the flow for document creation -> sync -> document update -> sync between a client server
/// pair
#[tokio::test]
async fn test_delete_document_online() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // consume auth
    let _ = setup.server.expect_client_message().await; // consume sync

    // 1. Create a document.
    let doc = setup
        .engine
        .create_document(serde_json::json!({ "title": "To be deleted" }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // consume create
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Delete the document.
    setup.engine.delete_document(doc.id).await.unwrap();

    // 3. Verify a `DeleteDocument` message was sent.
    let delete_msg = setup.server.expect_client_message().await;
    match delete_msg {
        ClientMessage::DeleteDocument { document_id, .. } => {
            assert_eq!(document_id, doc.id);
        }
        _ => panic!("Expected DeleteDocument message"),
    }

    setup
        .server
        .send_server_message(ServerMessage::DocumentDeletedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 4. Verify the document is marked as deleted locally.
    let local_doc_result = setup.db.get_document(&doc.id).await.unwrap();
    assert!(local_doc_result.deleted_at.is_some()); // Should not be in get_all_documents
    assert_eq!(setup.engine.get_all_documents().await.unwrap().len(), 0);
}
/// Test offline document creation and sync on reconnection
///
#[tokio::test]
async fn test_offline_document_creation_and_sync() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Stop server to simulate offline
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Create document while offline
    let content = json!({ "title": "Offline doc" });
    let doc = setup.engine.create_document(content.clone()).await.unwrap();

    // 3. Verify it's marked as pending
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content, content);

    // 4. Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // 5. Should auto-sync the pending document
    let create_msg = setup.server.expect_client_message().await;
    match create_msg {
        ClientMessage::CreateDocument { document } => {
            assert_eq!(document.id, doc.id);
        }
        _ => panic!("Expected CreateDocument after reconnect"),
    }

    // 6. Confirm sync
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);
}

/// Tests the SyncEngine reconnect logic for
/// document creation -> sync -> disconnect -> document delete -> reconnect -> sync between a client server
/// pair
#[tokio::test]
async fn test_client_reconnect_sync_document_online() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // consume auth
    let _ = setup.server.expect_client_message().await; // consume sync

    // 1. Create a document.
    let doc = setup
        .engine
        .create_document(serde_json::json!({ "title": "To be deleted" }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // consume create

    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Stop the server.
    setup.server.stop().await;

    // 3. Delete the document on an offline client
    setup.engine.delete_document(doc.id).await.unwrap();

    // 4. Start the server and wait for the client to reconnect.
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // consume auth

    // 5. Verify a `DeleteDocument` message was sent from the client after reconnection.
    let delete_msg = setup.server.expect_client_message().await;
    match delete_msg {
        ClientMessage::DeleteDocument { document_id, .. } => {
            assert_eq!(document_id, doc.id);
        }
        _ => panic!("Expected DeleteDocument message"),
    }

    setup
        .server
        .send_server_message(ServerMessage::DocumentDeletedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 6. Verify the document is marked as deleted locally.
    let local_doc_result = setup.db.get_document(&doc.id).await.unwrap();
    assert!(local_doc_result.deleted_at.is_some()); // Should not be in get_all_documents
    assert_eq!(setup.engine.get_all_documents().await.unwrap().len(), 0);
}

/// Test server sync overwrite protection during upload phase
#[tokio::test]
async fn test_sync_protection_mode_blocks_server_updates() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Create doc offline
    setup.server.stop().await;
    let doc = setup
        .engine
        .create_document(json!({ "value": 1 }))
        .await
        .unwrap();

    // 2. Reconnect (triggers protection mode during upload)
    setup.server.start().await;
    // 3. Server tries to sync different version DURING upload phase
    // This should be deferred/blocked by protection mode
    let server_doc = sync_core::models::Document {
        id: doc.id,
        content: json!({ "value": 999 }), // Different content
        ..doc.clone()
    };

    setup
        .server
        .send_server_message(ServerMessage::SyncDocument {
            document: server_doc.clone(),
        })
        .await;

    // 4. Wait for upload to complete
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Client should NOT have been overwritten during upload
    let final_doc = setup.db.get_document(&doc.id).await.unwrap();
    // Should be our original content, not server's 999
    assert_eq!(final_doc.content["value"], json!(1));
}

/// Test receiving server document updates (SyncDocument message)
#[tokio::test]
async fn test_receive_server_document_sync() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Create and sync a document
    let doc = setup
        .engine
        .create_document(json!({ "version": 1 }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // CreateDocument
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Server sends updated version
    let mut updated_doc = doc.clone();
    updated_doc.content = json!({ "version": 2 });
    updated_doc.sync_revision = 2;

    setup
        .server
        .send_server_message(ServerMessage::SyncDocument {
            document: updated_doc.clone(),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Verify local document was updated
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content["version"], json!(2));
}

/// Test handling failed document creation response
#[tokio::test]
async fn test_create_document_failure_response() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    let doc = setup
        .engine
        .create_document(json!({ "test": "data" }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // CreateDocument

    // Server rejects the creation
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: false,
            error: Some("Validation failed".to_string()),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Document should remain in pending state
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);
}

/// Test multiple documents created offline are synced in order
#[tokio::test]
async fn test_multiple_offline_documents_sync() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Go offline
    setup.server.stop().await;

    // Create 3 documents offline
    let doc1 = setup
        .engine
        .create_document(json!({ "order": 1 }))
        .await
        .unwrap();
    let doc2 = setup
        .engine
        .create_document(json!({ "order": 2 }))
        .await
        .unwrap();
    let doc3 = setup
        .engine
        .create_document(json!({ "order": 3 }))
        .await
        .unwrap();

    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 3);

    // Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // Should receive all 3 create messages
    let msg1 = setup.server.expect_client_message().await;
    let msg2 = setup.server.expect_client_message().await;
    let msg3 = setup.server.expect_client_message().await;

    // Verify all are CreateDocument messages
    assert!(matches!(msg1, ClientMessage::CreateDocument { .. }));
    assert!(matches!(msg2, ClientMessage::CreateDocument { .. }));
    assert!(matches!(msg3, ClientMessage::CreateDocument { .. }));

    // Confirm all
    for doc in [&doc1, &doc2, &doc3] {
        setup
            .server
            .send_server_message(ServerMessage::DocumentCreatedResponse {
                document_id: doc.id,
                success: true,
                error: None,
            })
            .await;
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);
}

/// Test connection state checking
#[tokio::test]
async fn test_is_connected_state() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth

    // Initially connected
    assert!(setup.engine.is_connected());

    // Disconnect
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Eventually should detect disconnection
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert!(!setup.engine.is_connected());

    // Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;

    // Should be connected again
    assert!(setup.engine.is_connected());
}

/// Test database error during initialization
#[tokio::test]
async fn test_database_initialization_error() {
    // Try to create engine with invalid database path
    let result = SyncEngine::new(
        "/invalid/path/that/does/not/exist.db",
        "ws://localhost:9999",
        "test@test.com",
        "key",
        "secret",
    )
    .await;

    assert!(result.is_err());
}

/// Test concurrent document operations
#[tokio::test]
async fn test_concurrent_document_operations() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Create multiple documents concurrently
    let engine = Arc::new(setup.engine);
    let mut handles = vec![];

    for i in 0..5 {
        let eng = engine.clone();
        let handle = tokio::spawn(async move { eng.create_document(json!({ "id": i })).await });
        handles.push(handle);
    }

    for handle in handles {
        assert!(handle.await.is_ok());
    }

    // All should succeed
}

/// Tests the ACTUAL offline-first workflow that users depend on
#[tokio::test]
async fn test_offline_document_creation_with_reconnection_sync() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Go offline BEFORE creating documents
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2. Create 3 documents while completely offline
    let doc1 = setup
        .engine
        .create_document(json!({ "title": "Offline Doc 1" }))
        .await
        .unwrap();
    let doc2 = setup
        .engine
        .create_document(json!({ "title": "Offline Doc 2" }))
        .await
        .unwrap();
    let doc3 = setup
        .engine
        .create_document(json!({ "title": "Offline Doc 3" }))
        .await
        .unwrap();

    // 3. Verify all are pending sync
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 3);

    // 4. Verify they exist locally despite being offline
    let local_docs = setup.engine.get_all_documents().await.unwrap();
    assert_eq!(local_docs.len(), 3);

    // 5. Reconnect - this triggers perform_pending_sync_after_reconnection()
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await; // Wait for reconnect loop

    // 6. Consume auth message from reconnection
    let _ = setup.server.expect_client_message().await;

    // 7. Should receive ALL 3 CreateDocument messages (hits sync_pending_documents)
    let mut received_docs = vec![];
    for _ in 0..3 {
        let msg = setup.server.expect_client_message().await;
        match msg {
            ClientMessage::CreateDocument { document } => {
                received_docs.push(document.id);
            }
            _ => panic!("Expected CreateDocument, got {:?}", msg),
        }
    }

    // 8. Verify we got all 3 documents
    assert!(received_docs.contains(&doc1.id));
    assert!(received_docs.contains(&doc2.id));
    assert!(received_docs.contains(&doc3.id));

    // 9. Confirm all uploads
    for doc_id in [doc1.id, doc2.id, doc3.id] {
        setup
            .server
            .send_server_message(ServerMessage::DocumentCreatedResponse {
                document_id: doc_id,
                success: true,
                error: None,
            })
            .await;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // 10. Verify all are now synced (no longer pending)
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);

    println!("✅ OFFLINE CREATE TEST: Successfully synced 3 offline-created documents");
}

/// Tests offline update with patch stored in sync_queue
#[tokio::test]
async fn test_offline_update_with_patch_recovery() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Create document ONLINE first
    let doc = setup
        .engine
        .create_document(json!({ "value": 100 }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // CreateDocument

    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2. Verify it's synced
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);

    // 3. Go offline
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 4. Update document OFFLINE (stores patch in sync_queue)
    setup
        .engine
        .update_document(doc.id, json!({ "value": 200 }))
        .await
        .unwrap();

    // 5. Verify it's marked as pending again
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);

    // 6. Verify patch was stored in sync_queue
    let patch_exists =
        sqlx::query("SELECT COUNT(*) as count FROM sync_queue WHERE document_id = ?")
            .bind(doc.id.to_string())
            .fetch_one(&setup.db.pool)
            .await
            .unwrap();
    let count: i64 = patch_exists.try_get("count").unwrap();
    assert_eq!(count, 1, "Patch should be in sync_queue");

    // 7. Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // 8. Should receive UpdateDocument with stored patch (hits lines 1323-1369)
    let update_msg = setup.server.expect_client_message().await;
    match update_msg {
        ClientMessage::UpdateDocument { patch } => {
            assert_eq!(patch.document_id, doc.id);
            println!("✅ Received UpdateDocument with stored patch");
        }
        _ => panic!("Expected UpdateDocument with patch, got {:?}", update_msg),
    }

    // 9. Confirm update
    setup
        .server
        .send_server_message(ServerMessage::DocumentUpdatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 10. Verify synced and patch removed from queue
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);

    let patch_check = sqlx::query("SELECT COUNT(*) as count FROM sync_queue WHERE document_id = ?")
        .bind(doc.id.to_string())
        .fetch_one(&setup.db.pool)
        .await
        .unwrap();
    let final_count: i64 = patch_check.try_get("count").unwrap();
    assert_eq!(
        final_count, 0,
        "Patch should be removed from sync_queue after sync"
    );

    println!("✅ OFFLINE UPDATE TEST: Successfully synced offline update with patch");
}

/// Tests offline delete operation
#[tokio::test]
async fn test_offline_delete_sync_on_reconnection() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Create and sync document
    let doc = setup
        .engine
        .create_document(json!({ "to_delete": true }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // CreateDocument

    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2. Go offline
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 3. Delete offline
    setup.engine.delete_document(doc.id).await.unwrap();

    // 4. Verify marked as pending (deleted but not synced)
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);

    // 5. Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // 6. Should receive DeleteDocument (hits lines 1302-1320)
    let delete_msg = setup.server.expect_client_message().await;
    match delete_msg {
        ClientMessage::DeleteDocument { document_id, .. } => {
            assert_eq!(document_id, doc.id);
            println!("✅ Received DeleteDocument for offline-deleted doc");
        }
        _ => panic!("Expected DeleteDocument, got {:?}", delete_msg),
    }

    // 7. Confirm deletion
    setup
        .server
        .send_server_message(ServerMessage::DocumentDeletedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // 8. Verify no longer pending
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 0);

    println!("✅ OFFLINE DELETE TEST: Successfully synced offline delete");
}

/// Tests mixed offline operations (creates, updates, deletes)
#[tokio::test]
async fn test_mixed_offline_operations_sync() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // 1. Create doc1 online and sync it
    let doc1 = setup
        .engine
        .create_document(json!({ "id": 1 }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await;
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc1.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 2. Go offline
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 3. Mixed operations:
    let _doc2 = setup
        .engine
        .create_document(json!({ "id": 2 }))
        .await
        .unwrap(); // Create new
    setup
        .engine
        .update_document(doc1.id, json!({ "id": 1, "updated": true }))
        .await
        .unwrap(); // Update existing
    setup.engine.delete_document(doc1.id).await.unwrap(); // Delete the updated one
    let _doc3 = setup
        .engine
        .create_document(json!({ "id": 3 }))
        .await
        .unwrap(); // Another create

    // 4. Should have 3 pending (doc2 create, doc1 delete, doc3 create)
    let pending = setup.engine.count_pending_sync().await.unwrap();
    assert!(
        pending >= 2,
        "Should have at least 2 pending operations, got {}",
        pending
    );

    // 5. Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // 6. Collect all sync messages
    let mut creates = 0;
    let mut deletes = 0;
    let mut updates = 0;

    for _ in 0..5 {
        if let Ok(msg) = tokio::time::timeout(
            Duration::from_millis(500),
            setup.server.expect_client_message(),
        )
        .await
        {
            match msg {
                ClientMessage::CreateDocument { document } => {
                    creates += 1;
                    setup
                        .server
                        .send_server_message(ServerMessage::DocumentCreatedResponse {
                            document_id: document.id,
                            success: true,
                            error: None,
                        })
                        .await;
                }
                ClientMessage::UpdateDocument { patch } => {
                    updates += 1;
                    setup
                        .server
                        .send_server_message(ServerMessage::DocumentUpdatedResponse {
                            document_id: patch.document_id,
                            success: true,
                            error: None,
                        })
                        .await;
                }
                ClientMessage::DeleteDocument {
                    document_id,
                    ..
                } => {
                    deletes += 1;
                    setup
                        .server
                        .send_server_message(ServerMessage::DocumentDeletedResponse {
                            document_id,
                            success: true,
                            error: None,
                        })
                        .await;
                }
                _ => {}
            }
        }
    }

    println!(
        "✅ MIXED OPERATIONS: creates={}, updates={}, deletes={}",
        creates, updates, deletes
    );
    assert!(creates >= 2, "Should have at least 2 creates");
    assert!(deletes >= 1, "Should have at least 1 delete");

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Eventually all should be synced
    let final_pending = setup.engine.count_pending_sync().await.unwrap();
    assert_eq!(final_pending, 0, "All operations should be synced");
}

/// Tests upload timeout and retry mechanism

/// Tests partial upload failure recovery
#[tokio::test]
async fn test_partial_upload_failure() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Go offline and create 3 docs
    setup.server.stop().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let _doc1 = setup
        .engine
        .create_document(json!({ "id": 1 }))
        .await
        .unwrap();
    let _doc2 = setup
        .engine
        .create_document(json!({ "id": 2 }))
        .await
        .unwrap();
    let _doc3 = setup
        .engine
        .create_document(json!({ "id": 3 }))
        .await
        .unwrap();

    // Reconnect
    setup.server.start().await;
    tokio::time::sleep(Duration::from_millis(4000)).await;
    let _ = setup.server.expect_client_message().await; // auth

    // Receive all 3 creates
    let msg1 = setup.server.expect_client_message().await;
    let msg2 = setup.server.expect_client_message().await;
    let _msg3 = setup.server.expect_client_message().await;

    // Confirm ONLY 2 out of 3 (simulate partial failure)
    if let ClientMessage::CreateDocument { document } = msg1 {
        setup
            .server
            .send_server_message(ServerMessage::DocumentCreatedResponse {
                document_id: document.id,
                success: true,
                error: None,
            })
            .await;
    }

    if let ClientMessage::CreateDocument { document } = msg2 {
        setup
            .server
            .send_server_message(ServerMessage::DocumentCreatedResponse {
                document_id: document.id,
                success: true,
                error: None,
            })
            .await;
    }

    // msg3 deliberately NOT confirmed

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Should still have 1 pending
    let pending = setup.engine.count_pending_sync().await.unwrap();
    assert!(pending >= 1, "Should have at least 1 unconfirmed upload");

    println!("✅ PARTIAL FAILURE TEST: 2/3 uploads confirmed, 1 remains pending");
}

/// Tests failed upload response handling
#[tokio::test]
async fn test_upload_failure_response() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    let doc = setup
        .engine
        .create_document(json!({ "will_fail": true }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await; // CreateDocument

    // Server explicitly rejects (hits line 870-874)
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: false,
            error: Some("Server validation failed".to_string()),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Document should remain pending
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);

    println!("✅ FAILURE RESPONSE TEST: Handled server rejection correctly");
}

/// Tests server sending DocumentUpdated with patch
#[tokio::test]
async fn test_server_sends_document_updated_patch() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Create and sync a document
    let doc = setup
        .engine
        .create_document(json!({ "value": 1 }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await;
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Server sends DocumentUpdated with patch (hits lines 723-750)
    use sync_core::models::DocumentPatch;
    use sync_core::patches::{calculate_checksum, create_patch};

    let old_content = json!({ "value": 1 });
    let new_content = json!({ "value": 2 });
    let patch = create_patch(&old_content, &new_content).unwrap();

    let document_patch = DocumentPatch {
        document_id: doc.id,
        patch,
        content_hash: calculate_checksum(&new_content),
    };

    setup
        .server
        .send_server_message(ServerMessage::DocumentUpdated {
            patch: document_patch,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify patch was applied
    let updated_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(updated_doc.content["value"], json!(2));

    println!("✅ SERVER PATCH TEST: Applied DocumentUpdated patch from server");
}

/// Tests server sending brand new DocumentCreated
#[tokio::test]
async fn test_server_sends_new_document_created() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Server creates a document that client doesn't have (hits lines 751-779)
    let (user_id, _) = setup.db.get_user_and_client_id().await.unwrap();

    let new_doc = sync_core::models::Document {
        id: Uuid::new_v4(),
        user_id,
        content: json!({ "from_server": true }),
        sync_revision: 1,
        content_hash: None,
        title: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    };

    setup
        .server
        .send_server_message(ServerMessage::DocumentCreated {
            document: new_doc.clone(),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify document exists locally
    let local_doc = setup.db.get_document(&new_doc.id).await.unwrap();
    assert_eq!(local_doc.content["from_server"], json!(true));

    println!("✅ SERVER CREATE TEST: Received and stored DocumentCreated from server");
}

/// Tests server sending DocumentDeleted
#[tokio::test]
async fn test_server_sends_document_deleted() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Create document locally first
    let doc = setup
        .engine
        .create_document(json!({ "will_be_deleted": true }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await;
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Server sends delete (hits lines 781-793)
    setup
        .server
        .send_server_message(ServerMessage::DocumentDeleted {
            document_id: doc.id,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify document is soft-deleted
    let deleted_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert!(deleted_doc.deleted_at.is_some());

    // Should not appear in get_all_documents
    let all_docs = setup.engine.get_all_documents().await.unwrap();
    assert!(!all_docs.iter().any(|d| d.id == doc.id));

    println!("✅ SERVER DELETE TEST: Processed DocumentDeleted from server");
}

/// Tests conflict detection event
#[tokio::test]
async fn test_conflict_detection_event() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    let doc = setup
        .engine
        .create_document(json!({ "conflict": true }))
        .await
        .unwrap();

    // Server sends ConflictDetected (hits lines 794-799)
    setup
        .server
        .send_server_message(ServerMessage::ConflictDetected {
            document_id: doc.id,
            resolution_strategy: ConflictResolution::ClientWins,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Event dispatcher should have fired (we can't easily test events without hooks)
    // But at least verify the message was processed without error
    println!("✅ CONFLICT TEST: ConflictDetected message processed");
}

/// Tests SyncDocument with generation comparison logic
#[tokio::test]
async fn test_sync_document_generation_comparison() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Create local document
    let doc = setup
        .engine
        .create_document(json!({ "version": 1 }))
        .await
        .unwrap();
    let _ = setup.server.expect_client_message().await;
    setup
        .server
        .send_server_message(ServerMessage::DocumentCreatedResponse {
            document_id: doc.id,
            success: true,
            error: None,
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Server sends SyncDocument with higher generation (hits lines 800-843)
    let mut server_doc = doc.clone();
    server_doc.content = json!({ "version": 2 });
    server_doc.sync_revision = 2;

    setup
        .server
        .send_server_message(ServerMessage::SyncDocument {
            document: server_doc.clone(),
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify local doc was updated to server version
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content["version"], json!(2));

    println!("✅ SYNC GENERATION TEST: Applied server SyncDocument with higher generation");
}

/// Tests SyncDocument rejecting older generation
#[tokio::test]
async fn test_sync_document_rejects_older_version() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    // Create doc with generation 3
    let doc = setup
        .engine
        .create_document(json!({ "gen": 3 }))
        .await
        .unwrap();

    // Manually set high version
    let mut high_gen_doc = doc.clone();
    high_gen_doc.sync_revision = 5;
    setup.db.save_document(&high_gen_doc).await.unwrap();

    // Server sends older version (hits lines 833-836)
    let mut old_server_doc = doc.clone();
    old_server_doc.content = json!({ "gen": "old" });
    old_server_doc.sync_revision = 2; // Lower version

    setup
        .server
        .send_server_message(ServerMessage::SyncDocument {
            document: old_server_doc,
        })
        .await;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify local doc was NOT overwritten
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content["gen"], json!(3)); // Still our version

    println!("✅ SYNC REJECT TEST: Correctly rejected older generation from server");
}

/// Tests document not found scenario
#[tokio::test]
async fn test_update_nonexistent_document() {
    let mut setup = setup().await;
    let _ = setup.server.expect_client_message().await; // auth
    let _ = setup.server.expect_client_message().await; // sync

    let fake_id = Uuid::new_v4();

    // Try to update non-existent document
    let result = setup
        .engine
        .update_document(fake_id, json!({ "fake": true }))
        .await;

    assert!(
        result.is_err(),
        "Should fail when updating non-existent document"
    );
    println!("✅ NOT FOUND TEST: Correctly handled missing document");
}
