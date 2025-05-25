use axum::{
    Router,
    routing::get,
    extract::{ws::WebSocketUpgrade, State},
    response::Response,
};
use colored::*;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tokio::sync::mpsc;
use chrono::Local;
use sync_server::{database::ServerDatabase, auth::AuthState, AppState};
use sync_core::protocol::{ClientMessage, ServerMessage};

// Combined state for monitoring
#[derive(Clone)]
struct MonitoringState {
    app_state: Arc<AppState>,
    log_tx: mpsc::Sender<LogMessage>,
}

// Custom websocket handler that logs all activity
async fn monitoring_websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<MonitoringState>,
) -> Response {
    ws.on_upgrade(move |socket| monitoring_websocket(socket, state.app_state, state.log_tx))
}

#[derive(Debug, Clone)]
enum LogMessage {
    ClientConnected { client_id: String },
    ClientDisconnected { client_id: String },
    MessageReceived { client_id: String, message: ClientMessage },
    MessageSent { client_id: String, message: ServerMessage },
    PatchApplied { document_id: String, patch: String },
    ConflictDetected { document_id: String },
    Error { message: String },
}

async fn monitoring_websocket(
    socket: axum::extract::ws::WebSocket,
    state: Arc<AppState>,
    log_tx: mpsc::Sender<LogMessage>,
) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};
    
    let client_id = uuid::Uuid::new_v4().to_string();
    let _ = log_tx.send(LogMessage::ClientConnected { 
        client_id: client_id.clone() 
    }).await;

    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(100);
    
    // Spawn task to forward messages to WebSocket
    let client_id_clone = client_id.clone();
    let log_tx_clone = log_tx.clone();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let _ = log_tx_clone.send(LogMessage::MessageSent {
                client_id: client_id_clone.clone(),
                message: msg.clone(),
            }).await;
            
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });
    
    let mut handler = MonitoringSyncHandler::new(
        state.db.clone(), 
        tx.clone(),
        log_tx.clone(),
        client_id.clone(),
    );
    let mut authenticated_user_id = None;
    
    // Handle incoming messages
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(client_msg) => {
                    let _ = log_tx.send(LogMessage::MessageReceived {
                        client_id: client_id.clone(),
                        message: client_msg.clone(),
                    }).await;
                    
                    match client_msg {
                        ClientMessage::Authenticate { user_id, auth_token: _ } => {
                            // Simple auth for demo
                            authenticated_user_id = Some(user_id);
                            handler.set_user_id(user_id);
                            
                            let _ = tx.send(ServerMessage::AuthSuccess {
                                session_id: uuid::Uuid::new_v4(),
                            }).await;
                        }
                        _ => {
                            if authenticated_user_id.is_none() {
                                let _ = tx.send(ServerMessage::AuthError {
                                    reason: "Not authenticated".to_string(),
                                }).await;
                                break;
                            }
                            
                            if let Err(e) = handler.handle_message(client_msg).await {
                                let _ = log_tx.send(LogMessage::Error {
                                    message: e.to_string(),
                                }).await;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = log_tx.send(LogMessage::Error {
                        message: format!("Failed to parse message: {}", e),
                    }).await;
                }
            }
        }
    }
    
    let _ = log_tx.send(LogMessage::ClientDisconnected { 
        client_id: client_id.clone() 
    }).await;
    
    if let Some(user_id) = authenticated_user_id {
        state.db.remove_active_connection(&user_id).await.ok();
    }
}

// Custom sync handler that logs patches
struct MonitoringSyncHandler {
    db: Arc<ServerDatabase>,
    tx: mpsc::Sender<ServerMessage>,
    log_tx: mpsc::Sender<LogMessage>,
    client_id: String,
    user_id: Option<uuid::Uuid>,
}

impl MonitoringSyncHandler {
    fn new(
        db: Arc<ServerDatabase>, 
        tx: mpsc::Sender<ServerMessage>,
        log_tx: mpsc::Sender<LogMessage>,
        client_id: String,
    ) -> Self {
        Self {
            db,
            tx,
            log_tx,
            client_id,
            user_id: None,
        }
    }
    
    fn set_user_id(&mut self, user_id: uuid::Uuid) {
        self.user_id = Some(user_id);
    }
    
