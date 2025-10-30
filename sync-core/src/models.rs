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
    pub revision_id: String, // CouchDB-style: "generation-hash" e.g. "2-a8d73487645ef123abc"
    pub version: i64,
    pub vector_clock: VectorClock,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Document {
    /// Get the generation number from CouchDB-style revision ID
    /// "2-a8d73487645ef123abc" -> 2
    pub fn generation(&self) -> u32 {
        self.revision_id
            .split('-')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1)
    }

    /// Get the hash part from CouchDB-style revision ID  
    /// "2-a8d73487645ef123abc" -> "a8d73487645ef123abc"
    pub fn hash(&self) -> &str {
        self.revision_id.split('-').nth(1).unwrap_or("")
    }

    /// Generate next revision ID with incremented generation
    /// Uses SHA256 hash of content for uniqueness
    pub fn next_revision(&self, content: &serde_json::Value) -> String {
        use sha2::{Digest, Sha256};

        let content_bytes = content.to_string().into_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&content_bytes);
        let hash = format!("{:x}", hasher.finalize());

        // Take first 16 characters of hash for brevity (like CouchDB)
        let short_hash = &hash[..16];
        format!("{}-{}", self.generation() + 1, short_hash)
    }

    /// Create initial revision ID for new documents
    pub fn initial_revision(content: &serde_json::Value) -> String {
        use sha2::{Digest, Sha256};

        let content_bytes = content.to_string().into_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&content_bytes);
        let hash = format!("{:x}", hasher.finalize());

        let short_hash = &hash[..16];
        format!("1-{}", short_hash)
    }

    /// Check if this revision is newer than another
    pub fn is_newer_than(&self, other_revision: &str) -> bool {
        let other_gen = other_revision
            .split('-')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        self.generation() > other_gen
    }

    /// Check if two revisions are in conflict (same generation, different hash)
    pub fn conflicts_with(&self, other_revision: &str) -> bool {
        let other_gen = other_revision
            .split('-')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let other_hash = other_revision.split('-').nth(1).unwrap_or("");

        self.generation() == other_gen && self.hash() != other_hash
    }

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
pub struct VectorClock(pub HashMap<String, u64>);

impl VectorClock {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn increment(&mut self, node_id: &str) {
        let counter = self.0.entry(node_id.to_string()).or_insert(0);
        *counter += 1;
    }

    pub fn merge(&mut self, other: &VectorClock) {
        for (node, &timestamp) in &other.0 {
            let current = self.0.entry(node.clone()).or_insert(0);
            *current = (*current).max(timestamp);
        }
    }

    pub fn is_concurrent(&self, other: &VectorClock) -> bool {
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
    fn test_vector_clock_increment() {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        assert_eq!(vc.0.get("node1"), Some(&1));

        vc.increment("node1");
        assert_eq!(vc.0.get("node1"), Some(&2));
    }

    #[test]
    fn test_vector_clock_merge() {
        let mut vc1 = VectorClock::new();
        let mut vc2 = VectorClock::new();

        vc1.increment("node1");
        vc2.increment("node2");

        vc1.merge(&vc2);
        assert_eq!(vc1.0.get("node1"), Some(&1));
        assert_eq!(vc1.0.get("node2"), Some(&1));
    }

    #[test]
    fn test_vector_clock_concurrent() {
        let mut vc1 = VectorClock::new();
        let mut vc2 = VectorClock::new();

        vc1.increment("node1");
        vc2.increment("node2");

        assert!(vc1.is_concurrent(&vc2));
    }

    #[test]
    fn test_document_revision_generation() {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"title": "Test", "test": true}),
            revision_id: "3-a8d73487645ef123".to_string(),
            version: 3,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        assert_eq!(doc.generation(), 3);
        assert_eq!(doc.hash(), "a8d73487645ef123");
    }

    #[test]
    fn test_document_revision_comparison() {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"title": "Test", "test": true}),
            revision_id: "3-abc123".to_string(),
            version: 3,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        // Test newer comparison
        assert!(doc.is_newer_than("2-def456"));
        assert!(!doc.is_newer_than("4-def456"));
        assert!(!doc.is_newer_than("3-def456"));

        // Test conflict detection
        assert!(doc.conflicts_with("3-def456")); // Same generation, different hash
        assert!(!doc.conflicts_with("3-abc123")); // Same revision
        assert!(!doc.conflicts_with("2-def456")); // Different generation
    }

    #[test]
    fn test_initial_revision() {
        let content = serde_json::json!({"title": "Test", "value": 42});
        let revision = Document::initial_revision(&content);

        assert!(revision.starts_with("1-"));
        assert_eq!(revision.split('-').count(), 2);

        // Same content should produce same revision
        let revision2 = Document::initial_revision(&content);
        assert_eq!(revision, revision2);
    }

    #[test]
    fn test_next_revision() {
        let doc = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"title": "Test", "test": true}),
            revision_id: "2-abc123".to_string(),
            version: 2,
            vector_clock: VectorClock::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };

        let new_content = serde_json::json!({"test": false});
        let next_revision = doc.next_revision(&new_content);

        assert!(next_revision.starts_with("3-"));
        assert_eq!(next_revision.split('-').count(), 2);

        // Different content should produce different revision
        let other_content = serde_json::json!({"test": "maybe"});
        let other_revision = doc.next_revision(&other_content);
        assert_ne!(next_revision, other_revision);
    }

    #[test]
    fn test_document_title_helpers() {
        // Test document with title
        let doc_with_title = Document {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            content: serde_json::json!({"title": "My Document", "test": true}),
            revision_id: "1-abc123".to_string(),
            version: 1,
            vector_clock: VectorClock::new(),
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
            revision_id: "1-def456".to_string(),
            version: 1,
            vector_clock: VectorClock::new(),
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
    pub revision_id: String, // CouchDB-style: "generation-hash"
    pub patch: json_patch::Patch,
    pub vector_clock: VectorClock,
    pub checksum: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Pending,
    Conflict,
}
