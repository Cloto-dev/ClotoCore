-- Optimize prefix searches for plugin data
CREATE INDEX IF NOT EXISTS idx_plugin_data_plugin_key ON plugin_data (plugin_id, key);
