-- Trusted commands for the command approval system.
-- Stores exact-match commands that the user has approved permanently (per-agent).
-- Command-name trust is session-scoped (in-memory only, not stored here).

CREATE TABLE IF NOT EXISTS trusted_commands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    pattern TEXT NOT NULL,
    pattern_type TEXT NOT NULL DEFAULT 'exact' CHECK(pattern_type IN ('exact')),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(agent_id, pattern, pattern_type)
);

CREATE INDEX IF NOT EXISTS idx_trusted_commands_agent ON trusted_commands(agent_id);
