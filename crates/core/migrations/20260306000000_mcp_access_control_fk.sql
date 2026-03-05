-- H-05: Add FOREIGN KEY constraint to mcp_access_control.server_id
-- SQLite cannot ALTER TABLE to add FK, so recreate the table.

-- Ensure all config-loaded servers referenced by access_control exist in mcp_servers
-- (config-loaded servers live only in memory; this backfills them into the DB).
INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at)
    SELECT DISTINCT server_id, 'config-loaded', '[]', strftime('%s', 'now')
    FROM mcp_access_control
    WHERE server_id NOT IN (SELECT name FROM mcp_servers);

CREATE TABLE IF NOT EXISTS mcp_access_control_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_type TEXT NOT NULL CHECK(entry_type IN ('capability', 'server_grant', 'tool_grant')),
    agent_id TEXT NOT NULL,
    server_id TEXT NOT NULL REFERENCES mcp_servers(name) ON DELETE CASCADE,
    tool_name TEXT,
    permission TEXT NOT NULL DEFAULT 'allow',
    granted_by TEXT,
    granted_at TEXT NOT NULL,
    expires_at TEXT,
    justification TEXT,
    metadata TEXT
);

INSERT INTO mcp_access_control_new
    SELECT * FROM mcp_access_control;

DROP TABLE mcp_access_control;

ALTER TABLE mcp_access_control_new RENAME TO mcp_access_control;

CREATE INDEX IF NOT EXISTS idx_ac_agent_server_tool ON mcp_access_control(agent_id, server_id, tool_name);
CREATE INDEX IF NOT EXISTS idx_ac_server ON mcp_access_control(server_id);
CREATE INDEX IF NOT EXISTS idx_ac_entry_type ON mcp_access_control(entry_type);
