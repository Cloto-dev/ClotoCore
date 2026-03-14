-- Add marketplace tracking fields to mcp_servers table.
-- source: 'config' (mcp.toml), 'dynamic' (API/YOLO), 'marketplace' (marketplace install)
ALTER TABLE mcp_servers ADD COLUMN source TEXT NOT NULL DEFAULT 'dynamic';
ALTER TABLE mcp_servers ADD COLUMN installed_version TEXT;
ALTER TABLE mcp_servers ADD COLUMN marketplace_id TEXT;
