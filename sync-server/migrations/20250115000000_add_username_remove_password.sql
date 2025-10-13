-- Add username column for user identification (optional, for future use)
-- Username allows users to change their email while maintaining stable identifier
ALTER TABLE users ADD COLUMN username TEXT;

-- Add unique constraint (NULL values are allowed and don't conflict)
ALTER TABLE users ADD CONSTRAINT users_username_unique UNIQUE (username);

-- Remove password_hash column (no longer needed)
ALTER TABLE users DROP COLUMN password_hash;

-- Create index for efficient username lookups (when usernames are set)
CREATE INDEX idx_users_username ON users(username) WHERE username IS NOT NULL;
