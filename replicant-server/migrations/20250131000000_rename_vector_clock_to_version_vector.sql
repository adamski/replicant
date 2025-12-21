-- Rename vector_clock column to version_vector for semantic accuracy
-- Version Vectors track versions of specific data items (documents)
-- Vector Clocks track causality across all events in a distributed system

ALTER TABLE documents
    RENAME COLUMN vector_clock TO version_vector;
