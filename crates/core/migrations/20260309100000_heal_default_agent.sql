-- Healing migration: Ensure default agent and MCP infrastructure are intact.
--
-- Fixes cascading data loss from 20260304200000 where INSERT OR IGNORE
-- silently dropped ALL default MCP grants (missing NOT NULL granted_at).
-- Also ensures the default agent exists for fresh installs and recovery.

-- 1. Clean up stale memory.ks22 entries that may have been recreated
--    by 20260308000000 on databases where 20260309000000 already ran.
DELETE FROM mcp_access_control WHERE server_id = 'memory.ks22';
DELETE FROM mcp_servers WHERE name = 'memory.ks22';

-- 2. Ensure memory.cpersona server record exists
INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at)
VALUES ('memory.cpersona', 'config-loaded', '[]', strftime('%s', 'now'));

-- 3. Ensure all other default server records exist
INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at)
VALUES
    ('tool.cron',        'config-loaded', '[]', strftime('%s', 'now')),
    ('tool.terminal',    'config-loaded', '[]', strftime('%s', 'now')),
    ('tool.websearch',   'config-loaded', '[]', strftime('%s', 'now')),
    ('tool.research',    'config-loaded', '[]', strftime('%s', 'now')),
    ('tool.agent_utils', 'config-loaded', '[]', strftime('%s', 'now'));

-- 4. Ensure default agent exists (recovery for fresh or corrupted DBs)
INSERT OR IGNORE INTO agents
    (id, name, description, default_engine_id, status, metadata, enabled, last_seen)
VALUES
    ('agent.cloto_default',
     'Cloto Assistant',
     'Cloto''s default AI assistant. Operates as the primary interface between the user and the Cloto platform. Has access to persistent memory, tool execution, and web search capabilities. Communicates naturally and assists with system management, information retrieval, and general tasks.',
     'mind.cerebras',
     'online',
     '{}',
     1,
     0);

-- 5. Ensure default agent MCP grants exist (idempotent: skip if already granted)
INSERT INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_at)
SELECT 'server_grant', 'agent.cloto_default', s.server_name, 'allow', datetime('now')
FROM (
    SELECT 'memory.cpersona' AS server_name
    UNION ALL SELECT 'tool.cron'
    UNION ALL SELECT 'tool.terminal'
    UNION ALL SELECT 'tool.websearch'
    UNION ALL SELECT 'tool.research'
    UNION ALL SELECT 'tool.agent_utils'
) s
WHERE NOT EXISTS (
    SELECT 1 FROM mcp_access_control
    WHERE entry_type = 'server_grant'
      AND agent_id = 'agent.cloto_default'
      AND server_id = s.server_name
);
