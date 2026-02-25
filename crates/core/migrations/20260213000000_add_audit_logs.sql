-- Add audit logs table for security event tracking
CREATE TABLE IF NOT EXISTS audit_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor_id TEXT,
    target_id TEXT,
    permission TEXT,
    result TEXT NOT NULL,
    reason TEXT,
    metadata TEXT,
    trace_id TEXT
);

-- Indexes for efficient querying
CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_logs(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_event_type ON audit_logs(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_trace_id ON audit_logs(trace_id);
