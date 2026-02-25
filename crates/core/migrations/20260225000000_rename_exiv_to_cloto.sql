-- Rename agent.exiv_default → agent.cloto_default
-- Strategy: insert new → migrate references → delete old (avoids FK violation)

-- 1. Insert new agent with cloto_default ID (copy from exiv_default if it exists)
INSERT OR IGNORE INTO agents (id, name, description, default_engine_id, status, metadata, enabled, last_seen, power_password_hash)
    SELECT 'agent.cloto_default', 'Cloto Assistant', description, default_engine_id, status, metadata, enabled, last_seen, power_password_hash
    FROM agents WHERE id = 'agent.exiv_default';

-- 2. Migrate agent_plugins references
UPDATE agent_plugins SET agent_id = 'agent.cloto_default'
    WHERE agent_id = 'agent.exiv_default';

-- 3. Migrate chat_messages references (if any exist)
UPDATE chat_messages SET agent_id = 'agent.cloto_default'
    WHERE agent_id = 'agent.exiv_default';

-- 4. Delete old agent
DELETE FROM agents WHERE id = 'agent.exiv_default';
