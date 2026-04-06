-- Fallback memory lock table for MCP servers that don't support lock_memory natively.
-- Kernel checks this table before forwarding delete/update requests.
CREATE TABLE IF NOT EXISTS memory_locks (
    server_id  TEXT NOT NULL,
    memory_id  INTEGER NOT NULL,
    locked_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (server_id, memory_id)
);
