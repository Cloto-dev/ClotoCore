-- Default Plugins
INSERT OR REPLACE INTO plugin_settings (plugin_id, is_active, allowed_permissions) VALUES 
('core.ks22', 1, '[]'),
('mind.deepseek', 1, '["NetworkAccess"]'),
('mind.cerebras', 1, '["NetworkAccess"]'),
('core.moderator', 1, '[]'),
('hal.cursor', 1, '[]'),
('python.analyst', 1, '[]'),
('python.gaze', 1, '["VisionRead"]'),
('adapter.mcp', 1, '["ProcessExecution"]'),
('vision.screen', 1, '["VisionRead"]');

-- Default Configs
INSERT OR REPLACE INTO plugin_configs (plugin_id, config_key, config_value) VALUES 
('python.analyst', 'script_path', 'scripts/bridge_main.py'),
('python.gaze', 'script_path', 'scripts/vision_gaze_webcam.py');

-- Default Agents
INSERT OR REPLACE INTO agents (id, name, description, default_engine_id, status, metadata) VALUES 
('agent.exiv_default', 'Exiv Assistant', 'The primary system assistant.', 'mind.deepseek', 'online', '{}'),
('agent.analyst', 'Python Analyst', 'Advanced Data Analyst powered by Python Bridge', 'python.analyst', 'online', '{"preferred_memory": "core.ks22"}');
