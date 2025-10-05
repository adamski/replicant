use uuid::Uuid;
use sqlx::{SqlitePool, Row};
use sync_core::protocol::ClientMessage;
use crate::ClientResult;
use crate::errors::ClientError;
use crate::queries::Queries;

pub struct OfflineQueue {
    pool: SqlitePool,
}

impl OfflineQueue {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    
    pub async fn enqueue(&self, message: ClientMessage) -> ClientResult<()> {
        let message_json = serde_json::to_string(&message)?;
        
        sqlx::query(Queries::INSERT_SYNC_QUEUE)
            .bind(extract_document_id(&message).map(|id| id.to_string()))
            .bind(operation_type(&message))
            .bind(message_json)
            .execute(&self.pool)
            .await?;
        
        Ok(())
    }
    
    pub async fn process_queue<F, Fut>(&self, mut send_fn: F) -> ClientResult<()>
    where
        F: FnMut(ClientMessage) -> Fut,
        Fut: std::future::Future<Output = ClientResult<()>>,
    {
        let rows = sqlx::query(Queries::GET_SYNC_QUEUE)
            .fetch_all(&self.pool)
            .await?;
        
        for row in rows {
            let id: i64 = row.get("id");
            let patch: Option<String> = row.get("patch");
            let _retry_count: i64 = row.get("retry_count");
            
            let message: ClientMessage = serde_json::from_str(&patch.unwrap_or_default())?;
            
            // Simple retry logic without backoff crate due to closure limitations
            let result = send_fn(message.clone()).await;
            
            match result {
                Ok(_) => {
                    // Remove from queue
                    sqlx::query(Queries::DELETE_FROM_QUEUE)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                }
                Err(_) => {
                    // Increment retry count
                    sqlx::query(Queries::INCREMENT_RETRY_COUNT)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                }
            }
        }
        
        Ok(())
    }
}

pub fn extract_document_id(message: &ClientMessage) -> Option<Uuid> {
    match message {
        ClientMessage::CreateDocument { document } => Some(document.id),
        ClientMessage::UpdateDocument { patch } => Some(patch.document_id),
        ClientMessage::DeleteDocument { document_id, .. } => Some(*document_id),
        _ => None,
    }
}

pub fn operation_type(message: &ClientMessage) -> &'static str {
    match message {
        ClientMessage::CreateDocument { .. } => "create",
        ClientMessage::UpdateDocument { .. } => "update",
        ClientMessage::DeleteDocument { .. } => "delete",
        _ => "other",
    }
}