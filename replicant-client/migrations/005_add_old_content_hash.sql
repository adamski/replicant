-- Add old_content_hash column to sync_queue table
-- This stores the hash of the document content BEFORE an update was made
-- Used for optimistic locking when syncing offline edits
ALTER TABLE sync_queue ADD COLUMN old_content_hash TEXT;
