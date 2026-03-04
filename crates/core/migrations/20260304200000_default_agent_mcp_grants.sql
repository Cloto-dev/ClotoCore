-- Grant default MCP servers to Cloto Assistant (agent.cloto_default).
-- These server_grant entries make the servers explicitly associated with
-- the default agent, improving UX for new users who expect the assistant
-- to have useful capabilities out of the box.
--
-- Note: default_policy is already opt-out (allow all), so these grants
-- are primarily for UI visibility (dashboard shows assigned servers)
-- and for get_granted_server_ids() which checks explicit server_grants.

INSERT OR IGNORE INTO mcp_access_control (entry_type, agent_id, server_id, permission)
VALUES
    ('server_grant', 'agent.cloto_default', 'memory.ks22',     'allow'),
    ('server_grant', 'agent.cloto_default', 'tool.cron',       'allow'),
    ('server_grant', 'agent.cloto_default', 'tool.terminal',   'allow'),
    ('server_grant', 'agent.cloto_default', 'tool.websearch',  'allow'),
    ('server_grant', 'agent.cloto_default', 'tool.research',   'allow');
