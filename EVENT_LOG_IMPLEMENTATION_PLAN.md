# Event Log Implementation Plan

*Date: December 28, 2024*

## Goal
Add an event log to enable reliable sync while keeping existing document storage and query patterns unchanged.

## Requirements
- **SQLite 3.45.0+** for JSONB support (client databases)
- **PostgreSQL 12+** for JSONB support (server database)
- **Consistent JSONB usage** across both database systems

## Overview

### What Changes
- ✅ Add `change_events` table for sync ordering
- ✅ Update document operations to write events
- ✅ Add sequence-based sync protocol
- ✅ Replace unreliable WebSocket broadcasting with pull-based sync

### What Stays the Same  
- ✅ Documents table structure (unchanged)
- ✅ All existing queries work as-is
- ✅ Client document operations (create/update/delete)
- ✅ PostgreSQL and SQLite support

## Phase 1: Database Schema (1-2 days)

### 1.1 Add Change Events Table

**Note**: This implementation uses JSONB in both PostgreSQL and SQLite (3.45.0+) for consistency and performance.

#### Benefits of JSONB Consistency:
- **Same queries work on both databases** - No database-specific JSON handling
- **Better performance** - Binary format is faster than text JSON
- **Index support** - Both databases can index JSONB fields efficiently  
- **Type safety** - Automatic JSON validation on insert
- **Future-proof** - Consistent with modern JSON database trends

```sql
-- PostgreSQL version
CREATE TABLE change_events (
    sequence BIGSERIAL PRIMARY KEY,
    document_id UUID NOT NULL,
    user_id UUID NOT NULL,
    event_type VARCHAR(10) NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id UUID NOT NULL,
    json_patch JSONB,  -- NULL for create/delete, patch for updates
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- Indexes for efficient sync queries
    INDEX idx_user_sequence (user_id, sequence),
    INDEX idx_document_sequence (document_id, sequence),
    INDEX idx_sequence_created (sequence, created_at)
);

-- SQLite version (for client databases) - Requires SQLite 3.45.0+
CREATE TABLE change_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id TEXT NOT NULL,
    json_patch BLOB,  -- JSONB binary format (SQLite 3.45.0+)
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    
    CREATE INDEX idx_user_sequence ON change_events(user_id, sequence);
    CREATE INDEX idx_document_sequence ON change_events(document_id, sequence);
);
```

### 1.2 Add Migration Files

```rust
// sync-server/migrations/YYYYMMDD_add_change_events.sql
-- Up migration
CREATE TABLE change_events (
    sequence BIGSERIAL PRIMARY KEY,
    document_id UUID NOT NULL,
    user_id UUID NOT NULL,
    event_type VARCHAR(10) NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id UUID NOT NULL,
    json_patch JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_user_sequence ON change_events(user_id, sequence);
CREATE INDEX idx_document_sequence ON change_events(document_id, sequence);
CREATE INDEX idx_sequence_created ON change_events(sequence, created_at);

-- Down migration
DROP TABLE change_events;
```

```sql
-- sync-client/migrations/002_add_change_events.sql
-- Requires SQLite 3.45.0+ for JSONB support
CREATE TABLE change_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id TEXT NOT NULL,
    json_patch BLOB,  -- JSONB binary format
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_user_sequence ON change_events(user_id, sequence);
CREATE INDEX idx_document_sequence ON change_events(document_id, sequence);
```

## Phase 2: Protocol Updates (1 day)

### 2.1 Add New Message Types

```rust
// sync-core/src/protocol.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    // ... existing messages unchanged
    
    // New sync messages
    GetChangesSince {
        last_sequence: u64,
        limit: Option<u32>,  // Optional pagination
    },
    
    AckChanges {
        up_to_sequence: u64,  // Client confirms it processed up to this sequence
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    // ... existing messages unchanged
    
    // New sync responses
    Changes {
        events: Vec<ChangeEvent>,
        latest_sequence: u64,
        has_more: bool,  // True if there are more changes beyond the limit
    },
    
    ChangesAcknowledged {
        sequence: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub sequence: u64,
    pub document_id: Uuid,
    pub user_id: Uuid,
    pub event_type: ChangeEventType,
    pub revision_id: Uuid,
    pub json_patch: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeEventType {
    Create,
    Update,
    Delete,
}
```

