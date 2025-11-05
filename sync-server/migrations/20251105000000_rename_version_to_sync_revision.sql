-- Rename version column to sync_revision in documents table
-- This better reflects that it's an internal sync mechanism, not a user-facing version
ALTER TABLE documents RENAME COLUMN version TO sync_revision;

-- Rename version column to sync_revision in document_revisions table
ALTER TABLE document_revisions RENAME COLUMN version TO sync_revision;
