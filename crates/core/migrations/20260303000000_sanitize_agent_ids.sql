-- Sanitize agent IDs containing URL-unsafe characters (e.g., slashes).
-- Replace '/' with '_' in agent IDs across all referencing tables.
UPDATE chat_messages SET agent_id = REPLACE(agent_id, '/', '_') WHERE agent_id LIKE '%/%';
UPDATE agents SET id = REPLACE(id, '/', '_') WHERE id LIKE '%/%';
