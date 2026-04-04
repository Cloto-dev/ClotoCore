# Security Hardening Roadmap

Based on the 15-layer security audit (2026-04-04).
Source of truth: `docs/SECURITY_LAYER_AUDIT_20260404.md`

## Current State: 15 layers, 11 fully enforced, 2 soft, 2 Phase 1

---

## CRITICAL (Security model fundamentals)

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| 4 | Process isolation | Phase 1 env-based | Phase 2: cgroups, seccomp, iptables (Linux) | ✅ Non-breaking | Large |
| 15 | Network scope | ProxyOnly=env vars; None=nothing | Block sockets via seccomp/iptables for None | ✅ Non-breaking | Large |
| 1 | Capability injection | Plugins can ignore env vars | OS-level network restriction (Phase 2 dep) | ✅ Non-breaking | Large (Phase 2) |

## HIGH (Practical gaps)

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| ~~11~~ | ~~Permission declarations~~ | ~~avatar/discord don't declare permissions_required~~ | ~~Add to Rust MGP server initialize response~~ | ~~✅ Non-breaking~~ | ~~Small~~ |
| 13 | Magic Seal | HMAC-SHA256 (shared key) | Ed25519 asymmetric signatures | ❌ MGP breaking (migration) | Medium |
| 10 | Code safety validation | Pattern-based | AST analysis (tree-sitter) or runtime sandbox | ⚠️ MCP potentially breaking | Large |
| 13 | Magic Seal | Core/Standard can be unsigned | Require signatures for all trust levels | ❌ MGP breaking (existing unsigned blocked) | Small |

## MEDIUM (Robustness improvements)

| # | Layer | Current | Hardening | MCP/MGP Compat | Effort |
|---|-------|---------|-----------|----------------|--------|
| 7 | API key auth | SHA-256 fingerprint | Argon2id (brute-force resistance) | ✅ Non-breaking | Small |
| ~~3~~ | ~~HITL~~ | ~~YOLO auto-approves everything~~ | ~~Force approval for filesystem.write, network.outbound even in YOLO~~ | ~~✅ Non-breaking~~ | ~~Small~~ |
| 2 | RBAC | Kernel-native tools bypass RBAC | Apply RBAC to mgp.*/gui.* tools | ✅ Non-breaking | Medium |
| ~~14~~ | ~~Trust levels~~ | ~~No warning on server self-declaration mismatch~~ | ~~Log warning when server declares higher trust than config allows~~ | ~~✅ Non-breaking~~ | ~~Small~~ |
| 5 | Audit log | SQLite file editable by root | Merkle chain checksums (OpenFang-style) | ✅ Non-breaking | Medium |
| ~~8~~ | ~~Host whitelist~~ | ~~Runtime add_host() unaudited~~ | ~~Audit log + HITL approval on host addition~~ | ~~✅ Non-breaking~~ | ~~Small~~ |
| ~~9~~ | ~~Event depth~~ | ~~Max configurable to 50~~ | ~~Lower cap to 25, warn at depth > 5~~ | ~~✅ Non-breaking~~ | ~~Trivial~~ |
| 12 | Tool security metadata | readOnlyHint is advisory only | HITL enforcement for destructiveHint=true tools (as MGP extension) | ⚠️ MGP extension needed | Small |
| ~~6~~ | ~~DNS rebinding~~ | ~~Private/loopback blocked~~ | ~~Add cloud metadata endpoints (169.254.169.254)~~ | ~~✅ Non-breaking~~ | ~~Trivial~~ |

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
| L7 | Deferred — API keys use 256-bit entropy (OsRng), making SHA-256 brute-force infeasible. Argon2id adds dependency + DB migration for marginal benefit. | N/A |
| L9 | MAX_EVENT_DEPTH cap lowered from 50 to 25; warning log added at depth > 5 | pending |
| L11 | `permissions_required: ["network.outbound"]` added to avatar and discord initialize responses | pending |
| L14 | Warning log when server self-declares higher trust level than config allows | pending |
| L8 | `tracing::warn!` added to `add_host()` for runtime whitelist additions | pending |
| L3 | YOLO exceptions: `filesystem.write` and `network.outbound` require approval even in YOLO mode (`CLOTO_YOLO_EXCEPTIONS` env var) | pending |

## Pre-HN Quick Wins (non-breaking, high impact)

1. ~~**Layer 11**: Add `permissions_required` to avatar/discord initialize — 30min~~ ✅
2. ~~**Layer 6**: Block 169.254.169.254 (cloud metadata SSRF) — already covered~~ ✅
3. **Layer 13**: Fix documentation: HMAC-SHA256, not Ed25519 — 5min
