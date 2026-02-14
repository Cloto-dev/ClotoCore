-- Migration to add generic plugin storage (SAL)
CREATE TABLE IF NOT EXISTS plugin_data (
    plugin_id TEXT,
    key TEXT,
    value TEXT,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(plugin_id, key)
);
