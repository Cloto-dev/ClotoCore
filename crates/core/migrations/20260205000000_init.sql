-- Initial schema for Exiv Kernel (Consolidated)

CREATE TABLE IF NOT EXISTS plugin_settings (
    plugin_id TEXT PRIMARY KEY,
    is_active BOOLEAN NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS plugin_configs (
    plugin_id TEXT,
    config_key TEXT,
    config_value TEXT,
    PRIMARY KEY(plugin_id, config_key)
);

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    default_engine_id TEXT NOT NULL,
    status TEXT DEFAULT 'offline',
    metadata TEXT DEFAULT '{}'
);

-- Initial Seeds
INSERT OR IGNORE INTO agents (id, name, description, default_engine_id, status, metadata)
VALUES ('agent.exiv_default', 'Exiv Assistant', 'The primary system assistant.', 'mind.deepseek', 'online', '{}');