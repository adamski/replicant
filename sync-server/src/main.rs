use axum::{
    Router,
    routing::{get, post},
    extract::{ws::WebSocketUpgrade, State},
    response::Response,
};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use dashmap::DashMap;
use sync_server::{
    database::ServerDatabase,
    auth::AuthState,
    monitoring::{self, MonitoringLayer},
    websocket::handle_websocket,
    api,
    AppState,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "sync-server")]
#[command(about = "Sync server with built-in credential management")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate new API credentials
    GenerateCredentials {
        /// Optional name for the credential set (e.g., "Production", "Staging")
        #[arg(short, long, default_value = "Default")]
        name: String,
    },
    /// Start the sync server
    Serve,
}

#[tokio::main]
async fn main() -> sync_core::SyncResult<()> {
    // Parse command line arguments
    let cli = Cli::parse();

    // Handle different commands
    match cli.command {
        Some(Commands::GenerateCredentials { name }) => {
            generate_credentials(&name).await
        }
        Some(Commands::Serve) | None => {
            // Default to serve if no command specified (backward compatibility)
            run_server().await
        }
    }
}

async fn generate_credentials(name: &str) -> sync_core::SyncResult<()> {
    use colored::*;

    // Initialize database connection
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://localhost:5432/sync_db".to_string());

    let db = Arc::new(ServerDatabase::new(&database_url).await?);

    // Run migrations to ensure api_credentials table exists
    db.run_migrations().await?;

    let auth = AuthState::new(db);

    // Generate credentials
    let credentials = AuthState::generate_api_credentials();

    // Save to database
    auth.save_credentials(&credentials, name).await?;

    // Display credentials
    println!("{}", "========================================".cyan());
    println!("{}", "API Credentials Generated Successfully".bold().green());
    println!("{}", "========================================".cyan());
    println!("Name:       {}", name.bold());
    println!();
    println!("API Key:    {}", credentials.api_key.yellow());
    println!("Secret:     {}", credentials.secret.yellow());
    println!();
    println!("{}", "⚠️  IMPORTANT: Save these credentials securely!".bold().red());
    println!("{}", "The secret will NEVER be shown again.".red());
    println!();
    println!("{}", "These credentials authenticate your APPLICATION.".cyan());
    println!("{}", "End users will still provide their user_id when connecting.".cyan());
    println!();
    println!("{}", "Add to your client application:".bold());
    println!("{}", "----------------------------------------".cyan());
    println!("const API_KEY = \"{}\";", credentials.api_key);
    println!("const API_SECRET = \"{}\";", credentials.secret);
    println!("{}", "========================================".cyan());

    Ok(())
}

async fn run_server() -> sync_core::SyncResult<()> {
    // Check if monitoring mode is enabled
    let monitoring_enabled = std::env::var("MONITORING").unwrap_or_default() == "true";

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("sync_server=debug,tower_http=debug")
        .init();
    
    // Print startup banner if monitoring is enabled
    if monitoring_enabled {
        use colored::*;
        tracing::info!("{}", "🚀 Sync Server with Monitoring".bold().cyan());
        tracing::info!("{}", "==============================".cyan());
        tracing::info!("");
    }
    // Database connection
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "http://localhost:5432/sync_db".to_string());

    let db = match ServerDatabase::new(&database_url).await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            tracing::error!(%e, "Failed to initialize database");
            return Ok(());
        }
    };

    if let Err(e) = db.run_migrations().await {
        tracing::error!(%e, "Failed to run migrations");
        return Ok(());
    }
    
    // Set up monitoring if enabled
    let monitoring_layer = if monitoring_enabled {
        let (tx, rx) = tokio::sync::mpsc::channel(1000);
        monitoring::spawn_monitoring_display(rx).await;
        Some(MonitoringLayer::new(tx))
    } else {
        None
    };
    
    // Application state
    let app_state = Arc::new(AppState {
        db: db.clone(),
        auth: AuthState::new(db),
        monitoring: monitoring_layer,
        clients: Arc::new(DashMap::new()),
        user_clients: Arc::new(DashMap::new()),
    });
    
    // Build router
    let app = Router::new()
        // WebSocket endpoint
        .route("/ws", get(websocket_handler))
        // REST API (partially disabled - update to use HMAC authentication)
        .route("/api/auth/create-user", post(api::create_user))
        // TODO: Re-enable these after updating to use HMAC
        // .route("/api/auth/create-api-key", post(api::create_api_key))
        // .route("/api/documents", get(api::list_documents))
        // .route("/api/documents/:id", get(api::get_document))
        // Health check
        .route("/health", get(|| async { "OK" }))
        .route("/test/reset", post(reset_server_state))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);
    
    let addr = std::env::var("BIND_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    
    tracing::info!("Starting sync server on {}", addr);
    
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(%e, addr=%addr);
            return Ok(());
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(%e, addr=%addr);
    }
    
    Ok(())
}

// AppState is now defined in lib.rs

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn reset_server_state(State(state): State<Arc<AppState>>) -> &'static str {
    // Clear all in-memory state for testing
    tracing::info!("Resetting server state for testing");
    
    // Clear the client registry
    state.clients.clear();
    state.user_clients.clear();
    
    // TODO: Could also reset other in-memory state here
    
    "Server state reset"
}