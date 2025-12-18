-- REP-31: Remove revision_id, use simpler fields
-- Migration: 20251101000000_remove_revision_id
--
-- This migration removes the redundant revision_id field from all tables.
-- The system will use version (generation counter) and content_hash directly.
-- Also renames checksum to content_hash for clarity.

-- Remove revision_id from documents table
ALTER TABLE documents DROP COLUMN revision_id;

-- Rename checksum to content_hash for clarity
ALTER TABLE documents RENAME COLUMN checksum TO content_hash;

-- Remove revision_id from document_revisions table
ALTER TABLE document_revisions DROP COLUMN revision_id;

-- Remove revision_id from change_events table
ALTER TABLE change_events DROP COLUMN revision_id;
