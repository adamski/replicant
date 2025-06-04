use clap::Parser;
use colored::*;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use serde_json::Value;
use sqlx::Row;
use sync_client::{ClientDatabase, SyncEngine};
use sync_core::models::Document;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "sync-client")]
#[command(about = "Interactive JSON database sync client", long_about = None)]
struct Cli {
    /// Database file name (will auto-create in databases/ directory)
    #[arg(short, long, default_value = "alice")]
    database: String,

    /// Server WebSocket URL
    #[arg(short, long, default_value = "ws://localhost:8080/ws")]
    server: String,

    /// Authentication token
    #[arg(short, long, default_value = "demo-token")]
    token: String,

    /// User ID (will be generated if not provided)
    #[arg(short, long)]
    user_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging (only show warnings and errors)
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .init();

    let cli = Cli::parse();

    // Auto-create database file path with .sqlite3 extension in databases/ folder
    std::fs::create_dir_all("databases")?;
    let db_file = format!("databases/{}.sqlite3", cli.database);
    let db_url = format!("sqlite:{}?mode=rwc", db_file);
    
    println!("{}", "🚀 JSON Database Sync Client".bold().cyan());
    println!("{}", "============================".cyan());
    println!("📁 Database: {}", db_file.green());

    // Initialize database (will auto-create the file)
    let db = ClientDatabase::new(&db_url).await?;
    
    // Run migrations
    db.run_migrations().await?;

    // Get or create user
    let user_id = match cli.user_id {
        Some(id) => Uuid::parse_str(&id)?,
        None => {
            match db.get_user_id().await {
                Ok(id) => id,
                Err(_) => {
                    let id = Uuid::new_v4();
                    println!("🆕 Creating new user: {}", id.to_string().yellow());
                    let client_id = Uuid::new_v4();
                    setup_user(&db, id, client_id, &cli.server, &cli.token).await?;
                    id
                }
            }
        }
    };

    println!("👤 User ID: {}", user_id.to_string().green());
    println!("🌐 Server: {}", cli.server.blue());
    println!();

    // Try to connect to server
    let sync_engine = match SyncEngine::new(&db_url, &cli.server, &cli.token).await {
        Ok(mut engine) => {
            println!("✅ Connected to sync server!");
            
            // Start the sync engine
            if let Err(e) = engine.start().await {
                println!("⚠️  Failed to start sync engine: {}", e.to_string().yellow());
                None
            } else {
                Some(engine)
            }
        }
        Err(e) => {
            println!("⚠️  Offline mode: {}", e.to_string().yellow());
            None
        }
    };

    // Main interactive loop
    loop {
        let choices = vec![
            "📄 List documents",
            "➕ Create new document",
            "✏️  Edit document",
            "🔍 View document",
            "🗑️  Delete document",
            "🔄 Sync status",
            "❌ Exit",
        ];

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("What would you like to do?")
            .items(&choices)
            .default(0)
            .interact()?;

        match selection {
            0 => list_documents(&db, user_id).await?,
            1 => create_document(&db, &sync_engine, user_id).await?,
            2 => edit_document(&db, &sync_engine, user_id).await?,
            3 => view_document(&db, user_id).await?,
            4 => delete_document(&db, &sync_engine, user_id).await?,
            5 => show_sync_status(&db).await?,
            6 => {
                if Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Are you sure you want to exit?")
                    .default(false)
                    .interact()?
                {
                    println!("👋 Goodbye!");
                    break;
                }
            }
            _ => unreachable!(),
        }
        println!();
    }

    Ok(())
}


async fn setup_user(
    db: &ClientDatabase,
    user_id: Uuid,
    client_id: Uuid,
    server_url: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id.to_string())
    .bind(client_id.to_string())
    .bind(server_url)
    .bind(token)
    .execute(&db.pool)
    .await?;
    Ok(())
}