### 2.2 Update Sync Logic

Replace push-based broadcasting with pull-based sync:

**Before (unreliable):**
```
Client creates doc → Server broadcasts to all clients
```

**After (reliable):**
```
Client creates doc → Server logs event
Client polls: "Any changes since sequence X?" → Server returns ordered events
```

## Phase 3: Server Implementation (2-3 days)

### 3.1 Add Event Logging to Database Layer

```rust
// sync-server/src/database.rs

impl ServerDatabase {
    // New method to log events
    pub async fn log_change_event(
        &self,
        document_id: &Uuid,
        user_id: &Uuid,
        event_type: &str,
        revision_id: &Uuid,
        json_patch: Option<&serde_json::Value>,
    ) -> Result<u64, sqlx::Error> {
        let sequence: (i64,) = sqlx::query_as(
            "INSERT INTO change_events (document_id, user_id, event_type, revision_id, json_patch) 
             VALUES ($1, $2, $3, $4, $5) 
             RETURNING sequence"
        )
        .bind(document_id)
        .bind(user_id)
        .bind(event_type)
        .bind(revision_id)
        .bind(json_patch)
        .fetch_one(&self.pool)
        .await?;
        
        Ok(sequence.0 as u64)
    }
    
    // New method to get changes for sync
    pub async fn get_changes_since(
        &self,
        user_id: &Uuid,
        since_sequence: u64,
        limit: Option<u32>,
    ) -> Result<Vec<ChangeEvent>, sqlx::Error> {
        let limit = limit.unwrap_or(1000);
        
        let rows = sqlx::query!(
            "SELECT sequence, document_id, user_id, event_type, revision_id, json_patch, created_at
             FROM change_events 
             WHERE user_id = $1 AND sequence > $2
             ORDER BY sequence ASC
             LIMIT $3",
            user_id,
            since_sequence as i64,
            limit as i64
        )
        .fetch_all(&self.pool)
        .await?;
        
        let mut events = Vec::new();
        for row in rows {
            events.push(ChangeEvent {
                sequence: row.sequence as u64,
                document_id: row.document_id,
                user_id: row.user_id,
                event_type: match row.event_type.as_str() {
                    "create" => ChangeEventType::Create,
                    "update" => ChangeEventType::Update,
                    "delete" => ChangeEventType::Delete,
                    _ => continue,
                },
                revision_id: row.revision_id,
                json_patch: row.json_patch,
                created_at: row.created_at,
            });
        }
        
        Ok(events)
    }
    
    pub async fn get_latest_sequence(&self, user_id: &Uuid) -> Result<u64, sqlx::Error> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT MAX(sequence) FROM change_events WHERE user_id = $1"
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(result.map(|r| r.0 as u64).unwrap_or(0))
    }
}
```

### 3.2 Update Document Operations

```rust
// sync-server/src/database.rs

impl ServerDatabase {
    // Update existing create_document to log events
    pub async fn create_document(&self, document: &Document) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        
        // 1. Insert document (existing logic)
        sqlx::query!(
            "INSERT INTO documents (id, user_id, title, content, revision_id, version, vector_clock, created_at, updated_at) 
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            document.id,
            document.user_id,
            document.title,
            document.content,
            document.revision_id,
            document.version,
            serde_json::to_value(&document.vector_clock).unwrap(),
            document.created_at,
            document.updated_at
        )
        .execute(&mut tx)
        .await?;
        
        // 2. Log change event (new)
        sqlx::query!(
            "INSERT INTO change_events (document_id, user_id, event_type, revision_id) 
             VALUES ($1, $2, 'create', $3)",
            document.id,
            document.user_id,
            document.revision_id
        )
        .execute(&mut tx)
        .await?;
        
        tx.commit().await?;
        Ok(())
    }
    
    // Update existing update_document to log events
    pub async fn update_document(&self, document: &Document, patch: &serde_json::Value) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        
        // 1. Update document (existing logic)
        sqlx::query!(
            "UPDATE documents SET content = $1, revision_id = $2, version = $3, vector_clock = $4, updated_at = $5 
             WHERE id = $6",
            document.content,
            document.revision_id,
            document.version,
            serde_json::to_value(&document.vector_clock).unwrap(),
            document.updated_at,
            document.id
        )
        .execute(&mut tx)
        .await?;
        
        // 2. Log change event with patch (new)
        sqlx::query!(
            "INSERT INTO change_events (document_id, user_id, event_type, revision_id, json_patch) 
             VALUES ($1, $2, 'update', $3, $4)",
            document.id,
            document.user_id,
            document.revision_id,
            patch
        )
        .execute(&mut tx)
        .await?;
        
        tx.commit().await?;
        Ok(())
    }
    
    // Similar updates for delete_document...
}
```

