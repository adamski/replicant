-- API credentials table
-- Note: Credentials are application-wide, not tied to individual users
-- Multiple end-users can authenticate through the same application credentials
CREATE TABLE IF NOT EXISTS api_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    api_key_hash TEXT NOT NULL,
    secret_hash TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT 'Default',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ,
    is_active BOOLEAN NOT NULL DEFAULT true
);

-- Index for efficient lookups
CREATE INDEX idx_api_credentials_active ON api_credentials(is_active);