async fn list_documents(
    db: &ClientDatabase,
    user_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query(
        r#"
        SELECT id, title, sync_status, updated_at 
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        "#,
    )
    .bind(user_id.to_string())
    .fetch_all(&db.pool)
    .await?;

    if rows.is_empty() {
        println!("📭 No documents found.");
    } else {
        println!("{}", "📚 Your Documents:".bold());
        println!("{}", "─".repeat(80).dimmed());
        
        for row in rows {
            let id = row.try_get::<String, _>("id")?;
            let title = row.try_get::<String, _>("title")?;
            let sync_status = row.try_get::<Option<String>, _>("sync_status")?;
            let updated_at = row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?;
            
            let status_icon = match sync_status.as_deref() {
                Some("synced") => "✅",
                Some("pending") => "⏳",
                Some("conflict") => "⚠️",
                _ => "❓",
            };
            
            println!(
                "{} {} {} {}",
                status_icon,
                id.blue(),
                title.white().bold(),
                format!("({})", updated_at).dimmed()
            );
        }
        println!("{}", "─".repeat(80).dimmed());
    }

    Ok(())
}

async fn create_document(
    db: &ClientDatabase,
    sync_engine: &Option<SyncEngine>,
    user_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let title: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Document title")
        .interact_text()?;

    println!("📝 Enter JSON content (or press Enter for template):");
    let content_str: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("JSON")
        .default(r#"{"text": "New document", "tags": []}"#.to_string())
        .interact_text()?;

    let content: Value = serde_json::from_str(&content_str)?;

    if let Some(engine) = sync_engine {
        // Use sync engine if connected
        let doc = engine.create_document(title.clone(), content).await?;
        println!("✅ Document created: {}", doc.id.to_string().green());
    } else {
        // Offline mode - create locally
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title,
            revision_id: Document::initial_revision(&content),
            content,
            version: 1,
            vector_clock: sync_core::models::VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None
        };
        
        db.save_document(&doc).await?;
        println!("✅ Document created (offline): {}", doc.id.to_string().yellow());
    }

    Ok(())
}

