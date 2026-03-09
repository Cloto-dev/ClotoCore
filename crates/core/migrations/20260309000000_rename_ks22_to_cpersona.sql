-- Rename memory.ks22 → memory.cpersona across all tables that store the server ID.
-- Note: agent_plugins was dropped in 20260307000000_drop_orphaned_tables.sql.
-- mcp_access_control.server_id has FK → mcp_servers(name).
-- SQLite FK enforcement blocks both orderings (parent-first breaks child refs,
-- child-first breaks FK check). Use DELETE + INSERT to work around.

-- 1. Remove old access control entries (FK cascade not needed since we re-insert)
DELETE FROM mcp_access_control WHERE server_id = 'memory.ks22';

-- 2. Rename the server itself
UPDATE mcp_servers SET name = 'memory.cpersona' WHERE name = 'memory.ks22';

-- 3. Re-grant default access under the new server ID
INSERT OR IGNORE INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_by, granted_at)
VALUES ('server_grant', 'agent.cloto_default', 'memory.cpersona', 'allow', 'migration', datetime('now'));

-- 4. Rename in plugin_configs (no FK constraint)
UPDATE plugin_configs SET plugin_id = 'memory.cpersona' WHERE plugin_id = 'memory.ks22';
