-- Persist MGP trust_level per server so the kernel can derive the correct
-- isolation profile BEFORE the MGP handshake (which currently arrives after
-- the child process is spawned with HTTPS_PROXY injected). NULL = fall back
-- to the existing Standard default; 'core' / 'standard' / 'experimental' /
-- 'untrusted' are recognised downstream (see mcp_mgp.rs::TrustLevel).
ALTER TABLE mcp_servers ADD COLUMN trust_level TEXT;
