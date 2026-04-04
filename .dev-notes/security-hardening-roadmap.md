# Security Hardening Roadmap

Based on the 15-layer security audit (2026-04-04).
Source of truth: `docs/SECURITY_LAYER_AUDIT_20260404.md`

## Current State: 15 layers, 11 fully enforced, 2 soft, 2 Phase 1

All non-breaking hardening items completed (2026-04-04).
Remaining items require OS-level infrastructure (Phase 2) or protocol-breaking changes.

---

## CRITICAL (Security model fundamentals) — Phase 2

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| 4 | Process isolation | Phase 1 env-based | Phase 2: cgroups, seccomp, iptables (Linux) | ✅ Non-breaking | Large |
| 15 | Network scope | ProxyOnly=env vars; None=nothing | Block sockets via seccomp/iptables for None | ✅ Non-breaking | Large |
| 1 | Capability injection | Plugins can ignore env vars | OS-level network restriction (Phase 2 dep) | ✅ Non-breaking | Large (Phase 2) |

## HIGH (Protocol-breaking changes) — Post-1.0

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| 13 | Magic Seal | HMAC-SHA256 (shared key) | Ed25519 asymmetric signatures | ❌ MGP breaking (migration) | Medium |
| 10 | Code safety validation | Pattern-based | AST analysis (tree-sitter) or runtime sandbox | ⚠️ MCP potentially breaking | Large |
| 13 | Magic Seal | Core/Standard can be unsigned | Require signatures for all trust levels | ❌ MGP breaking (existing unsigned blocked) | Small |

## MEDIUM — Deferred

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort | Status |
|---|-------|---------|-----------|----------------|--------|--------|
| 7 | API key auth | SHA-256 fingerprint | Argon2id (brute-force resistance) | ✅ Non-breaking | Small | Deferred — 256-bit key entropy makes brute-force infeasible |

## LOW (Future improvements)

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| 15 | Network scope | 3 levels (Unrestricted/ProxyOnly/None) | Per-server domain allowlist | ✅ Non-breaking | Medium |
| 1 | Capability injection | Global host whitelist | Per-server capability profiles | ✅ Non-breaking | Medium |

---

## Compatibility Summary

| Category | Count | Items |
|----------|-------|-------|
| Fully non-breaking | 12 | Phase 2 OS isolation, Argon2id, YOLO partial, kernel RBAC, trust warning, Merkle audit, host audit, depth cap, metadata HITL, cloud metadata, per-server domain, per-server capability |
| MGP breaking (migration required) | 2 | Magic Seal Ed25519, mandatory signatures |
| MCP potentially breaking | 1 | AST analysis (stricter validation may reject existing servers) |

## Completed Items (2026-04-04)

| Layer | Change | Commit |
|-------|--------|--------|
| L6 | Cloud metadata (169.254.169.254) already blocked by `is_link_local()` — no code change needed | N/A |
| L7 | Deferred — API keys use 256-bit entropy (OsRng), making SHA-256 brute-force infeasible | N/A |
| L9 | MAX_EVENT_DEPTH cap lowered from 50 to 25; warning log added at depth > 5 | `45982e7` |
| L14 | Warning log when server self-declares higher trust level than config allows | `00f224c` |
| L8 | `tracing::warn!` added to `add_host()` for runtime whitelist additions | `00bca4a` |
| L3 | YOLO exceptions: `filesystem.write` and `network.outbound` require approval even in YOLO mode (`CLOTO_YOLO_EXCEPTIONS` env var) | `e2d1c94` |
| L11 | `permissions_required: ["network.outbound"]` added to avatar and discord initialize responses | `3624990` (cloto-mcp-servers) |
| L13 | Documentation already correct (HMAC-SHA256). Ed25519 migration documented in PROJECT_VISION §10. | `288bede` |
| L5 | Merkle chain hash on audit log entries (SHA-256 chain, tamper detection) | `e7f974d` |
| L12 | destructiveHint HITL gate — parse MCP annotations, require approval for destructive tools | `1a28bb7` |
| L2 | Kernel tool RBAC — `mgp.*`/`gui.*` tools now checked via `resolve_tool_access(server_id="kernel")` | `17ff35b` |

## Pre-HN Quick Wins

1. ~~**Layer 11**: Add `permissions_required` to avatar/discord initialize~~ ✅
2. ~~**Layer 6**: Block 169.254.169.254 (cloud metadata SSRF) — already covered~~ ✅
3. ~~**Layer 13**: Fix documentation — already correct, Ed25519 migration planned~~ ✅
