-- Add base_content column to sync_queue table
-- Stores the full document content the queued patch is built against (the last
-- state the server acknowledged). The patch is regenerated from this base and
-- the document's current content at read time, so a run of offline edits flushes
-- as one cumulative diff instead of the newest fragment.
ALTER TABLE sync_queue ADD COLUMN base_content TEXT;
