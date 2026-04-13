-- Rename mind.lmstudio → mind.local (promote to generic recommended local LLM).
--
-- The earlier seed migration (20260412100000) inserts 'lmstudio' — which this
-- migration then renames to 'local'. Fresh installs run both in sequence and
-- land on 'local'. Existing installs that already ran 20260412100000 get
-- renamed here.
--
-- Child tables are updated first (FK-safe order) since SQLite doesn't
-- auto-cascade UPDATEs on primary keys.

-- mcp_access_control.server_id → mcp_servers.name
UPDATE mcp_access_control SET server_id = 'mind.local' WHERE server_id = 'mind.lmstudio';

-- llm_provider_model_history.provider_id → llm_providers.id
UPDATE llm_provider_model_history SET provider_id = 'local' WHERE provider_id = 'lmstudio';

-- Parent: mcp_servers (also rewrite directory + args so spawn uses servers/local/)
UPDATE mcp_servers
SET name         = 'mind.local',
    display_name = 'Local LLM',
    directory    = 'local',
    args         = REPLACE(args, '/lmstudio/server.py', '/local/server.py')
WHERE name = 'mind.lmstudio';

-- Parent: llm_providers
UPDATE llm_providers
SET id           = 'local',
    display_name = 'Local LLM'
WHERE id = 'lmstudio';
