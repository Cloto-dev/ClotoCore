-- Capability gate (bug-421): make per-agent MCP access deterministic.
--
-- Two coupled changes ship together with the code-level single capability gate
-- (managers/mcp.rs enforce_caller_grant / enforce_kernel_rbac):
--
--   1. Flip every server back to opt-in (deny-by-default). Under the prior
--      opt-out default, revoking a grant (deleting the row) fell back to allow,
--      and an agent that was never granted an engine still ran it. opt-in makes
--      both cases deny correctly: grant => allow, no grant => deny, revoke =
--      delete => deny.
--   2. Backfill each agent's reasoning engine as an explicit server_grant, so
--      agents that relied on the implicit opt-out allow-all for their engine
--      keep working — now with an explicit, auditable, revocable grant
--      (implicit -> explicit). Agents with no engine and no grant (the bug being
--      fixed) correctly lose access.

-- 1. Global opt-in flip. Does NOT touch the 'kernel' row (already opt-in): its
--    default is special-cased to Allow in code (Deny-only RBAC), so kernel-native
--    tools are not turned into deny-by-default by this flip.
UPDATE mcp_servers SET default_policy = 'opt-in' WHERE default_policy = 'opt-out';

-- 2. Engine backfill — FK-safe, non-clobbering, and PREFIX-AWARE.
--    The runtime engine resolver (handlers/system.rs, bug-396) strips a leading
--    'mind.' from default_engine_id when only the de-prefixed MCP server is
--    registered, and the capability gate then checks that de-prefixed server_id.
--    So the grant must target the server name the runtime will ACTUALLY use:
--    prefer default_engine_id when a server row matches it, else its de-prefixed
--    ('mind.' stripped) form. The CASE only ever yields a name that EXISTS in
--    mcp_servers (guaranteed by the WHERE clause), so the FK (foreign_keys=ON)
--    is satisfied and the migration cannot abort; granted_at is NOT NULL and is
--    supplied; the NOT EXISTS guard (matching the resolved name, allow OR deny)
--    never clobbers an existing grant.
INSERT INTO mcp_access_control (entry_type, agent_id, server_id, permission, granted_by, granted_at)
SELECT
    'server_grant',
    a.id,
    CASE
        WHEN EXISTS (SELECT 1 FROM mcp_servers s WHERE s.name = a.default_engine_id)
            THEN a.default_engine_id
        ELSE substr(a.default_engine_id, 6)   -- strip leading 'mind.' (5 chars)
    END,
    'allow',
    'migration:capability-gate',
    datetime('now')
FROM agents a
WHERE a.default_engine_id IS NOT NULL
  AND a.default_engine_id != ''
  AND (
        EXISTS (SELECT 1 FROM mcp_servers s WHERE s.name = a.default_engine_id)
        OR (
            a.default_engine_id LIKE 'mind.%'
            AND EXISTS (SELECT 1 FROM mcp_servers s WHERE s.name = substr(a.default_engine_id, 6))
        )
      )
  AND NOT EXISTS (
        SELECT 1 FROM mcp_access_control ac
        WHERE ac.agent_id = a.id
          AND ac.server_id = CASE
                WHEN EXISTS (SELECT 1 FROM mcp_servers s WHERE s.name = a.default_engine_id)
                    THEN a.default_engine_id
                ELSE substr(a.default_engine_id, 6)
              END
          AND ac.entry_type = 'server_grant'
          AND ac.tool_name IS NULL
      );
