-- 0.6.6 memory plugin decouple Phase C: back-fill agent.metadata.preferred_memory
--
-- Pre-0.6.6 the kernel hardcoded `memory.cpersona` as the default memory plugin
-- via CLOTO_MEMORY_PLUGIN_ID env (config.rs default) + seeded plugin_configs at
-- boot (db/mod.rs::init_db). Existing users therefore have a working memory
-- binding even though `agent.cloto_default.metadata.preferred_memory` was never
-- written.
--
-- After 0.6.6 (PR B = #137), the kernel reads `preferred_memory` as the single
-- source of truth and the new Setup Wizard / marketplace install path writes
-- it automatically (handlers/marketplace.rs::bind_default_memory_if_unset).
-- This migration makes existing users converge: when the default agent has no
-- preferred_memory yet AND a memory plugin is installed (legacy `memory.cpersona`
-- or new `cpersona`), write the canonical id `cpersona` directly.
--
-- C+D are bundled in a single PR so this migration can write the post-rename id
-- `cpersona` unconditionally — Phase D (next migration) handles the runtime row
-- rename so the value stays valid.
--
-- Idempotent:
--   - Touches nothing if preferred_memory already has a value (manual choice
--     via dashboard, or earlier migration run, etc).
--   - Touches nothing if no memory plugin is installed yet (fresh kernel before
--     first Setup Wizard run).

UPDATE agents
SET metadata = json_set(
    COALESCE(metadata, '{}'),
    '$.preferred_memory',
    'cpersona'
)
WHERE id = 'agent.cloto_default'
  AND (
    json_extract(metadata, '$.preferred_memory') IS NULL
    OR json_extract(metadata, '$.preferred_memory') = ''
  )
  AND EXISTS (
    SELECT 1 FROM mcp_servers WHERE name IN ('memory.cpersona', 'cpersona')
  );
