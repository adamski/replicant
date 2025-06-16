use std::env;
use serde_json::json;
use uuid::Uuid;
use sync_client::SyncEngine;
use tracing::{info, debug, warn, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,sync_client=debug,sync_core=debug".to_string())
        )
        .init();
    
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("Usage: {} <action> [args...]", args[0]);
        eprintln!("Actions:");
        eprintln!("  create --database <path> --user <email> --title <title> --desc <description>");
        eprintln!("  update --database <path> --user <email> --id <doc_id> --title <title>");
        eprintln!("  delete --database <path> --user <email> --id <doc_id>");
        eprintln!("  list --database <path> --user <email>");
        eprintln!("  sync --database <path> --user <email>");
        eprintln!("  status --database <path> --user <email>");
        std::process::exit(1);
    }
    
    let action = &args[1];
    let mut database_path = String::new();
    let mut user_email = String::new();
    let mut doc_id = None;
    let mut title = String::new();
    let mut description = String::new();
    
    // Parse arguments
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--database" => {
                if i + 1 < args.len() {
                    database_path = format!("sqlite:databases/{}.sqlite3?mode=rwc", args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("--database requires a value");
                    std::process::exit(1);
                }
            }
            "--user" => {
                if i + 1 < args.len() {
                    user_email = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("--user requires a value");
                    std::process::exit(1);
                }
            }
            "--id" => {
                if i + 1 < args.len() {
                    doc_id = Some(Uuid::parse_str(&args[i + 1])?);
                    i += 2;
                } else {
                    eprintln!("--id requires a value");
                    std::process::exit(1);
                }
            }
            "--title" => {
                if i + 1 < args.len() {
                    title = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("--title requires a value");
                    std::process::exit(1);
                }
            }
            "--desc" => {
                if i + 1 < args.len() {
                    description = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("--desc requires a value");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
    }
    
    if database_path.is_empty() || user_email.is_empty() {
        eprintln!("--database and --user are required");
        std::process::exit(1);
    }
    
    info!("Starting simple_sync_test: action={}, database={}, user={}", 
          action, database_path, user_email);
    
    // Create and start sync engine
    debug!("Creating sync engine for database: {}", database_path);
    let mut engine = SyncEngine::new(
        &database_path,
        "ws://localhost:8080/ws",
        "test-token",
        &user_email
    ).await?;
    
    // Only start the engine for sync operations
    if matches!(action.as_str(), "sync" | "create" | "update" | "delete") {
        debug!("Starting sync engine for action: {}", action);
        engine.start().await?;
    }
    
    match action.as_str() {
        "create" => {
            if title.is_empty() {
                eprintln!("--title is required for create");
                std::process::exit(1);
            }
            
            let content = json!({
                "title": title,
                "description": description
            });
            
            debug!("Creating document with content: {:?}", content);
            let document = engine.create_document(content).await?;
            info!("Successfully created document: {} with revision: {}", 
                  document.id, document.revision_id);
            println!("Created document: {}", document.id);
            
            // Allow time for immediate sync to complete before disconnecting
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        
        "update" => {
            let id = doc_id.ok_or("--id is required for update")?;
            if title.is_empty() {
                eprintln!("--title is required for update");
                std::process::exit(1);
            }
            
            let content = json!({
                "title": title,
                "description": description
            });
            
            debug!("Updating document {} with content: {:?}", id, content);
            engine.update_document(id, content).await?;
            
            // Allow time for immediate sync to complete before disconnecting
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            info!("Successfully updated document: {}", id);
            println!("Updated document: {}", id);
        }
        
        "delete" => {
            let id = doc_id.ok_or("--id is required for delete")?;
            engine.delete_document(id).await?;
            
            // Allow time for immediate sync to complete before disconnecting
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            println!("Deleted document: {}", id);
        }
        
        "list" => {
            let documents = engine.get_all_documents().await?;
            if documents.is_empty() {
                println!("No documents found");
            } else {
                for doc in documents {
                    let title = doc.content.get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No title");
                    let desc = doc.content.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    
                    println!("Document: {} | Title: {} | Description: {} | Rev: {}", 
                             doc.id, title, desc, doc.revision_id);
                }
            }
        }
        
        "sync" => {
            info!("Starting sync_all operation");
            let start = std::time::Instant::now();
            engine.sync_all().await?;
            let elapsed = start.elapsed();
            info!("Sync completed in {:?}", elapsed);
            
            // Allow time for sync messages to complete before disconnecting
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            println!("Sync completed");
        }
        
        "status" => {
            let documents = engine.get_all_documents().await?;
            println!("Database: {}", database_path);
            println!("User: {}", user_email);
            println!("Documents: {}", documents.len());
            
            for doc in documents {
                let title = doc.content.get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No title");
                    
                println!("  {} | {} | Rev: {}", doc.id, title, doc.revision_id);
            }
        }
        
        _ => {
            eprintln!("Unknown action: {}", action);
            std::process::exit(1);
        }
    }
    
    Ok(())
}