async fn edit_document(
    db: &ClientDatabase,
    sync_engine: &Option<SyncEngine>,
    user_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    // List documents for selection
    let rows = sqlx::query(
        r#"
        SELECT id, title 
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        "#,
    )
    .bind(user_id.to_string())
    .fetch_all(&db.pool)
    .await?;

    if rows.is_empty() {
        println!("📭 No documents to edit.");
        return Ok(());
    }

    let mut doc_info = Vec::new();
    let mut choices = Vec::new();
    
    for row in rows {
        let id = row.try_get::<String, _>("id")?;
        let title = row.try_get::<String, _>("title")?;
        doc_info.push((id.clone(), title.clone()));
        choices.push(format!("{} - {}", id, title));
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select document to edit")
        .items(&choices)
        .interact()?;

    let doc_id = Uuid::parse_str(&doc_info[selection].0)?;
    let doc = db.get_document(&doc_id).await?;

    println!("Current content:");
    println!("{}", serde_json::to_string_pretty(&doc.content)?.dimmed());

    let new_content_str: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("New JSON content")
        .default(serde_json::to_string(&doc.content)?)
        .interact_text()?;

    let new_content: Value = serde_json::from_str(&new_content_str)?;

    if let Some(engine) = sync_engine {
        engine.update_document(doc_id, new_content).await?;
        println!("✅ Document updated and synced!");
    } else {
        // Offline update
        let mut updated_doc = doc;
        updated_doc.revision_id = updated_doc.next_revision(&new_content);
        updated_doc.content = new_content;
        updated_doc.version += 1;
        updated_doc.updated_at = chrono::Utc::now();
        
        db.save_document(&updated_doc).await?;
        
        // Mark as pending sync
        sqlx::query("UPDATE documents SET sync_status = 'pending' WHERE id = ?1")
            .bind(doc_id.to_string())
            .execute(&db.pool)
            .await?;
        
        println!("✅ Document updated (offline)");
    }

    Ok(())
}

async fn view_document(
    db: &ClientDatabase,
    user_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query(
        r#"
        SELECT id, title 
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        "#,
    )
    .bind(user_id.to_string())
    .fetch_all(&db.pool)
    .await?;

    if rows.is_empty() {
        println!("📭 No documents to view.");
        return Ok(());
    }

    let mut doc_info = Vec::new();
    let mut choices = Vec::new();
    
    for row in rows {
        let id = row.try_get::<String, _>("id")?;
        let title = row.try_get::<String, _>("title")?;
        doc_info.push((id.clone(), title.clone()));
        choices.push(format!("{} - {}", id, title));
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select document to view")
        .items(&choices)
        .interact()?;

    let doc_id = Uuid::parse_str(&doc_info[selection].0)?;
    let doc = db.get_document(&doc_id).await?;

    println!("{}", "📄 Document Details".bold());
    println!("{}", "─".repeat(80).dimmed());
    println!("ID:         {}", doc.id.to_string().blue());
    println!("Title:      {}", doc.title.white().bold());
    println!("Version:    {}", doc.version.to_string().yellow());
    println!("Created:    {}", doc.created_at.to_string().dimmed());
    println!("Updated:    {}", doc.updated_at.to_string().dimmed());
    println!("Content:");
    println!("{}", serde_json::to_string_pretty(&doc.content)?.green());
    println!("{}", "─".repeat(80).dimmed());

    Ok(())
}

async fn delete_document(
    db: &ClientDatabase,
    sync_engine: &Option<SyncEngine>,
    user_id: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    // List documents for selection
    let rows = sqlx::query(
        r#"
        SELECT id, title 
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        "#,
    )
    .bind(user_id.to_string())
    .fetch_all(&db.pool)
    .await?;

    if rows.is_empty() {
        println!("📭 No documents to delete.");
        return Ok(());
    }

    let mut doc_info = Vec::new();
    let mut choices = Vec::new();
    
    for row in rows {
        let id = row.try_get::<String, _>("id")?;
        let title = row.try_get::<String, _>("title")?;
        doc_info.push((id.clone(), title.clone()));
        choices.push(format!("{} - {}", id, title));
    }

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select document to delete")
        .items(&choices)
        .interact()?;

    let doc_id = Uuid::parse_str(&doc_info[selection].0)?;
    let doc_title = &doc_info[selection].1;

    // Confirm deletion
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(&format!("Are you sure you want to delete '{}'?", doc_title))
        .default(false)
        .interact()?
    {
        println!("❌ Deletion cancelled.");
        return Ok(());
    }

    if let Some(engine) = sync_engine {
        engine.delete_document(doc_id).await?;
        println!("✅ Document deleted and synced!");
    } else {
        // Offline delete - just mark as deleted locally
        db.delete_document(&doc_id).await?;
        println!("✅ Document deleted (offline)");
    }

    Ok(())
}

async fn show_sync_status(db: &ClientDatabase) -> Result<(), Box<dyn std::error::Error>> {
    let pending_row = sqlx::query(
        "SELECT COUNT(*) as count FROM documents WHERE sync_status = 'pending'"
    )
    .fetch_one(&db.pool)
    .await?;
    let pending_count = pending_row.try_get::<i64, _>("count")?;

    let conflict_row = sqlx::query(
        "SELECT COUNT(*) as count FROM documents WHERE sync_status = 'conflict'"
    )
    .fetch_one(&db.pool)
    .await?;
    let conflict_count = conflict_row.try_get::<i64, _>("count")?;

    println!("{}", "🔄 Sync Status".bold());
    println!("{}", "─".repeat(40).dimmed());
    println!("⏳ Pending sync:  {}", pending_count.to_string().yellow());
    println!("⚠️  Conflicts:     {}", conflict_count.to_string().red());
    println!("{}", "─".repeat(40).dimmed());

    Ok(())
}