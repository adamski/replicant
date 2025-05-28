use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::{Document, DocumentPatch};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    // Authentication
    Authenticate {
        user_id: Uuid,
        auth_token: String,
    },
    
    // Document operations
    CreateDocument {
        document: Document,
    },
    UpdateDocument {
        patch: DocumentPatch,
    },
    DeleteDocument {
        document_id: Uuid,
        revision_id: String,  // CouchDB-style
    },
    
    // Sync operations
    RequestSync {
        document_ids: Vec<Uuid>,
    },
    RequestFullSync,
    
    // New sequence-based sync operations
    GetChangesSince {
        last_sequence: u64,
        limit: Option<u32>,  // Optional pagination
    },
    AckChanges {
        up_to_sequence: u64,  // Client confirms it processed up to this sequence
    },
    
    // Heartbeat
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    // Authentication responses
    AuthSuccess {
        session_id: Uuid,
    },
    AuthError {
        reason: String,
    },
    
    // Document updates
    DocumentCreated {
        document: Document,
    },
    DocumentUpdated {
        patch: DocumentPatch,
    },
    DocumentDeleted {
        document_id: Uuid,
        revision_id: String,  // CouchDB-style
    },
    
    // Sync responses
    SyncDocument {
        document: Document,
    },
    SyncComplete {
        synced_count: usize,
    },
    
    // Conflict notification
    ConflictDetected {
        document_id: Uuid,
        local_revision: String,  // CouchDB-style
        server_revision: String,  // CouchDB-style
        resolution_strategy: ConflictResolution,
    },
    
    // New sequence-based sync responses
    Changes {
        events: Vec<ChangeEvent>,
        latest_sequence: u64,
        has_more: bool,  // True if there are more changes beyond the limit
    },
    ChangesAcknowledged {
        sequence: u64,
    },
    
    // Errors
    Error {
        code: ErrorCode,
        message: String,
    },
    
    // Heartbeat
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolution {
    ServerWins,
    ClientWins,
    Manual {
        server_document: Document,
        client_patch: DocumentPatch,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidAuth,
    DocumentNotFound,
    InvalidPatch,
    VersionMismatch,
    ServerError,
    RateLimitExceeded,
}

// New types for sequence-based sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub sequence: u64,
    pub document_id: Uuid,
    pub user_id: Uuid,
    pub event_type: ChangeEventType,
    pub revision_id: String,  // CouchDB-style
    pub json_patch: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeEventType {
    Create,
    Update,
    Delete,
}