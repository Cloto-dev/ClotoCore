import type { McpServerInfo } from '../types';

/**
 * MGP server identification (single source of truth, replaces inline duplicates
 * in `ServerAccessSection.tsx` and `McpServersPage.tsx`).
 *
 * A server is rendered as MGP (purple card branding) when *either*:
 *   (a) `mgp_supported` — kernel handshake negotiated `capabilities.mgp`
 *       (protocol-level MGP, per MGP_SPEC §2.3), or
 *   (b) `marketplace_id` is set — the server was installed via the ClotoHub
 *       catalog (`POST /api/marketplace/install`), regardless of whether the
 *       seal was present (per MGP §10 invariant 3: seal absence demotes
 *       `trust_level` to `untrusted` but does not revoke MGP membership).
 *
 * Runtime connection state (`status`) is intentionally NOT gated — MGP
 * membership is an install-time property, not a runtime attribute. A disconnected
 * MGP server still renders purple.
 *
 * Legacy `id.startsWith('io.')` / `id.startsWith('output.')` hardcoding was
 * removed: `io.discord` and `output.avatar` both declare `capabilities.mgp`
 * v0.6.0 since the 2026-04-22 PR, so handshake negotiation covers them.
 * Hardcoded ID prefixes violated ARCHITECTURE.md §1.2 ("Not who it is, but
 * what it can do").
 */
export function isMgpServer(server: McpServerInfo): boolean {
  return Boolean(server.mgp_supported) || Boolean(server.marketplace_id);
}
