use axum::{
    Router,
    routing::{get, post},
    extract::{ws::WebSocketUpgrade, State},
    response::Response,
};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub mod database;
pub mod websocket;
pub mod auth;
pub mod sync_handler;
pub mod api;
pub mod queries;

// Re-export for library usage
pub use database::ServerDatabase;
pub use auth::AuthState;

use websocket::handle_websocket;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("sync_server=debug,tower_http=debug")
        .init();
    
    // Database connection
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "http://localhost:5432/sync_db".to_string());
    
    let db = Arc::new(ServerDatabase::new(&database_url).await?);
    db.run_migrations().await?;
    
    // Application state
    let app_state = Arc::new(AppState {
        db,
        auth: AuthState::new(),
    });
    
    // Build router
    let app = Router::new()
        // WebSocket endpoint
        .route("/ws", get(websocket_handler))
        // REST API
        .route("/api/auth/register", post(api::register))
        .route("/api/auth/login", post(api::login))
        .route("/api/documents", get(api::list_documents))
        .route("/api/documents/:id", get(api::get_document))
        // Health check
        .route("/health", get(|| async { "OK" }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);
    
    let addr = std::env::var("BIND_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    
    tracing::info!("Starting sync server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}

#[derive(Clone)]
struct AppState {
    db: Arc<ServerDatabase>,
    auth: AuthState,
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}