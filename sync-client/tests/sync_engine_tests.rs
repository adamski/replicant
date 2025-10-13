mod common;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use sync_client::{ClientDatabase, SyncEngine};
use sync_core::protocol::{ClientMessage, ServerMessage};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use uuid::Uuid;

/// A mock WebSocket server to simulate the backend for testing.
/// It allows tests to control the messages sent to the SyncEngine
/// and to inspect messages received from it.
struct MockServer {
    pub addr: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
    // Channel to send server messages to the connected client.
    to_client_tx: mpsc::Sender<ServerMessage>,
    // Channel to receive client messages from the connected client.
    from_client_rx: mpsc::Receiver<ClientMessage>,
}

impl MockServer {
    /// Starts a new mock server on a random available port.
    pub async fn new() -> Self {
        let listener = TcpListener::bind("localhost:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (to_client_tx, mut to_client_rx) = mpsc::channel(100);
        let (from_client_tx, from_client_rx) = mpsc::channel(100);

        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let (mut ws_tx, mut ws_rx) = accept_async(stream).await.unwrap().split();
                // Forward server messages to SyncEngine
                let h1 = tokio::spawn(async move {
                    loop {
                        if let Some(msg) = to_client_rx.recv().await {
                            let json = serde_json::to_string(&msg).unwrap();
                            if let Err(e) = ws_tx.send(Message::Text(json)).await {
                                println!("{:?}", e)
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
        });

        Self {
            addr,
            handle,
            to_client_tx,
            from_client_rx,
        }
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

    let server = MockServer::new().await;
    let server_url = format!("ws://{}", server.addr);
    let user_identifier = "test-user";
    let auth_token = "test-token";

    let engine = SyncEngine::new(
        &format!("file:{}?mode=memory&cache=shared", db_id),
        &server_url,
        auth_token,
        user_identifier,
    )
    .await
    .unwrap();
    TestSetup { engine, server, db }
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
            revision_id: doc.revision_id.clone(),
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
            revision_id: doc.revision_id.clone(),
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

    // 3. Verify an `UpdateDocument` message was sent.
    let update_msg = setup.server.expect_client_message().await;
    match update_msg {
        // This should be a UpdatedDocument message, but for the reasons mentioned in sync_engine.rs
        // line 992 It is still a CreateDocument message
        ClientMessage::CreateDocument { document } => {
            assert_eq!(document.id, doc.id);
            assert_eq!(document.version, 2); // a single 'replace' op
        }
        _ => panic!("Expected UpdateDocument message"),
    }

    // 4. Verify the document is updated locally.
    let local_doc = setup.db.get_document(&doc.id).await.unwrap();
    assert_eq!(local_doc.content, updated_content);
    assert_eq!(setup.engine.count_pending_sync().await.unwrap(), 1);

    setup
        .server
        .send_server_message(ServerMessage::DocumentUpdatedResponse {
            document_id: doc.id,
            revision_id: doc.revision_id.clone(),
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
            revision_id: doc.revision_id.clone(),
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
            revision_id: doc.revision_id.clone(),
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
