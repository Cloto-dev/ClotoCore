-- Search across the dashboard chat: a full-text index of each message's text
-- (db/chat_search.rs).
--
-- Trigram tokens, because a word tokenizer does not split Japanese: a run of
-- kana and kanji is one token to it, and a phrase inside the run never matches.
-- A term of three characters or more is served by the index; a shorter one is
-- matched by scanning the indexed text.
--
-- The index is keyed through chat_message_search_keys, whose INTEGER PRIMARY
-- KEY survives VACUUM. chat_messages has a TEXT primary key, so its implicit
-- rowid may be renumbered by VACUUM and cannot key an index kept elsewhere.
CREATE TABLE IF NOT EXISTS chat_message_search_keys (
    key        INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE
);

CREATE VIRTUAL TABLE IF NOT EXISTS chat_message_search USING fts5(
    body,
    tokenize = 'trigram'
);

-- What is indexed is the text blocks of a message's content (a JSON array of
-- ContentBlock), joined by newlines. Content that is not a JSON array, and a
-- message with no text, index nothing; neither makes the write fail.

-- Existing history.
INSERT INTO chat_message_search_keys (message_id)
SELECT id FROM chat_messages ORDER BY created_at, id;

INSERT INTO chat_message_search (rowid, body)
SELECT key, body FROM (
    SELECT k.key AS key,
           (SELECT group_concat(json_extract(e.value, '$.text'), char(10))
            FROM json_each(CASE WHEN json_valid(m.content)
                                THEN CASE WHEN json_type(m.content) = 'array' THEN m.content ELSE '[]' END
                                ELSE '[]' END) AS e
            WHERE e.type = 'object' AND json_extract(e.value, '$.type') = 'text') AS body
    FROM chat_messages m
    JOIN chat_message_search_keys k ON k.message_id = m.id
)
WHERE body IS NOT NULL AND body <> '';

-- New messages.
CREATE TRIGGER IF NOT EXISTS chat_message_search_after_insert
AFTER INSERT ON chat_messages
BEGIN
    INSERT INTO chat_message_search_keys (message_id) VALUES (NEW.id);
    INSERT INTO chat_message_search (rowid, body)
    SELECT key, body FROM (
        SELECT (SELECT key FROM chat_message_search_keys WHERE message_id = NEW.id) AS key,
               (SELECT group_concat(json_extract(e.value, '$.text'), char(10))
                FROM json_each(CASE WHEN json_valid(NEW.content)
                                    THEN CASE WHEN json_type(NEW.content) = 'array' THEN NEW.content ELSE '[]' END
                                    ELSE '[]' END) AS e
                WHERE e.type = 'object' AND json_extract(e.value, '$.type') = 'text') AS body
    )
    WHERE body IS NOT NULL AND body <> '';
END;

-- Rewritten content: the message keeps its key, and the text is indexed again.
CREATE TRIGGER IF NOT EXISTS chat_message_search_after_update
AFTER UPDATE OF content ON chat_messages
BEGIN
    DELETE FROM chat_message_search
    WHERE rowid = (SELECT key FROM chat_message_search_keys WHERE message_id = OLD.id);
    INSERT INTO chat_message_search (rowid, body)
    SELECT key, body FROM (
        SELECT (SELECT key FROM chat_message_search_keys WHERE message_id = NEW.id) AS key,
               (SELECT group_concat(json_extract(e.value, '$.text'), char(10))
                FROM json_each(CASE WHEN json_valid(NEW.content)
                                    THEN CASE WHEN json_type(NEW.content) = 'array' THEN NEW.content ELSE '[]' END
                                    ELSE '[]' END) AS e
                WHERE e.type = 'object' AND json_extract(e.value, '$.type') = 'text') AS body
    )
    WHERE key IS NOT NULL AND body IS NOT NULL AND body <> '';
END;

-- Deleted messages (one at a time, a conversation, an agent): nothing of them
-- stays findable.
CREATE TRIGGER IF NOT EXISTS chat_message_search_after_delete
AFTER DELETE ON chat_messages
BEGIN
    DELETE FROM chat_message_search
    WHERE rowid = (SELECT key FROM chat_message_search_keys WHERE message_id = OLD.id);
    DELETE FROM chat_message_search_keys WHERE message_id = OLD.id;
END;
