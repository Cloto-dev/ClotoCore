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
-- Only insert when the referenced agent actually exists (prevents FK violation).

-- agent.exiv_default: had mind.cerebras at (0,0)
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    SELECT 'agent.exiv_default', 'mind.cerebras', 0, 0
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = 'agent.exiv_default');

-- agent.deepseek_test: had mind.deepseek at (0,0)
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    SELECT 'agent.deepseek_test', 'mind.deepseek', 0, 0
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = 'agent.deepseek_test');

-- Seed default Exiv agent with system capabilities.
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    SELECT 'agent.exiv_default', 'core.ks22', 1, 0
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = 'agent.exiv_default');
INSERT OR IGNORE INTO agent_plugins (agent_id, plugin_id, pos_x, pos_y)
    SELECT 'agent.exiv_default', 'core.skill_manager', 2, 0
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = 'agent.exiv_default');
