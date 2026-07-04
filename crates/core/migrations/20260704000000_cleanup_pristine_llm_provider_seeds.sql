-- Remove pristine seeded llm_providers rows (Goal #148 follow-up).
--
-- Provider metadata now flows from the ClotoHub catalog at install time
-- (marketplace ingest, PR #250): a provider row is created/refreshed when
-- its engine is installed, so the historical migration seeds are no longer
-- the source of truth. On a fresh install they only produce ghost rows for
-- engines that were never installed (hidden by the dashboard as
-- `catalog_only`, but still dead data).
--
-- A row is deleted only when ALL of the following hold:
--   1. No user-owned setting was ever applied: api_key empty,
--      context_length unset, thinking_mode at its 'auto' default.
--      (Same "configured" predicate the /api/llm/providers handler uses;
--      model_id is compared against the exact per-id seed value instead,
--      because the seeds wrote non-empty defaults there.)
--   2. model_id still equals the value the seed migrations left behind,
--      so a user-chosen model is never discarded.
--   3. No MCP server with the same id is registered: an installed engine
--      needs its row for env augmentation, and pre-catalog installs would
--      not be re-ingested until reinstall.
--
-- Deleted rows are recreated (with fresh catalog metadata) the next time
-- the engine is installed. The old seed migrations stay in place for
-- checksum history but are frozen: new engines must not add seeds.
DELETE FROM llm_providers
WHERE (api_key IS NULL OR api_key = '')
  AND context_length IS NULL
  AND thinking_mode = 'auto'
  AND (
        (id = 'deepseek' AND model_id = 'deepseek-chat')
     OR (id = 'cerebras' AND model_id = 'gpt-oss-120b')
     OR (id = 'ollama'   AND model_id = '')
     OR (id = 'claude'   AND model_id = 'claude-sonnet-4-6')
     OR (id = 'groq'     AND model_id = 'openai/gpt-oss-120b')
     OR (id = 'local'    AND model_id = '')
  )
  AND id NOT IN (SELECT name FROM mcp_servers);
