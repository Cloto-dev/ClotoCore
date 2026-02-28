-- Default Plugins
INSERT OR REPLACE INTO plugin_settings (plugin_id, is_active, allowed_permissions) VALUES
('memory.ks22', 1, '[]'),
('mind.deepseek', 1, '["NetworkAccess"]'),
('mind.cerebras', 1, '["NetworkAccess"]');

-- Default Agents
INSERT OR REPLACE INTO agents (id, name, description, default_engine_id, status, metadata) VALUES
('agent.exiv_default', 'Exiv Assistant', 'The primary system assistant.', 'mind.deepseek', 'online', '{}');
