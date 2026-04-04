# ClotoCore Security Layer Audit — 2026-04-04

Code-level verification of all 15 claimed security layers.
Source of truth: codebase. Documentation treated as reference only.

## Summary

| # | Layer | Status | Enforcement | Bypass |
|---|-------|--------|-------------|--------|
| 1 | Capability injection | **VERIFIED** | Command whitelist enforced; network/FS soft isolation (Phase 1) | Plugin could ignore env vars via raw syscalls |
| 2 | 3-level RBAC | **VERIFIED** | All tool execution gated; 3 priority levels checked | Kernel-native tools YOLO-gated, not bypassed |
| 3 | Human-in-the-loop | **VERIFIED** | Server connection blocked until approval | YOLO auto-approves but creates audit trail |
| 4 | Process isolation | **PARTIAL** | Separate processes, sandbox dirs, env redirection | Phase 2 OS-level (cgroups, seccomp) not implemented |
| 5 | Append-only audit log | **VERIFIED** | INSERT-only, no DELETE/UPDATE API, AUTOINCREMENT | None found |
| 6 | DNS rebinding protection | **VERIFIED** | IP validation before HTTP request | None found |
| 7 | API key auth + rate limiting | **VERIFIED** | Constant-time comparison, token bucket per-IP | None found |
| 8 | Host whitelisting | **VERIFIED** | Checked before DNS resolution, global scope | None found |
| 9 | Event depth limiting | **VERIFIED** | Events dropped at max depth (default 10) | None found |
| 10 | Code safety validation | **VERIFIED** | Pattern-based blocks at registration + execution | Sophisticated obfuscation could bypass |
| 11 | Permission declarations | **VERIFIED** | DB checks before tool execution, audit-logged | YOLO auto-approves |
| 12 | Tool security metadata | **VERIFIED** | Computed, attached to schema, used in validation | None found |
| 13 | Magic Seal | **VERIFIED** | HMAC-SHA256, blocks invalid/unsigned (Untrusted) | Core/Standard can be unsigned |
| 14 | Trust levels | **VERIFIED** | Kernel-authoritative, affects isolation/seal/risk | Config override possible |
| 15 | Network scope | **PARTIAL** | Phase 1 env vars only; Phase 2 OS-level pending | Plugin can ignore env vars |

## Verdict

- **VERIFIED (fully enforced)**: 11 layers (2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 14)
- **VERIFIED (functional, soft enforcement)**: 2 layers (1, 13)
- **PARTIAL (Phase 1 only)**: 2 layers (4, 15)

No security layer is a stub or missing. All 15 have working code.
Phase 2 OS-level enforcement (cgroups, seccomp, iptables) is designed but deferred.

## Corrections from Initial Claims

| Claim | Reality |
|-------|---------|
| Magic Seal uses Ed25519 | **HMAC-SHA256** with constant-time comparison |
| API key uses Argon2id | **SHA-256** with fixed salt (appropriate for API key fingerprinting, not password hashing) |
| OS-level isolation enforced | **Phase 1 only** — environment-based soft isolation |
| MGP servers declare permissions | **Config-driven primarily** — Rust MGP servers (avatar, discord) don't declare `permissions_required` in initialize response |

## Detailed Findings

### Kernel-Side (Layers 1-10)

#### Layer 1: Capability Injection
- **Command whitelist**: `mcp_transport.rs:46,91-113` — only `python`, `node`, `npx`, `deno`, `bun` + workspace `mgp-*` binaries
- **Network isolation**: `mcp_transport.rs:209-217` — HTTP_PROXY/HTTPS_PROXY injected for ProxyOnly
- **FS isolation**: `mcp_transport.rs:191-206` — HOME/TMPDIR redirected to sandbox
- **HTTP URL validation**: `mcp_transport.rs:496-519` — HTTPS required for non-localhost

#### Layer 2: 3-Level RBAC
- **Tool-level**: `db/mcp.rs:354-414` — `resolve_tool_access()` with expiration
- **Server-level**: implicit via tool discovery filtering
- **Enforcement point**: `registry.rs:326-344` — checked BEFORE execution, returns error on deny
- **Kernel tools**: YOLO-gated (`create_mcp_server`, `ask_agent`, `audit.replay`)

#### Layer 3: Human-in-the-Loop
- **Permission requests**: `mcp.rs:572-606` — pending PermissionRequest created in DB
- **Blocking**: `mcp.rs:626-632` — server connection BLOCKED until approved
- **Approval**: `db/permissions.rs:126-164` — status transition with double-approval prevention
- **YOLO mode**: auto-approves with `YOLO_APPROVER_ID`, still creates audit records

