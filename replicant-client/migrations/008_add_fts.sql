-- Full-text search support using FTS5

-- Store which JSON paths to index for search
CREATE TABLE IF NOT EXISTS search_config (
    json_path TEXT NOT NULL PRIMARY KEY
);

-- FTS5 virtual table for full-text search
CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
    document_id UNINDEXED,
    title,
    body,
    tokenize='porter unicode61'
);
