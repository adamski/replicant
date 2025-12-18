-- Add change_events table for reliable sync
-- Migration: 002_add_change_events  
-- Using JSON TEXT for now, can upgrade to JSONB later

-- Create change_events table for tracking all document changes
CREATE TABLE change_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id TEXT NOT NULL,
    json_patch TEXT,  -- JSON text format (can upgrade to JSONB later)
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Indexes for efficient sync queries
CREATE INDEX idx_user_sequence ON change_events(user_id, sequence);
CREATE INDEX idx_document_sequence ON change_events(document_id, sequence);

-- Add a table to track last synced sequence per user (for client state)
CREATE TABLE sync_state (
    user_id TEXT PRIMARY KEY,
    last_synced_sequence INTEGER NOT NULL DEFAULT 0,
    last_sync_time TEXT NOT NULL DEFAULT (datetime('now'))
);