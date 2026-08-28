use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub content: serde_json::Value,
    pub sync_revision: i64,
    pub content_hash: Option<String>, // SHA256 hash for integrity verification
    pub title: Option<String>,        // Derived from content['title'] for query performance
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub provenance: Option<serde_json::Value>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_title_helpers() {
        // Test document with title
        let doc_with_title = Document {
            id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
            content: serde_json::json!({"title": "My Document", "test": true}),
            sync_revision: 1,
            content_hash: None,
            title: Some("My Document".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
            author_name: None,
            visibility: None,
            provenance: None,
        };

        assert_eq!(doc_with_title.title(), Some("My Document"));
        assert_eq!(doc_with_title.title_or_default(), "My Document");

        // Test document without title
        let doc_without_title = Document {
            id: Uuid::new_v4(),
            user_id: Some(Uuid::new_v4()),
            content: serde_json::json!({"test": true}),
            sync_revision: 1,
            content_hash: None,
            title: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
            author_name: None,
            visibility: None,
            provenance: None,
        };

        assert_eq!(doc_without_title.title(), None);
        assert_eq!(doc_without_title.title_or_default(), "Untitled");
    }

    #[test]
    fn document_round_trips_attribution_via_serde() {
        let json = r#"{
            "id": "71b2b712-7878-56ee-8323-43809b8198a5",
            "user_id": null,
            "content": {"title": "T"},
            "sync_revision": 1,
            "content_hash": null,
            "title": null,
            "created_at": "2026-07-03T00:00:00Z",
            "updated_at": "2026-07-03T00:00:00Z",
            "deleted_at": null,
            "author_name": "Robert Rich",
            "visibility": "public",
            "provenance": {"copied_from": "x"}
        }"#;
        let doc: Document = serde_json::from_str(json).unwrap();
        assert_eq!(doc.author_name.as_deref(), Some("Robert Rich"));
        assert_eq!(doc.visibility.as_deref(), Some("public"));
        assert!(doc.provenance.is_some());

        // Pre-1C payloads (no attribution keys) still deserialize.
        let legacy = r#"{
            "id": "71b2b712-7878-56ee-8323-43809b8198a5",
            "user_id": null, "content": {}, "sync_revision": 1,
            "content_hash": null, "title": null,
            "created_at": "2026-07-03T00:00:00Z", "updated_at": "2026-07-03T00:00:00Z",
            "deleted_at": null
        }"#;
        let doc: Document = serde_json::from_str(legacy).unwrap();
        assert_eq!(doc.author_name, None);
    }

    #[test]
    fn server_document_patch_round_trips_with_document_id_key() {
        let json = r#"{
            "document_id": "71b2b712-7878-56ee-8323-43809b8198a5",
            "patch": [{"op": "replace", "path": "/title", "value": "T"}],
            "sync_revision": 7,
            "content_hash": "abc123"
        }"#;
        let patch: ServerDocumentPatch = serde_json::from_str(json).unwrap();
        assert_eq!(
            patch.document_id.to_string(),
            "71b2b712-7878-56ee-8323-43809b8198a5"
        );
        assert_eq!(patch.sync_revision, 7);
        assert_eq!(patch.content_hash, "abc123");
    }

    #[test]
    fn server_document_patch_round_trips_with_id_key() {
        // Wire shape actually sent by the server (channel.ex / documents.ex
        // both use "id" for the document_updated broadcast).
        let json = r#"{
            "id": "71b2b712-7878-56ee-8323-43809b8198a5",
            "patch": [{"op": "replace", "path": "/title", "value": "T"}],
            "sync_revision": 7,
            "content_hash": "abc123"
        }"#;
        let patch: ServerDocumentPatch = serde_json::from_str(json).unwrap();
        assert_eq!(
            patch.document_id.to_string(),
            "71b2b712-7878-56ee-8323-43809b8198a5"
        );
        assert_eq!(patch.sync_revision, 7);
        assert_eq!(patch.content_hash, "abc123");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPatch {
    pub document_id: Uuid,
    pub patch: json_patch::Patch,
    pub content_hash: String, // SHA256 hash for integrity verification
}

/// Server broadcast of an applied patch. Unlike [`DocumentPatch`] (a
/// client→server request carrying a pre-update hash), `content_hash` here is
/// the hash of the content AFTER the patch was applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerDocumentPatch {
    #[serde(alias = "document_id", alias = "id")]
    pub document_id: Uuid,
    pub patch: json_patch::Patch,
    /// Revision AFTER this patch was applied on the server.
    pub sync_revision: i64,
    /// Hash of the content AFTER this patch (NOT a base hash — unlike
    /// client→server DocumentPatch.content_hash).
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SyncStatus {
    Synced,
    Pending,
    Conflict,
}