    async fn handle_message(&mut self, msg: ClientMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use sync_core::patches::apply_patch;
        
        let user_id = self.user_id.ok_or("Not authenticated")?;
        
        match msg {
            ClientMessage::CreateDocument { document } => {
                if document.user_id != user_id {
                    return Err("Cannot create document for another user".into());
                }
                
                self.db.create_document(&document).await?;
                
                self.tx.send(ServerMessage::DocumentCreated { 
                    document: document.clone() 
                }).await?;
            }
            
            ClientMessage::UpdateDocument { patch } => {
                let mut doc = self.db.get_document(&patch.document_id).await?;
                
                if doc.user_id != user_id {
                    return Err("Cannot update another user's document".into());
                }
                
                // Log the patch
                let patch_json = serde_json::to_string_pretty(&patch.patch)?;
                let _ = self.log_tx.send(LogMessage::PatchApplied {
                    document_id: doc.id.to_string(),
                    patch: patch_json,
                }).await;
                
                // Check for conflicts
                if doc.vector_clock.is_concurrent(&patch.vector_clock) {
                    let _ = self.log_tx.send(LogMessage::ConflictDetected {
                        document_id: doc.id.to_string(),
                    }).await;
                    
                    self.tx.send(ServerMessage::ConflictDetected {
                        document_id: patch.document_id,
                        local_revision: patch.revision_id,
                        server_revision: doc.revision_id,
                        resolution_strategy: sync_core::protocol::ConflictResolution::ServerWins,
                    }).await?;
                    return Ok(());
                }
                
                // Apply patch
                apply_patch(&mut doc.content, &patch.patch)?;
                
                // Update metadata
                doc.revision_id = patch.revision_id;
                doc.version += 1;
                doc.vector_clock.merge(&patch.vector_clock);
                doc.updated_at = chrono::Utc::now();
                
                // Save to database
                self.db.update_document(&doc).await?;
                self.db.create_revision(&doc, Some(&patch.patch)).await?;
                
                // Confirm to sender
                self.tx.send(ServerMessage::DocumentUpdated { patch }).await?;
            }
            
            ClientMessage::RequestFullSync => {
                let documents = self.db.get_user_documents(&user_id).await?;
                
                for doc in &documents {
                    self.tx.send(ServerMessage::SyncDocument { 
                        document: doc.clone() 
                    }).await?;
                }
                
                self.tx.send(ServerMessage::SyncComplete { 
                    synced_count: documents.len() 
                }).await?;
            }
            
            _ => {}
        }
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("sync_server=debug,tower_http=debug")
        .init();
    
    println!("{}", "🚀 Sync Server Monitor".bold().cyan());
    println!("{}", "=====================".cyan());
    println!();
    
    // Database connection
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/sync_db".to_string());
    
    println!("📊 Connecting to database: {}", database_url.blue());
    
    let db = Arc::new(ServerDatabase::new(&database_url).await?);
    
    // Create test user for demo
    let test_user_id = uuid::Uuid::new_v4();
    match db.create_user(&format!("demo_{}@example.com", test_user_id), "demo_hash").await {
        Ok(_) => println!("✅ Created demo user: {}", test_user_id.to_string().green()),
        Err(_) => println!("ℹ️  Demo user may already exist"),
    }
    
    // Application state
    let app_state = Arc::new(AppState {
        db,
        auth: AuthState::new(),
    });
    
    // Create logging channel
    let (log_tx, mut log_rx) = mpsc::channel::<LogMessage>(1000);
    
    // Spawn log display task
    tokio::spawn(async move {
        println!();
        println!("{}", "📋 Activity Log:".bold());
        println!("{}", "─".repeat(80).dimmed());
        
        while let Some(log) = log_rx.recv().await {
            let timestamp = Local::now().format("%H:%M:%S%.3f");
            
            match log {
                LogMessage::ClientConnected { client_id } => {
                    println!(
                        "{} {} Client connected: {}",
                        timestamp.to_string().dimmed(),
                        "→".green().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::ClientDisconnected { client_id } => {
                    println!(
                        "{} {} Client disconnected: {}",
                        timestamp.to_string().dimmed(),
                        "←".red().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::MessageReceived { client_id, message } => {
                    let msg_type = match message {
                        ClientMessage::Authenticate { .. } => "Authenticate",
                        ClientMessage::CreateDocument { .. } => "CreateDocument",
                        ClientMessage::UpdateDocument { .. } => "UpdateDocument",
                        ClientMessage::DeleteDocument { .. } => "DeleteDocument",
                        ClientMessage::RequestSync { .. } => "RequestSync",
                        ClientMessage::RequestFullSync => "RequestFullSync",
                        ClientMessage::Ping => "Ping",
                    };
                    println!(
                        "{} {} {} from {}",
                        timestamp.to_string().dimmed(),
                        "↓".blue(),
                        msg_type.white().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::MessageSent { client_id, message } => {
                    let msg_type = match message {
                        ServerMessage::AuthSuccess { .. } => "AuthSuccess",
                        ServerMessage::AuthError { .. } => "AuthError",
                        ServerMessage::DocumentCreated { .. } => "DocumentCreated",
                        ServerMessage::DocumentUpdated { .. } => "DocumentUpdated",
                        ServerMessage::DocumentDeleted { .. } => "DocumentDeleted",
                        ServerMessage::SyncDocument { .. } => "SyncDocument",
                        ServerMessage::SyncComplete { .. } => "SyncComplete",
                        ServerMessage::ConflictDetected { .. } => "ConflictDetected",
                        ServerMessage::Error { .. } => "Error",
                        ServerMessage::Pong => "Pong",
                    };
                    println!(
                        "{} {} {} to {}",
                        timestamp.to_string().dimmed(),
                        "↑".green(),
                        msg_type.white().bold(),
                        client_id.yellow()
                    );
                }
                LogMessage::PatchApplied { document_id, patch } => {
                    println!(
                        "{} {} Patch applied to document {}:",
                        timestamp.to_string().dimmed(),
                        "🔧".to_string(),
                        document_id.blue()
                    );
                    // Print patch with indentation
                    for line in patch.lines() {
                        println!("     {}", line.cyan());
                    }
                }
                LogMessage::ConflictDetected { document_id } => {
                    println!(
                        "{} {} Conflict detected for document {}",
                        timestamp.to_string().dimmed(),
                        "⚠️".to_string(),
                        document_id.red().bold()
                    );
                }
                LogMessage::Error { message } => {
                    println!(
                        "{} {} Error: {}",
                        timestamp.to_string().dimmed(),
                        "❌".to_string(),
                        message.red()
                    );
                }
            }
        }
    });
    
    // Build router with combined state
    let monitoring_state = MonitoringState {
        app_state,
        log_tx,
    };
    
    let app = Router::new()
        .route("/ws", get(monitoring_websocket_handler))
        .route("/health", get(|| async { "OK" }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(monitoring_state);
    
    let addr = std::env::var("BIND_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    
    println!();
    println!("🌐 Server listening on: {}", addr.green().bold());
    println!("🔌 WebSocket endpoint: {}/ws", format!("ws://{}", addr).blue());
    println!();
    println!("💡 {}", "Tip: Run the interactive client example to see sync in action!".italic().dimmed());
    println!();
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}