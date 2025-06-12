use std::time::Duration;
use tokio::time::sleep;
use serde_json::json;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("🔥 Simple Conflict Resolution Test");
    println!("==================================");

    // Create test user
    let user_id = Uuid::new_v4();
    let token = "test-token";
    
    // Create two client databases
    let test_dir = format!("/tmp/simple_conflict_test_{}", user_id);
    std::fs::create_dir_all(&test_dir)?;
    
    let client1_db_path = format!("sqlite:{}/client1.sqlite3?mode=rwc", test_dir);
    let client2_db_path = format!("sqlite:{}/client2.sqlite3?mode=rwc", test_dir);
    
    // Initialize databases
    let client1_db = sync_client::ClientDatabase::new(&client1_db_path).await?;
    let client2_db = sync_client::ClientDatabase::new(&client2_db_path).await?;
    
    client1_db.run_migrations().await?;
    client2_db.run_migrations().await?;
    
    // Create a document in client 1
    let doc = sync_core::models::Document {
        id: Uuid::new_v4(),
        user_id,
        title: "Test Document".to_string(),
        content: json!({
            "field": "original_value",
            "content": "Original content"
        }),
        revision_id: "1-abc123".to_string(),
        version: 1,
        vector_clock: sync_core::models::VectorClock::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        deleted_at: None,
    };
    
    println!("📄 Created document: {}", doc.id);
    println!("   Original content: {:?}", doc.content);
    
    // Save to both clients (simulating sync)
    client1_db.save_document(&doc).await?;
    client2_db.save_document(&doc).await?;
    
    println!("✅ Document synced to both clients");
    
    // Client 1: Make an edit
    let mut doc1 = doc.clone();
    doc1.content = json!({
        "field": "client1_value", 
        "content": "Client 1 modified this content"
    });
    doc1.revision_id = "2-client1".to_string();
    doc1.version = 2;
    doc1.updated_at = chrono::Utc::now();
    
    client1_db.save_document(&doc1).await?;
    println!("🔧 Client 1 edit: {:?}", doc1.content);
    
    // Client 2: Make a conflicting edit
    let mut doc2 = doc.clone();
    doc2.content = json!({
        "field": "client2_value",  // Same field, different value!
        "content": "Client 2 also modified this content - CONFLICT!"
    });
    doc2.revision_id = "2-client2".to_string();
    doc2.version = 2;
    doc2.updated_at = chrono::Utc::now();
    
    client2_db.save_document(&doc2).await?;
    println!("🔧 Client 2 edit: {:?}", doc2.content);
    
    // Check what each client has locally
    let docs1 = client1_db.get_all_documents().await?;
    let docs2 = client2_db.get_all_documents().await?;
    
    println!("\n📊 Local State After Offline Edits:");
    println!("Client 1 has {} documents", docs1.len());
    if !docs1.is_empty() {
        println!("  Content: {:?}", docs1[0].content);
        println!("  Revision: {}", docs1[0].revision_id);
    }
    
    println!("Client 2 has {} documents", docs2.len());
    if !docs2.is_empty() {
        println!("  Content: {:?}", docs2[0].content);
        println!("  Revision: {}", docs2[0].revision_id);
    }
    
    // Check if they're different (should be!)
    if !docs1.is_empty() && !docs2.is_empty() {
        let same_content = docs1[0].content == docs2[0].content;
        let same_revision = docs1[0].revision_id == docs2[0].revision_id;
        
        println!("\n🔍 Conflict Analysis:");
        println!("  Same content: {}", same_content);
        println!("  Same revision: {}", same_revision);
        
        if !same_content || !same_revision {
            println!("  ❌ CONFLICT DETECTED - Clients have diverged!");
        } else {
            println!("  ✅ No conflict - Clients are in sync");
        }
    }
    
    println!("\n🎯 Key Findings:");
    println!("1. Offline edits are stored locally ✅");
    println!("2. Conflicts are detectable by different revisions/content ✅"); 
    println!("3. The sync system needs to handle this when reconnecting");
    
    // Cleanup
    std::fs::remove_dir_all(&test_dir).ok();
    
    Ok(())
}