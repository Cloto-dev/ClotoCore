-- Conversations: the persistent unit of the dashboard chat (docs/CONVERSATIONS_DESIGN.md).
-- A message belongs to one conversation; the model reads that conversation as its context.
CREATE TABLE IF NOT EXISTS conversations (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    user_id     TEXT NOT NULL DEFAULT 'default',
    title       TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,   -- Unix timestamp ms
    updated_at  INTEGER NOT NULL,   -- ms; the newest message's time
    archived_at INTEGER,            -- NULL = live; set = hidden from the list, kept whole
    FOREIGN KEY (agent_id) REFERENCES agents(id)
);
ALTER TABLE chat_messages ADD COLUMN conversation_id TEXT;
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation
    ON chat_messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS idx_conversations_agent
    ON conversations(agent_id, user_id, archived_at, updated_at DESC);

-- Existing history becomes one conversation per (agent, user): the thread the
-- person actually saw. No row is moved, split or dropped. The id doubles as the
-- default conversation a message without an id lands in (see db/conversations.rs).
INSERT INTO conversations (id, agent_id, user_id, title, created_at, updated_at, archived_at)
SELECT 'default:' || agent_id || ':' || user_id,
       agent_id,
       user_id,
       strftime('%Y-%m-%d', MIN(created_at) / 1000, 'unixepoch')
           || ' – ' ||
       strftime('%Y-%m-%d', MAX(created_at) / 1000, 'unixepoch'),
       MIN(created_at),
       MAX(created_at),
       NULL
FROM chat_messages
GROUP BY agent_id, user_id;

UPDATE chat_messages
SET conversation_id = 'default:' || agent_id || ':' || user_id
WHERE conversation_id IS NULL;