#### Layer 4: Process Isolation
- **Separate processes**: all MCP servers are child processes via `Command::new()`
- **Sandbox directory**: `mcp_transport.rs:192` — created per-server
- **Phase 1**: env-based (HOME, TMPDIR, cwd redirection)
- **Phase 2**: designed in `mcp_isolation.rs:127-154` (memory_limit_mb, max_child_processes) but NOT enforced

#### Layer 5: Audit Log
- **INSERT-only**: `db/audit.rs:34` — no DELETE/UPDATE functions exist
- **AUTOINCREMENT**: ensures sequential, tamper-evident IDs
- **Retry**: 3 attempts with backoff (`audit.rs:55-75`)
- **Events logged**: TOOL_EXECUTED, TOOL_BLOCKED, permission grants/revocations

#### Layer 6: DNS Rebinding Protection
- **IP validation**: `capabilities.rs:39-57` — blocks private, loopback, link-local, multicast
- **Checked before request**: `capabilities.rs:104-124` — `lookup_host()` + IP validation
- **Comment**: explicitly marked "DNS Rebinding対策"

#### Layer 7: API Key Auth + Rate Limiting
- **Auth**: `handlers.rs:83-139` — `X-API-Key` header, constant-time comparison (`subtle::ConstantTimeEq`)
- **Revocation**: SHA-256 fingerprint, in-memory cache
- **Rate limiter**: `middleware.rs:1-180` — `governor` crate token bucket, per-IP, configurable (10/s, burst 20)

#### Layer 8: Host Whitelisting
- **SafeHttpClient**: `capabilities.rs:13-79` — `Arc<RwLock<HashSet<String>>>`
- **Enforcement**: checked BEFORE DNS resolution (`capabilities.rs:91`)
- **Scope**: global, all plugin outgoing requests filtered
- **Default hosts**: `api.deepseek.com`, `api.cerebras.ai`, `api.openai.com`, `api.anthropic.com`

#### Layer 9: Event Depth Limiting
- **EnvelopedEvent.depth**: `lib.rs:54` — u8 field, incremented on each cascade
- **Enforcement**: `registry.rs:361-372` — drops events at max depth
- **Config**: `MAX_EVENT_DEPTH` default 10, range 1-50

#### Layer 10: Code Safety Validation
- **Tool arguments**: `mcp_tool_validator.rs:43-128` — shell metachar blocking, dangerous pattern detection
- **MCP code**: `mcp_tool_validator.rs:212-285` — blocked imports/patterns, safety levels (Unrestricted/Standard/Strict/Readonly)
- **When**: at registration AND before execution
- **Limitation**: pattern-based, not semantic analysis

### MGP Protocol-Side (Layers 11-15)

#### Layer 11: Permission Declarations
- **Config-level**: `mcp.rs:534-627` — `required_permissions` from mcp.toml
- **Server-level**: `mcp.rs:823-935` — `permissions_required` from MGP initialize response
- **Enforcement**: DB lookup before tool execution, pending = blocked
- **Note**: Rust MGP servers (avatar, discord) do NOT currently declare permissions in initialize

#### Layer 12: Tool Security Metadata
- **Computation**: `mcp.rs:1260-1282` — derived from trust level + validator + permissions
- **Attached to schema**: `mcp_types.rs:94-107` — serialized in tool JSON before LLM exposure
- **Used in validation**: `mcp.rs:1557-1574` — validator checked before tool execution

#### Layer 13: Magic Seal
- **Algorithm**: HMAC-SHA256 (NOT Ed25519 as previously claimed)
- **Constant-time**: `subtle::ConstantTimeEq` (`mcp_seal.rs:71`)
- **Key management**: env var → file → auto-generate (`mcp_seal.rs:126-167`)
- **Enforcement matrix**: Untrusted without seal = blocked; invalid seal = always blocked

#### Layer 14: Trust Levels
- **Enum**: Core > Standard > Experimental > Untrusted (`mcp_mgp.rs:360-385`)
- **Kernel-authoritative**: config > server declaration > default Untrusted (`mcp_mgp.rs:478-514`)
- **Affects**: seal checks, isolation profile, risk level, override permissions

#### Layer 15: Network Scope
- **Enum**: Unrestricted / ProxyOnly / None (`mcp_isolation.rs:55-84`)
- **Phase 1**: env vars (HTTP_PROXY, HTTPS_PROXY) for ProxyOnly (`mcp_transport.rs:209-217`)
- **Phase 2**: OS-level (cgroups, seccomp, netfilter) — NOT implemented
- **Bypass risk**: plugin can ignore env vars and make direct network calls
