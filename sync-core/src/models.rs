use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub content: serde_json::Value,
    pub revision_id: Uuid,
    pub version: i64,
    pub vector_clock: VectorClock,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
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
        
        vc1.merge(&vc2);
        assert!(!vc1.is_concurrent(&vc2));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPatch {
    pub document_id: Uuid,
    pub revision_id: Uuid,
    pub patch: json_patch::Patch,
    pub vector_clock: VectorClock,
    pub checksum: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Pending,
    Conflict,
}