### 3.3 Update Sync Handler

```rust
// sync-server/src/sync_handler.rs

impl SyncHandler {
    pub async fn handle_message(&mut self, message: ClientMessage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match message {
            // ... existing message handling unchanged
            
            ClientMessage::GetChangesSince { last_sequence, limit } => {
                let user_id = self.user_id.ok_or("Not authenticated")?;
                
                let events = self.db.get_changes_since(&user_id, last_sequence, limit).await?;
                let latest_sequence = self.db.get_latest_sequence(&user_id).await?;
                let has_more = limit.is_some() && events.len() == limit.unwrap() as usize;
                
                self.tx.send(ServerMessage::Changes {
                    events,
                    latest_sequence,
                    has_more,
                }).await?;
            }
            
            ClientMessage::AckChanges { up_to_sequence } => {
                // Client confirmed it processed changes up to this sequence
                // Could log this for monitoring or cleanup purposes
                self.tx.send(ServerMessage::ChangesAcknowledged {
                    sequence: up_to_sequence,
                }).await?;
            }
        }
        
        Ok(())
    }
}
```

## Phase 4: Client Implementation (2-3 days)

### 4.1 Add Client Event Logging

```rust
// sync-client/src/database.rs

impl ClientDatabase {
    // Log change event with JSONB support
    pub async fn log_change_event(
        &self,
        document_id: &Uuid,
        user_id: &Uuid,
        event_type: &str,
        revision_id: &Uuid,
        json_patch: Option<&serde_json::Value>,
    ) -> Result<u64, ClientError> {
        // Convert JSON patch to JSONB binary format for SQLite
        let json_patch_blob = if let Some(patch) = json_patch {
            Some(sqlx::types::Json(patch))
        } else {
            None
        };
        
        let sequence: (i64,) = sqlx::query_as(
            "INSERT INTO change_events (document_id, user_id, event_type, revision_id, json_patch) 
             VALUES (?1, ?2, ?3, ?4, jsonb(?5))  -- SQLite's jsonb() function for binary encoding
             RETURNING sequence"
        )
        .bind(document_id.to_string())
        .bind(user_id.to_string())
        .bind(event_type)
        .bind(revision_id.to_string())
        .bind(json_patch_blob)
        .fetch_one(&self.pool)
        .await?;
        
        Ok(sequence.0 as u64)
    }
    
    pub async fn get_changes_since(
        &self,
        user_id: &Uuid,
        since_sequence: u64,
        limit: Option<u32>,
    ) -> Result<Vec<ChangeEvent>, ClientError> {
        let limit = limit.unwrap_or(1000);
        
        let rows = sqlx::query!(
            "SELECT sequence, document_id, user_id, event_type, revision_id, 
                    json(json_patch) as json_patch_text, created_at  -- Convert JSONB back to text for sqlx
             FROM change_events 
             WHERE user_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
            user_id.to_string(),
            since_sequence as i64,
            limit as i64
        )
        .fetch_all(&self.pool)
        .await?;
        
        let mut events = Vec::new();
        for row in rows {
            let json_patch = if let Some(patch_text) = row.json_patch_text {
                Some(serde_json::from_str(&patch_text)?)
            } else {
                None
            };
            
            events.push(ChangeEvent {
                sequence: row.sequence as u64,
                document_id: Uuid::parse_str(&row.document_id)?,
                user_id: Uuid::parse_str(&row.user_id)?,
                event_type: match row.event_type.as_str() {
                    "create" => ChangeEventType::Create,
                    "update" => ChangeEventType::Update,
                    "delete" => ChangeEventType::Delete,
                    _ => continue,
                },
                revision_id: Uuid::parse_str(&row.revision_id)?,
                json_patch,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&chrono::Utc),
            });
        }
        
        Ok(events)
    }
    
    pub async fn get_last_synced_sequence(&self, user_id: &Uuid) -> Result<u64, ClientError> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT MAX(sequence) FROM change_events WHERE user_id = ?1"
        )
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(result.map(|r| r.0 as u64).unwrap_or(0))
    }
}
```

