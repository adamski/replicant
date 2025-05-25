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
        revision_id: Uuid,
    },
    
    // Sync operations
    RequestSync {
        document_ids: Vec<Uuid>,
    },
    RequestFullSync,
    
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
        revision_id: Uuid,
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
        local_revision: Uuid,
        server_revision: Uuid,
        resolution_strategy: ConflictResolution,
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