-- User configuration (single row)
CREATE TABLE user_config (
    user_id TEXT PRIMARY KEY,
    server_url TEXT NOT NULL,
    last_sync_at TIMESTAMP,
    auth_token TEXT
);

-- Documents table
CREATE TABLE documents (
    id TEXT PRIMARY KEY,                          -- UUID
    user_id TEXT NOT NULL,                        -- Owner UUID
    title TEXT NOT NULL,
    content JSON NOT NULL,                        -- Document data
    revision_id TEXT NOT NULL,                    -- UUID for each revision
    version INTEGER NOT NULL DEFAULT 1,           -- Incremental version
    vector_clock JSON,                            -- For conflict resolution
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMP,                         -- Soft delete
    
    -- Sync metadata
    local_changes JSON,                           -- Pending changes
    sync_status TEXT DEFAULT 'synced',            -- 'synced', 'pending', 'conflict'
    last_synced_revision TEXT,                    -- Last known server revision
    
    CHECK (sync_status IN ('synced', 'pending', 'conflict'))
);

-- Sync queue for offline changes
CREATE TABLE sync_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL,
    operation_type TEXT NOT NULL,                 -- 'create', 'update', 'delete'
    patch JSON,                                   -- JSON patch for updates
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    retry_count INTEGER DEFAULT 0,
    
    FOREIGN KEY (document_id) REFERENCES documents(id),
    CHECK (operation_type IN ('create', 'update', 'delete'))
);

-- Indexes for performance
CREATE INDEX idx_documents_user_id ON documents(user_id);
CREATE INDEX idx_documents_sync_status ON documents(sync_status);
CREATE INDEX idx_sync_queue_created_at ON sync_queue(created_at);