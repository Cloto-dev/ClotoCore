-- Dynamic MCP server persistence
-- Stores runtime-added MCP server configurations for restart restoration
CREATE TABLE IF NOT EXISTS mcp_servers (
    name TEXT PRIMARY KEY,
    command TEXT NOT NULL,
    args TEXT NOT NULL DEFAULT '[]',
    script_content TEXT,
    description TEXT,
    created_at INTEGER NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 1
);
