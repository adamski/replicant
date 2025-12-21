-- Add change_events table for reliable sync
-- Migration: 20241228000000_add_change_events

-- Create change_events table for tracking all document changes
CREATE TABLE change_events (
    sequence BIGSERIAL PRIMARY KEY,
    document_id UUID NOT NULL,
    user_id UUID NOT NULL,
    event_type VARCHAR(10) NOT NULL CHECK (event_type IN ('create', 'update', 'delete')),
    revision_id TEXT NOT NULL,
    json_patch JSONB,  -- NULL for create/delete, patch for updates
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Indexes for efficient sync queries
CREATE INDEX idx_user_sequence ON change_events(user_id, sequence);
CREATE INDEX idx_document_sequence ON change_events(document_id, sequence);
CREATE INDEX idx_sequence_created ON change_events(sequence, created_at);

-- Add foreign key constraint to documents table
ALTER TABLE change_events 
ADD CONSTRAINT fk_change_events_document 
FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE;

-- Add foreign key constraint to users table  
ALTER TABLE change_events
ADD CONSTRAINT fk_change_events_user
FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;