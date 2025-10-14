-- Clean up authentication: email-only users, single api_credentials table with plaintext storage

-- Remove username column (was added in previous migration)
ALTER TABLE users DROP COLUMN IF EXISTS username;
DROP INDEX IF EXISTS idx_users_username;

-- Remove password_hash if it still exists
ALTER TABLE users DROP COLUMN IF EXISTS password_hash;

-- Drop api_keys table if exists (not needed)
DROP TABLE IF EXISTS api_keys;

-- Drop old api_credentials and recreate clean
DROP TABLE IF EXISTS api_credentials;

-- Create clean api_credentials table (plaintext storage)
CREATE TABLE api_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    api_key TEXT NOT NULL UNIQUE,
    secret TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT 'Default',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT true
);

CREATE INDEX idx_api_credentials_api_key ON api_credentials(api_key);
CREATE INDEX idx_api_credentials_active ON api_credentials(is_active);
