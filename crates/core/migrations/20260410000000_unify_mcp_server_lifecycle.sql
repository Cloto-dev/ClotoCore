-- Unify MCP server lifecycle: DB becomes sole source of truth.
-- mcp.toml is deprecated; all server state lives in mcp_servers table.
-- The existing 'source' column remains (SQLite cannot DROP COLUMN) but is ignored by code.

ALTER TABLE mcp_servers ADD COLUMN transport TEXT NOT NULL DEFAULT 'stdio';
ALTER TABLE mcp_servers ADD COLUMN directory TEXT;
ALTER TABLE mcp_servers ADD COLUMN display_name TEXT;
ALTER TABLE mcp_servers ADD COLUMN updated_at INTEGER;
ALTER TABLE mcp_servers ADD COLUMN auto_restart BOOLEAN NOT NULL DEFAULT 1;

-- Backfill directory from marketplace_id where available
UPDATE mcp_servers SET directory = marketplace_id WHERE marketplace_id IS NOT NULL AND directory IS NULL;

-- Backfill updated_at from created_at
UPDATE mcp_servers SET updated_at = created_at WHERE updated_at IS NULL;
