-- Persist the endpoint and bearer credential required to reconnect remote MCP servers.
ALTER TABLE mcp_servers ADD COLUMN url TEXT;
ALTER TABLE mcp_servers ADD COLUMN auth_token TEXT;
