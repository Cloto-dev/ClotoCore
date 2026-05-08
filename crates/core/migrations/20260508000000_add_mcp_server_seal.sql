-- Add Magic Seal column to mcp_servers (MGP_ISOLATION_DESIGN.md §8 L0).
-- NULL ⇒ unsealed; the kernel forces effective trust_level to `untrusted`
-- on connect (MGP v0.6.3 §10 inv 3, see crates/core/src/managers/mcp.rs).
-- Existing rows stay NULL, so the v0.6.3 force-untrusted path activates
-- immediately after migration without operator action.
ALTER TABLE mcp_servers ADD COLUMN seal TEXT NULL;
