-- Add env column to mcp_servers for storing environment variables (JSON map)
ALTER TABLE mcp_servers ADD COLUMN env TEXT NOT NULL DEFAULT '{}';
