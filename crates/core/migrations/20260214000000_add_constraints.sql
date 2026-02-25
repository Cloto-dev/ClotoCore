-- Add CHECK constraints and cleanup triggers for data integrity

-- Remove redundant index (plugin_data already has PRIMARY KEY(plugin_id, key)
-- which creates an implicit index)
DROP INDEX IF EXISTS idx_plugin_data_plugin_key;

-- Audit log retention: auto-delete entries older than 90 days
-- This trigger fires after every INSERT to keep the table bounded
CREATE TRIGGER IF NOT EXISTS audit_log_cleanup
AFTER INSERT ON audit_logs
BEGIN
    DELETE FROM audit_logs
    WHERE timestamp < datetime('now', '-90 days');
END;
