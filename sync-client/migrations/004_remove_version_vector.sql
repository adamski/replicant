-- Remove version_vector column from documents table
-- Part of simplification for server-authoritative sync architecture
-- Version vectors are unnecessary for centralized systems

ALTER TABLE documents DROP COLUMN version_vector;
