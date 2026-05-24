-- 0.6.6 memory plugin decouple Phase D: rename legacy `memory.cpersona` → `cpersona`
--
-- After the 0.6.6 decouple (PR B = #137), the kernel no longer cares about the
-- plugin id — it discovers memory plugins by capability. The id `cpersona`
-- (without the `memory.` prefix) is the canonical ClotoHub catalog convention;
-- new installs come down as `cpersona` from the marketplace. This migration
-- normalises pre-0.6.6 user data so all rows (legacy + new) use the same id.
--
-- Precedent: `20260309000000_rename_ks22_to_cpersona.sql` performed an identical
-- shape of rename (mcp_servers → mcp_access_control DELETE + re-INSERT around
-- the rename, because `mcp_access_control.server_id` FK → `mcp_servers(name)`
-- blocks both orderings of a direct UPDATE).
--
-- Idempotent: WHERE clauses target only the legacy `memory.cpersona` rows; if
-- the migration has already run (or this user installed straight from the
-- post-0.6.6 catalog) every UPDATE/INSERT is a zero-row no-op.
--
-- agents.metadata.preferred_memory: NOT touched here. Phase C wrote `cpersona`
-- directly; if any pre-existing manual setting points at `memory.cpersona`,
-- the cosmetic cleanup is handled by the second UPDATE below.

-- 1. Drop the FK reference rows so we can rename the parent.
DELETE FROM mcp_access_control WHERE server_id = 'memory.cpersona';

-- 2. Rename the server itself.
UPDATE mcp_servers SET name = 'cpersona' WHERE name = 'memory.cpersona';

-- 3. Re-grant default access under the new id (mirrors the KS22 precedent —
--    preserves the default agent's ability to invoke the memory plugin).
INSERT OR IGNORE INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_by, granted_at)
SELECT 'server_grant', 'agent.cloto_default', 'cpersona', 'allow', 'migration', datetime('now')
WHERE EXISTS (SELECT 1 FROM mcp_servers WHERE name = 'cpersona');

-- 4. plugin_configs has no FK; direct UPDATE is safe.
UPDATE plugin_configs SET plugin_id = 'cpersona' WHERE plugin_id = 'memory.cpersona';

-- 5. Cosmetic cleanup: any agent metadata that explicitly carried the legacy id
--    (e.g. user who manually pinned `memory.cpersona` via dashboard pre-0.6.6)
--    is normalised to `cpersona`. Phase C already wrote `cpersona` for empty
--    fields, so this only catches the explicit-legacy-value case.
UPDATE agents
SET metadata = json_set(
    COALESCE(metadata, '{}'),
    '$.preferred_memory',
    'cpersona'
)
WHERE json_extract(metadata, '$.preferred_memory') = 'memory.cpersona';
