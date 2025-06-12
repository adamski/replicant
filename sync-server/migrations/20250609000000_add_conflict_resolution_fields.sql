-- Add server_timestamp and applied fields for conflict resolution tracking
ALTER TABLE change_events 
ADD COLUMN server_timestamp TIMESTAMPTZ DEFAULT NOW() NOT NULL,
ADD COLUMN applied BOOLEAN DEFAULT true NOT NULL;

-- Add index on server_timestamp for efficient ordering
CREATE INDEX idx_change_events_server_timestamp ON change_events(server_timestamp);

-- Add index for finding unapplied changes (for conflict analysis)
CREATE INDEX idx_change_events_applied ON change_events(document_id, applied) WHERE applied = false;