-- Chat persistence: server-side message storage with rich content support

CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT 'default',
    source TEXT NOT NULL CHECK (source IN ('user', 'agent', 'system')),
    content TEXT NOT NULL,         -- JSON array of ContentBlock[]
    metadata TEXT,                 -- optional JSON metadata
    created_at INTEGER NOT NULL,   -- Unix timestamp ms
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_agent_time
    ON chat_messages(agent_id, user_id, created_at DESC);

CREATE TABLE IF NOT EXISTS chat_attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    storage_type TEXT NOT NULL CHECK (storage_type IN ('inline', 'disk')),
    inline_data BLOB,             -- for <=64KB files
    disk_path TEXT,               -- for >64KB files
    created_at INTEGER NOT NULL,
    FOREIGN KEY (message_id) REFERENCES chat_messages(id) ON DELETE CASCADE
);
