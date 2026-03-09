-- Fix: Ensure MCP server records exist before FK-dependent migrations.
--
-- Root cause: Migration 20260304200000 used INSERT OR IGNORE without
-- specifying granted_at (a NOT NULL column with no DEFAULT), causing all
-- MCP grant rows to be silently dropped. This broke the FK chain for
-- migration 20260309000000_rename_ks22_to_cpersona which expects
-- memory.ks22 to exist in mcp_servers.
--
-- This migration runs between 20260307 and 20260309 to restore the
-- prerequisite state.

-- Conditionally insert memory.ks22 only if it hasn't already been renamed
-- to memory.cpersona (handles both fresh DBs and existing DBs).
INSERT OR IGNORE INTO mcp_servers (name, command, args, created_at)
SELECT 'memory.ks22', 'config-loaded', '[]', strftime('%s', 'now')
WHERE NOT EXISTS (SELECT 1 FROM mcp_servers WHERE name = 'memory.cpersona');
