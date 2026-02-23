-- MCP Access Control: Unified access control table
-- Merges legacy permission_requests with new tool-level access grants
-- Design: docs/MCP_SERVER_UI_DESIGN.md §3

CREATE TABLE IF NOT EXISTS mcp_access_control (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_type TEXT NOT NULL CHECK(entry_type IN ('capability', 'server_grant', 'tool_grant')),
    agent_id TEXT NOT NULL,
    server_id TEXT NOT NULL,
    tool_name TEXT,
    permission TEXT NOT NULL DEFAULT 'allow',
    granted_by TEXT,
    granted_at TEXT NOT NULL,
    expires_at TEXT,
    justification TEXT,
    metadata TEXT
);

CREATE INDEX IF NOT EXISTS idx_ac_agent_server_tool ON mcp_access_control(agent_id, server_id, tool_name);
CREATE INDEX IF NOT EXISTS idx_ac_server ON mcp_access_control(server_id);
CREATE INDEX IF NOT EXISTS idx_ac_entry_type ON mcp_access_control(entry_type);

-- Add default_policy column to mcp_servers
ALTER TABLE mcp_servers ADD COLUMN default_policy TEXT NOT NULL DEFAULT 'opt-in';
