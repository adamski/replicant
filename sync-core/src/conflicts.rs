use crate::errors::SyncError;
use crate::models::{Document, VersionVector};
use crate::SyncResult;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum ConflictStrategy {
    LastWriteWins,
    FirstWriteWins,
    MergeJson,
    Manual,
}

pub struct ConflictResolver {
    strategy: ConflictStrategy,
}

impl ConflictResolver {
    pub fn new(strategy: ConflictStrategy) -> Self {
        Self { strategy }
    }

    pub fn resolve(&self, local: &Document, remote: &Document) -> SyncResult<Document> {
        match &self.strategy {
            ConflictStrategy::LastWriteWins => {
                if local.updated_at > remote.updated_at {
                    Ok(local.clone())
                } else {
                    Ok(remote.clone())
                }
            }
            ConflictStrategy::FirstWriteWins => {
                if local.created_at < remote.created_at {
                    Ok(local.clone())
                } else {
                    Ok(remote.clone())
                }
            }
            ConflictStrategy::MergeJson => self.merge_json_documents(local, remote),
            ConflictStrategy::Manual => Err(SyncError::ConflictDetected(local.id)),
        }
    }

    fn merge_json_documents(&self, local: &Document, remote: &Document) -> SyncResult<Document> {
        let merged_content = merge_json_values(&local.content, &remote.content)?;

        let mut merged_doc = local.clone();
        merged_doc.content = merged_content;
        merged_doc.version = local.version.max(remote.version) + 1;
        merged_doc.content_hash = None; // Will be recalculated when saved

        // Merge version vectors
        merged_doc.version_vector.merge(&remote.version_vector);

        Ok(merged_doc)
    }
}

fn merge_json_values(local: &Value, remote: &Value) -> SyncResult<Value> {
    match (local, remote) {
        (Value::Object(local_map), Value::Object(remote_map)) => {
            let mut merged = local_map.clone();

            for (key, remote_value) in remote_map {
                match local_map.get(key) {
                    Some(local_value) => {
                        merged.insert(key.clone(), merge_json_values(local_value, remote_value)?);
                    }
                    None => {
                        merged.insert(key.clone(), remote_value.clone());
                    }
                }
            }

            Ok(Value::Object(merged))
        }
        (Value::Array(local_arr), Value::Array(remote_arr)) => {
            // Simple merge: concatenate arrays and remove duplicates
            let mut merged = local_arr.clone();
            for item in remote_arr {
                if !merged.contains(item) {
                    merged.push(item.clone());
                }
            }
            Ok(Value::Array(merged))
        }
        (_, _) => {
            // For non-mergeable types, prefer remote
            Ok(remote.clone())
        }
    }
}

pub fn detect_conflict(local: &VersionVector, remote: &VersionVector) -> bool {
    local.is_concurrent(remote)
}
