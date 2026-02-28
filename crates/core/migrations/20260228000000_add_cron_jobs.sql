-- Cron job scheduler for autonomous agent execution (Layer 2: Autonomous Trigger)
CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    schedule_type TEXT NOT NULL DEFAULT 'interval',  -- 'interval' | 'cron' | 'once'
    schedule_value TEXT NOT NULL,                     -- seconds | cron expr | ISO 8601
    engine_id TEXT,                                   -- NULL = use agent default
    message TEXT NOT NULL,                            -- prompt sent to agent
    next_run_at INTEGER NOT NULL,                     -- unix milliseconds
    last_run_at INTEGER,
    last_status TEXT,                                 -- 'success' | 'error'
    last_error TEXT,
    max_iterations INTEGER DEFAULT 8,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY(agent_id) REFERENCES agents(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_cron_next ON cron_jobs(next_run_at) WHERE enabled = 1;
CREATE INDEX IF NOT EXISTS idx_cron_agent ON cron_jobs(agent_id);
