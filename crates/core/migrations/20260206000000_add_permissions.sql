-- Add allowed_permissions column safely using a transient table or conditional logic
-- Since SQLite doesn't have "IF NOT EXISTS" for ADD COLUMN, 
-- we use a more robust way: ignore the error if it fails because the column is there.

-- We'll use a trick: Create a temporary trigger or just do it in one go
-- But wait, SQLx fails the whole migration on any error.
-- Let's use the most reliable SQLite way: create a new table, copy data, drop old one.

CREATE TABLE plugin_settings_new (
    plugin_id TEXT PRIMARY KEY,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    allowed_permissions TEXT DEFAULT '[]'
);

INSERT OR IGNORE INTO plugin_settings_new (plugin_id, is_active)
SELECT plugin_id, is_active FROM plugin_settings;

DROP TABLE plugin_settings;
ALTER TABLE plugin_settings_new RENAME TO plugin_settings;

-- Update existing plugins with sensible default permissions
UPDATE plugin_settings SET allowed_permissions = '["NetworkAccess"]' WHERE plugin_id = 'mind.deepseek';
UPDATE plugin_settings SET allowed_permissions = '["NetworkAccess"]' WHERE plugin_id = 'mind.cerebras';
