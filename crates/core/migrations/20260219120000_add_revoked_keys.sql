-- Revoked API keys table
-- Stores SHA-256 hashes of invalidated keys.
-- A revoked key is permanently blocked even if still present in .env.
-- To restore access, restart with a new CLOTO_API_KEY.
CREATE TABLE IF NOT EXISTS revoked_keys (
    key_hash TEXT PRIMARY KEY,
    revoked_at INTEGER NOT NULL
);
