-- Retire the `mind.` engine-id prefix (Goal #142 / Task #128).
--
-- Collapses the dual id spelling so every reasoning engine is identified by its
-- bare id (`local`, `ollama`, `deepseek`, …). Built-in engine servers
-- (`mind.local`, `mind.ollama`) are renamed to bare across all three storage
-- sites; wizard-persisted aliases (`mind.deepseek`, `mind.cerebras`, …) that
-- shadow an already-bare catalog server are de-prefixed to match it. After this
-- runs there is exactly one spelling and the kernel neither emits nor normalizes
-- the `mind.` form (docs/MIND_PREFIX_RETIREMENT_DESIGN.md §8).
--
-- Precedent: 20260524010000_rename_memory_cpersona_to_cpersona.sql and
-- 20260309000000_rename_ks22_to_cpersona.sql — same shape. The
-- mcp_access_control.server_id FK → mcp_servers(name) (ON DELETE CASCADE) blocks
-- a naive rename of the parent while children reference it, so the grants are
-- detached, the servers renamed, then the grants re-attached under bare ids.
-- This ordering keeps the FK satisfied at every statement boundary, so it is
-- correct whether the migration runs inside a transaction (sqlx default) or in
-- autocommit — it does not rely on `defer_foreign_keys`.
--
-- Idempotent: every clause targets only `mind.%` rows, so a re-run (or a DB that
-- never had a `mind.*` row) is a zero-row no-op. Collision-safe: where a bare
-- twin already exists (catalog `deepseek`, or a partially-migrated DB) the
-- prefixed row/grant is merged into the bare one rather than duplicated.
-- Fresh-DB tolerant. This migration only renames/merges specific rows — it never
-- deletes a `.db` or truncates a table.

-- 1. Detach every `mind.*` grant (FK child), de-prefixed, into a temp table,
--    then remove the originals so the parent rows can be renamed without the
--    server_id FK blocking the rename.
CREATE TEMP TABLE _mind_prefix_grants AS
  SELECT entry_type, agent_id, substr(server_id, 6) AS server_id, tool_name,
         permission, granted_by, granted_at, expires_at, justification, metadata
    FROM mcp_access_control
   WHERE server_id LIKE 'mind.%';
DELETE FROM mcp_access_control WHERE server_id LIKE 'mind.%';

-- 2. Rename the engine server rows to bare. mcp_servers.name is PRIMARY KEY, so
--    OR IGNORE keeps an existing bare twin; the prefixed row left behind on a
--    collision is dropped. No grant references a `mind.%` name now, so
--    ON DELETE CASCADE removes nothing.
UPDATE OR IGNORE mcp_servers SET name = substr(name, 6) WHERE name LIKE 'mind.%';
DELETE FROM mcp_servers WHERE name LIKE 'mind.%';

-- 3. Re-attach the grants under their bare server ids, skipping any that would
--    duplicate an already-bare grant for the same (entry_type, agent_id, tool).
INSERT INTO mcp_access_control
      (entry_type, agent_id, server_id, tool_name, permission,
       granted_by, granted_at, expires_at, justification, metadata)
  SELECT g.entry_type, g.agent_id, g.server_id, g.tool_name, g.permission,
         g.granted_by, g.granted_at, g.expires_at, g.justification, g.metadata
    FROM _mind_prefix_grants g
   WHERE NOT EXISTS (
       SELECT 1 FROM mcp_access_control b
        WHERE b.server_id  = g.server_id
          AND b.entry_type = g.entry_type
          AND b.agent_id   = g.agent_id
          AND IFNULL(b.tool_name, '') = IFNULL(g.tool_name, '')
   );
DROP TABLE _mind_prefix_grants;

-- 4. De-prefix the agent's default engine (plain column, no FK).
UPDATE agents
   SET default_engine_id = substr(default_engine_id, 6)
 WHERE default_engine_id LIKE 'mind.%';
