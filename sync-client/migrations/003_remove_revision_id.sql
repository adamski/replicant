-- REP-31: Remove revision_id, use simpler fields
-- Migration: 003_remove_revision_id
--
-- This migration removes the redundant revision_id field from all tables.
-- The system will use version (generation counter) directly.

-- Remove revision_id from documents table
ALTER TABLE documents DROP COLUMN revision_id;

-- Remove last_synced_revision which also stored revision_id
ALTER TABLE documents DROP COLUMN last_synced_revision;

-- Remove revision_id from change_events table
ALTER TABLE change_events DROP COLUMN revision_id;
