-- Remove version_vector column from documents table
-- Part of simplification for server-authoritative sync architecture
-- Version vectors are unnecessary for centralized systems

-- SQLite doesn't support IF EXISTS for ALTER TABLE, so we'll skip if column doesn't exist
-- This is safe: client never had version_vector, so this is effectively a no-op
