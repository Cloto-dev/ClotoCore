-- Unread on the roster: a conversation the agent has spoken in since the person
-- last looked at it (docs/CONVERSATIONS_DESIGN.md).
--
-- The roster already marks a row when an agent is waiting on an answer, but an
-- agent that simply said something — a scheduled run reporting, a reply that
-- arrived after the person left — left no mark at all. Whether a question is
-- outstanding and whether words have been read are different questions, and the
-- notification store only answers the first.
--
-- Existing conversations are backfilled as read rather than left NULL. NULL
-- reads as "never opened", which would put a mark beside every agent the moment
-- this lands, for history the person has in fact already seen.
ALTER TABLE conversations ADD COLUMN last_read_at INTEGER;  -- ms; NULL = never opened
UPDATE conversations SET last_read_at = updated_at;
