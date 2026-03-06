-- Revoked keys TTL: auto-delete entries older than 90 days (bug-158)
-- Mirrors the audit_log_cleanup trigger pattern from 20260214000000_add_constraints.sql
CREATE TRIGGER IF NOT EXISTS revoked_keys_cleanup
AFTER INSERT ON revoked_keys
BEGIN
    DELETE FROM revoked_keys
    WHERE revoked_at < (strftime('%s', 'now') - 90 * 86400) * 1000;
END;
