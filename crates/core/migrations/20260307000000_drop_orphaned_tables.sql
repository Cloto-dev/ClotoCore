-- Drop orphaned tables that are no longer referenced by any runtime code.
--
-- runtime_plugins: Created for L5 Self-Extension (Python Bridge).
--   All runtime code deleted in commit 03a5f79 (2026-02-22).
--
-- agent_plugins: Created for per-agent plugin grid layout.
--   All runtime code deleted in commit 6a7fd6f (2026-02-28, bug-127~138).

DROP TABLE IF EXISTS runtime_plugins;
DROP TABLE IF EXISTS agent_plugins;
