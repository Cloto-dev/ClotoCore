-- Grant tool.agent_utils to Cloto Assistant (agent.cloto_default).

INSERT OR IGNORE INTO mcp_access_control (entry_type, agent_id, server_id, permission)
VALUES ('server_grant', 'agent.cloto_default', 'tool.agent_utils', 'allow');
