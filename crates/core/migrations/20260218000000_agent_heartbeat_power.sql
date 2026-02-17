-- Add dynamic status resolution fields (replaces static 'status' column)
ALTER TABLE agents ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT 1;
ALTER TABLE agents ADD COLUMN last_seen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN power_password_hash TEXT DEFAULT NULL;

-- Migrate existing data: status='online' → enabled=1, others → enabled=0
UPDATE agents SET enabled = CASE WHEN status = 'online' THEN 1 ELSE 0 END;
-- Set last_seen to current timestamp (ms) for enabled agents
UPDATE agents SET last_seen = (strftime('%s', 'now') * 1000) WHERE enabled = 1;
