-- Model change history for llm_providers. Internal-only log for future
-- features (analytics, rollback UI). No UI exposure in this phase.
CREATE TABLE IF NOT EXISTS llm_provider_model_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL REFERENCES llm_providers(id) ON DELETE CASCADE,
    old_model_id TEXT,
    new_model_id TEXT NOT NULL,
    changed_at TEXT NOT NULL DEFAULT (datetime('now')),
    changed_by TEXT NOT NULL DEFAULT 'admin'
);
CREATE INDEX idx_model_history_provider ON llm_provider_model_history(provider_id, changed_at DESC);
