-- Argument rules narrow what an agent may pass to a server's tools. A grant says
-- whether the agent may call a tool at all; a rule says which argument values
-- that call must carry. Kept apart from mcp_access_control because the bulk
-- grant updates delete and re-insert grant rows: a rule stored on a grant row
-- would vanish the next time an operator saved the agent's access.
CREATE TABLE IF NOT EXISTS mcp_argument_rules (
    agent_id   TEXT NOT NULL,
    server_id  TEXT NOT NULL,
    -- '' applies the rule to every tool on the server
    tool_name  TEXT NOT NULL DEFAULT '',
    -- JSON object: argument name -> the value it must equal whenever present
    equals     TEXT NOT NULL DEFAULT '{}',
    -- JSON array: names of arguments that must be present
    required   TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (agent_id, server_id, tool_name)
);
