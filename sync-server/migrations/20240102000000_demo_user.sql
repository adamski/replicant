-- Create demo user for testing
-- Token: demo-token (hashed with argon2)

INSERT INTO users (email, auth_token_hash, created_at)
VALUES (
    'demo@example.com',
    '$argon2id$v=19$m=19456,t=2,p=1$w7yd7e3//w/u1MM3eoA/tw$u7/4HyenkGRaAcuOJePehvgwyP7/5W4sxGnt/rN/dFs',
    NOW()
) ON CONFLICT (email) DO NOTHING;