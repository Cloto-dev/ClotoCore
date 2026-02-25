-- Permission Requests Table for Human-in-the-Loop Workflow
-- Stores pending permission requests that require human approval

CREATE TABLE IF NOT EXISTS permission_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    plugin_id TEXT NOT NULL,
    permission_type TEXT NOT NULL,
    target_resource TEXT,
    justification TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    approved_by TEXT,
    approved_at TEXT,
    expires_at TEXT,
    metadata TEXT
);

CREATE INDEX IF NOT EXISTS idx_permission_status ON permission_requests(status);
CREATE INDEX IF NOT EXISTS idx_permission_plugin ON permission_requests(plugin_id);
CREATE INDEX IF NOT EXISTS idx_permission_created ON permission_requests(created_at);
CREATE INDEX IF NOT EXISTS idx_permission_request_id ON permission_requests(request_id);
