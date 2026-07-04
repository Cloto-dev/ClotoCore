-- Retire the remaining category prefixes (Goal #143 / Task #153):
-- `tool.` / `vision.` / `voice.` / `io.` / `output.`.
--
-- Completes the programme started by 20260701120000_retire_mind_prefix.sql
-- (Goal #142): every server is identified by a single canonical bare id equal
-- to its `servers/<id>` directory / hub connector id
-- (docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md). One explicit mapping departs
-- from the mechanical first-segment strip: `vision.gaze_webcam` -> `gaze`
-- (the strip would mint a fifth spelling of that server's name; the canonical
-- id follows the install directory, design doc S4).
--
-- Same detach -> rename -> re-attach shape as the mind migration: the
-- mcp_access_control.server_id FK -> mcp_servers(name) (ON DELETE CASCADE)
-- blocks a naive parent rename while children reference it, so grants are
-- detached, servers renamed, grants re-inserted under bare ids. FK-satisfied
-- at every statement boundary; correct in both transaction and autocommit
-- execution; no reliance on `defer_foreign_keys`.
--
-- Idempotent: every clause targets only prefixed rows, so a re-run (or a DB
-- that never had them) is a zero-row no-op. Collision-safe: real databases
-- hold bare twins (`terminal` / `websearch` / `cscheduler` installed through
-- the hub next to seeded `tool.*` rows) — the bare row wins and the prefixed
-- row/grant merges into it. Fresh-DB tolerant. This migration only
-- renames/merges specific rows — it never deletes a `.db` or truncates a
-- table. The historical seed migrations that wrote `tool.*` ids
-- (20260304200000, 20260304200001, 20260309100000) are frozen checksum
-- history; this migration renames what they seeded.

-- 1. Detach every category-prefixed grant (FK child) into a temp table under
--    its bare id, then remove the originals so the parent rows can be renamed
--    without the server_id FK blocking the rename.
CREATE TEMP TABLE _cat_prefix_grants AS
  SELECT entry_type, agent_id,
         CASE WHEN server_id = 'vision.gaze_webcam' THEN 'gaze'
              ELSE substr(server_id, instr(server_id, '.') + 1)
         END AS server_id,
         tool_name, permission, granted_by, granted_at, expires_at,
         justification, metadata
    FROM mcp_access_control
   WHERE server_id LIKE 'tool.%'
      OR server_id LIKE 'vision.%'
      OR server_id LIKE 'voice.%'
      OR server_id LIKE 'io.%'
      OR server_id LIKE 'output.%';
DELETE FROM mcp_access_control
 WHERE server_id LIKE 'tool.%'
    OR server_id LIKE 'vision.%'
    OR server_id LIKE 'voice.%'
    OR server_id LIKE 'io.%'
    OR server_id LIKE 'output.%';

-- 2. Rename the server rows to bare. The explicit gaze mapping runs first so
--    the generic strip below never sees it. mcp_servers.name is PRIMARY KEY,
--    so OR IGNORE keeps an existing bare twin; the prefixed row left behind
--    on a collision is dropped. No grant references a prefixed name now, so
--    ON DELETE CASCADE removes nothing.
UPDATE OR IGNORE mcp_servers SET name = 'gaze' WHERE name = 'vision.gaze_webcam';
UPDATE OR IGNORE mcp_servers
   SET name = substr(name, instr(name, '.') + 1)
 WHERE name LIKE 'tool.%'
    OR name LIKE 'vision.%'
    OR name LIKE 'voice.%'
    OR name LIKE 'io.%'
    OR name LIKE 'output.%';
DELETE FROM mcp_servers
 WHERE name LIKE 'tool.%'
    OR name LIKE 'vision.%'
    OR name LIKE 'voice.%'
    OR name LIKE 'io.%'
    OR name LIKE 'output.%';

-- 3. Re-attach the grants under their bare server ids, skipping any that
--    would duplicate an already-bare grant for the same
--    (entry_type, agent_id, tool).
INSERT INTO mcp_access_control
      (entry_type, agent_id, server_id, tool_name, permission,
       granted_by, granted_at, expires_at, justification, metadata)
  SELECT g.entry_type, g.agent_id, g.server_id, g.tool_name, g.permission,
         g.granted_by, g.granted_at, g.expires_at, g.justification, g.metadata
    FROM _cat_prefix_grants g
   WHERE NOT EXISTS (
       SELECT 1 FROM mcp_access_control b
        WHERE b.server_id  = g.server_id
          AND b.entry_type = g.entry_type
          AND b.agent_id   = g.agent_id
          AND IFNULL(b.tool_name, '') = IFNULL(g.tool_name, '')
   );
DROP TABLE _cat_prefix_grants;

-- 4. Residue sweep for Goal #142: cron_jobs.engine_id stores engine ids but
--    was not covered by the mind migration (which only renamed
--    agents.default_engine_id), so a `mind.*` value could still linger here.
--    Plain column, no FK.
UPDATE cron_jobs
   SET engine_id = substr(engine_id, 6)
 WHERE engine_id LIKE 'mind.%';
