use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strum::{Display, EnumString};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub user_id: Uuid,
    pub content: serde_json::Value,
    pub version: i64,
    pub content_hash: Option<String>, // SHA256 hash for integrity verification
    pub version_vector: VersionVector,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Document {
    /// Get the title from the content JSON, if present
    pub fn title(&self) -> Option<&str> {
        self.content.get("title").and_then(|v| v.as_str())
    }

    /// Get the title from content JSON, or return a default
    pub fn title_or_default(&self) -> &str {
        self.title().unwrap_or("Untitled")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VersionVector(pub HashMap<String, u64>);

impl VersionVector {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn increment(&mut self, node_id: &str) {
        let counter = self.0.entry(node_id.to_string()).or_insert(0);
        *counter += 1;
    }

    pub fn merge(&mut self, other: &VersionVector) {
        for (node, &timestamp) in &other.0 {
            let current = self.0.entry(node.clone()).or_insert(0);
            *current = (*current).max(timestamp);
        }
    }

    pub fn is_concurrent(&self, other: &VersionVector) -> bool {
        let mut self_ahead = false;
        let mut other_ahead = false;

        for (node, &self_time) in &self.0 {
            let other_time = other.0.get(node).copied().unwrap_or(0);
            if self_time > other_time {
                self_ahead = true;
            } else if self_time < other_time {
                other_ahead = true;
            }
        }

        for (node, &other_time) in &other.0 {
            if !self.0.contains_key(node) && other_time > 0 {
                other_ahead = true;
            }
        }

        self_ahead && other_ahead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_vector_increment() {
        let mut vc = VersionVector::new();
        vc.increment("node1");
        assert_eq!(vc.0.get("node1"), Some(&1));

        vc.increment("node1");
        assert_eq!(vc.0.get("node1"), Some(&2));
    }

    #[test]
    fn test_version_vector_merge() {
        let mut vc1 = VersionVector::new();
        let mut vc2 = VersionVector::new();

        vc1.increment("node1");
        vc2.increment("node2");

        vc1.merge(&vc2);
        assert_eq!(vc1.0.get("node1"), Some(&1));
        assert_eq!(vc1.0.get("node2"), Some(&1));
    }

    #[test]
    fn test_version_vector_concurrent() {
        let mut vc1 = VersionVector::new();
        let mut vc2 = VersionVector::new();

        vc1.increment("node1");
        vc2.increment("node2");

        assert!(vc1.is_concurrent(&vc2));
    }


    #[test]
    fn test_document_title_helpers() {
        // Test document with title
        let doc_with_title = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"title": "My Document", "test": true}),
            version: 1,
            content_hash: None,
            version_vector: VersionVector::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        assert_eq!(doc_with_title.title(), Some("My Document"));
        assert_eq!(doc_with_title.title_or_default(), "My Document");

        // Test document without title
        let doc_without_title = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"test": true}),
            version: 1,
            content_hash: None,
            version_vector: VersionVector::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        assert_eq!(doc_without_title.title(), None);
        assert_eq!(doc_without_title.title_or_default(), "Untitled");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPatch {
    pub document_id: Uuid,
    pub patch: json_patch::Patch,
    pub version_vector: VersionVector,
    pub content_hash: String, // SHA256 hash for integrity verification
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Pending,
    Conflict,
}
