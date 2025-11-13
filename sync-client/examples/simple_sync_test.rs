use serde_json::json;
use std::env;
use std::sync::Arc;
use sync_client::SyncEngine;
use tracing::{debug, info};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "info,sync_client=debug,sync_core=debug".to_string()),
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
        eprintln!("  daemon --database <path> --user <email>");
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

    info!(
        "Starting simple_sync_test: action={}, database={}, user={}",
        action, database_path, user_email
    );

    // Create sync engine (auto-starts with built-in reconnection)
    debug!("Creating sync engine for database: {}", database_path);
    let engine = SyncEngine::new(
        &database_path,
        "ws://localhost:8080/ws",
        &user_email,
        "test-key",
        "test-secret",
    )
    .await?;

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
            info!("Successfully created document: {}", document.id);
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
                    let title = doc
                        .content
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("No title");
                    let desc = doc
                        .content
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    println!(
                        "Document: {} | Title: {} | Description: {}",
                        doc.id, title, desc
                    );
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
                let title = doc
                    .content
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("No title");

                println!("  {} | {}", doc.id, title);
            }

            // Allow extra time for potential incoming sync messages (like auto-sync after reconnection)
            info!("Waiting for potential incoming sync updates...");
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }

        "daemon" => {
            // Daemon mode - keep sync engine alive and process commands from stdin
            info!("Starting daemon mode - sync engine will stay alive");

            // Create a shared engine reference that can be used across async tasks
            let engine = Arc::new(engine);
            let engine_for_stdin = engine.clone();

            println!("DAEMON_READY");

            // Process stdin commands in a separate async task
            let stdin_task = tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};

                let stdin = tokio::io::stdin();
                let reader = BufReader::new(stdin);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();

                    if line.is_empty() {
                        continue;
                    }

                    if line == "QUIT" {
                        info!("Received QUIT command, exiting daemon");
                        break;
                    }

                    let parts: Vec<&str> = line.split(':').collect();
                    match parts.get(0) {
                        Some(&"CREATE") => {
                            if parts.len() >= 3 {
                                let title = parts[1];
                                let desc = parts.get(2).unwrap_or(&"");
                                let content = json!({
                                    "title": title,
                                    "description": desc,
                                    "priority": "medium",
                                    "tags": ["daemon"]
                                });

                                let create_future = engine_for_stdin.create_document(content);
                                match tokio::time::timeout(
                                    std::time::Duration::from_secs(5),
                                    create_future,
                                )
                                .await
                                {
                                    Ok(Ok(doc)) => println!("RESPONSE:CREATED:{}", doc.id),
                                    Ok(Err(e)) => println!("RESPONSE:ERROR:Create failed: {}", e),
                                    Err(_) => println!("RESPONSE:ERROR:Create timeout"),
                                }
                            } else {
                                println!("RESPONSE:ERROR:CREATE requires title:description");
                            }
                        }
                        Some(&"UPDATE") => {
                            if parts.len() >= 4 {
                                let doc_id_str = parts[1];
                                let title = parts[2];
                                let desc = parts.get(3).unwrap_or(&"");

                                match Uuid::parse_str(doc_id_str) {
                                    Ok(doc_id) => {
                                        let content = json!({
                                            "title": title,
                                            "description": desc,
                                            "updated": true
                                        });

                                        let update_future =
                                            engine_for_stdin.update_document(doc_id, content);
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(5),
                                            update_future,
                                        )
                                        .await
                                        {
                                            Ok(Ok(_)) => println!("RESPONSE:UPDATED:{}", doc_id),
                                            Ok(Err(e)) => {
                                                println!("RESPONSE:ERROR:Update failed: {}", e)
                                            }
                                            Err(_) => println!("RESPONSE:ERROR:Update timeout"),
                                        }
                                    }
                                    Err(_) => println!("RESPONSE:ERROR:Invalid document ID"),
                                }
                            } else {
                                println!("RESPONSE:ERROR:UPDATE requires doc_id:title:description");
                            }
                        }
                        Some(&"STATUS") => {
                            // Use timeouts to prevent blocking during reconnection
                            let doc_count_future = engine_for_stdin.count_documents();
                            let pending_count_future = engine_for_stdin.count_pending_sync();

                            let doc_count = match tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                doc_count_future,
                            )
                            .await
                            {
                                Ok(Ok(count)) => count,
                                _ => 0, // Return 0 on timeout or error
                            };

                            let pending_count = match tokio::time::timeout(
                                std::time::Duration::from_millis(500),
                                pending_count_future,
                            )
                            .await
                            {
                                Ok(Ok(count)) => count,
                                _ => 0, // Return 0 on timeout or error
                            };

                            let connected = engine_for_stdin.is_connected();
                            println!(
                                "RESPONSE:STATUS:{}:{}:{}",
                                doc_count, pending_count, connected
                            );
                        }
                        Some(&"LIST") => match engine_for_stdin.get_all_documents().await {
                            Ok(docs) => {
                                for doc in docs {
                                    let title = doc
                                        .content
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("No title");
                                    println!("RESPONSE:DOC:{}:{}", doc.id, title);
                                }
                                println!("RESPONSE:LIST_END");
                            }
                            Err(e) => println!("RESPONSE:ERROR:List failed: {}", e),
                        },
                        _ => {
                            println!("RESPONSE:ERROR:Unknown command: {}", line);
                        }
                    }
                }

                info!("Stdin processing task exiting");
            });

            // Wait for the stdin task to complete
            let _ = stdin_task.await;
            info!("Daemon mode exiting");
        }

        _ => {
            eprintln!("Unknown action: {}", action);
            std::process::exit(1);
        }
    }

    Ok(())
}
