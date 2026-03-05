-- Add branching support to chat_messages (edit/retry → new branches)
ALTER TABLE chat_messages ADD COLUMN parent_id TEXT;
ALTER TABLE chat_messages ADD COLUMN branch_index INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_chat_messages_parent ON chat_messages(parent_id, branch_index);
