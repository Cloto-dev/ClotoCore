-- Per-agent plugin assignment table.
-- Replaces plugin_layout JSON stored in agents.metadata.
-- Controls which tools are available to each agent.
CREATE TABLE IF NOT EXISTS agent_plugins (
    agent_id  TEXT    NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    plugin_id TEXT    NOT NULL,
    pos_x     INTEGER NOT NULL DEFAULT 0,
    pos_y     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, plugin_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_plugins_agent ON agent_plugins(agent_id);

-- Migrate existing plugin_layout metadata to the new table.
-- agent.exiv_default: had mind.cerebras at (0,0)
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    VALUES ('agent.exiv_default', 'mind.cerebras', 0, 0);

-- agent.deepseek_test: had mind.deepseek at (0,0)
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    VALUES ('agent.deepseek_test', 'mind.deepseek', 0, 0);

-- Seed default Exiv agent with system capabilities.
-- These are placed in the grid alongside the reasoning engine.
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    VALUES ('agent.exiv_default', 'core.ks22',          1, 0);
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    VALUES ('agent.exiv_default', 'core.skill_manager', 2, 0);