### 4.2 Update Sync Engine

```rust
// sync-client/src/sync_engine.rs

impl SyncEngine {
    // Replace sync_all() with sequence-based sync
    pub async fn sync_incremental(&self) -> Result<(), ClientError> {
        let last_sequence = self.db.get_last_synced_sequence(&self.user_id).await?;
        
        // Request changes since last sync
        self.ws_client.send(ClientMessage::GetChangesSince {
            last_sequence,
            limit: Some(100), // Process in batches
        }).await?;
        
        Ok(())
    }
    
    // Handle change events from server
    async fn handle_changes(&self, events: Vec<ChangeEvent>, latest_sequence: u64) -> Result<(), ClientError> {
        for event in events {
            match event.event_type {
                ChangeEventType::Create => {
                    // Fetch full document from server
                    // (or include it in the event)
                    // Save to local database
                }
                ChangeEventType::Update => {
                    // Apply the JSON patch
                    let mut doc = self.db.get_document(&event.document_id).await?;
                    if let Some(patch) = event.json_patch {
                        apply_patch(&mut doc.content, &patch)?;
                        doc.revision_id = event.revision_id;
                        self.db.save_document(&doc).await?;
                    }
                }
                ChangeEventType::Delete => {
                    // Soft delete the document
                    self.db.delete_document(&event.document_id).await?;
                }
            }
            
            // Track that we processed this event
            self.db.mark_sequence_processed(event.sequence).await?;
        }
        
        // Acknowledge we processed all these changes
        self.ws_client.send(ClientMessage::AckChanges {
            up_to_sequence: latest_sequence,
        }).await?;
        
        Ok(())
    }
}
```

## Phase 5: Testing & Validation (2-3 days)

### 5.1 Update Integration Tests

```rust
// Update concurrent sessions test to use sequence-based sync
crate::integration_test!(test_concurrent_sessions_reliable, |ctx: TestContext| async move {
    let user_id = Uuid::new_v4();
    let token = "demo-token";
    
    // Create clients
    let clients = /* ... */;
    
    // Each client creates a document
    for (i, client) in clients.iter().enumerate() {
        client.create_document(format!("Doc {}", i), json!({"test": true})).await?;
    }
    
    // Force sync on all clients
    for client in &clients {
        client.sync_incremental().await?;
    }
    
    // Wait for convergence with detailed logging
    let converged = assert_eventual_convergence(&clients, 5, Duration::from_secs(10)).await;
    assert!(converged, "All clients should eventually see all 5 documents");
});
```

### 5.2 Add Chaos Testing

```rust
// Test with packet loss, delays, etc.
crate::integration_test!(test_sync_with_network_issues, |ctx: TestContext| async move {
    // Simulate network problems
    // Verify system recovers through incremental sync
});
```

## Phase 6: Performance & Cleanup (1-2 days)

### 6.1 Add Event Cleanup

```rust
// Clean up old events periodically
pub async fn cleanup_old_events(&self, older_than_days: u32) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query!(
        "DELETE FROM change_events WHERE created_at < NOW() - INTERVAL '%d days'",
        older_than_days
    )
    .execute(&self.pool)
    .await?;
    
    Ok(deleted.rows_affected())
}
```

### 6.2 Add Monitoring

```rust
// Track sync performance
pub async fn get_sync_metrics(&self, user_id: &Uuid) -> SyncMetrics {
    // Events per hour, lag time, etc.
}
```

## Success Criteria

1. **Concurrent sessions test passes reliably** (100% success rate)
2. **No message loss** under normal conditions
3. **Graceful recovery** from network issues
4. **Existing queries work unchanged** 
5. **Performance acceptable** (< 100ms for incremental sync)

## Timeline Summary

- **Week 1**: Phases 1-3 (Schema, Protocol, Server)
- **Week 2**: Phases 4-5 (Client, Testing)  
- **Week 3**: Phase 6 (Performance, Cleanup)

This approach gives you reliable sync while keeping all your existing PostgreSQL/SQLite knowledge and infrastructure intact!