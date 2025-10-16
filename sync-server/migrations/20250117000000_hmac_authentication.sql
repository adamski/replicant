-- Consolidated HMAC authentication migration
-- This replaces the previous 4 separate migrations for cleaner upgrade path

-- Remove password_hash column from users (no longer using password authentication)
ALTER TABLE users DROP COLUMN IF EXISTS password_hash;

-- Drop old tables if they exist
DROP TABLE IF EXISTS api_keys;
DROP TABLE IF EXISTS api_credentials;

-- Create clean api_credentials table (plaintext storage for MVP)
-- Note: Credentials are application-wide, not tied to individual users
-- Multiple end-users can authenticate through the same application credentials
CREATE TABLE api_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    api_key TEXT NOT NULL UNIQUE,
    secret TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT 'Default',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT true
);

-- Indexes for efficient lookups
CREATE INDEX idx_api_credentials_api_key ON api_credentials(api_key);
CREATE INDEX idx_api_credentials_active ON api_credentials(is_active);
