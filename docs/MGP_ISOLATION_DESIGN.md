# MGP-Enforced Isolation Model

**Version:** 0.1.0-draft
**Status:** Approved Design
**Date:** 2026-03-05
**Prerequisite:** MGP_SPEC.md v0.5.3+, ARCHITECTURE.md

---

## 1. Problem Statement

### 1.1 Context

ClotoCore targets a **multi-tenant model** where third-party MCP/MGP servers connect
to the kernel alongside first-party servers. MGP (§2-7) provides a comprehensive
**policy layer** — permission declarations, tool security metadata, access control,
and audit trails — but policy alone is insufficient against adversarial actors.

### 1.2 The Gentleman's Agreement Problem

MGP §4 allows servers to declare `side_effects: ["filesystem", "process"]` and
`risk_level: "dangerous"`, but nothing prevents a malicious server from:

- Ignoring its own declarations and accessing arbitrary files
- Consuming unbounded CPU/memory to starve the kernel
- Opening raw sockets for data exfiltration or C2 communication
- Spawning child processes (cryptominers, reverse shells)

MGP's security metadata is a **gentleman's agreement**: effective against bugs in
honest servers, useless against adversarial ones.

### 1.3 The Requirement

> MGP defines **what is permitted**.
> OS isolation ensures **what is not permitted is physically impossible**.

These two layers are orthogonal and must be combined. Neither alone is sufficient
for a multi-tenant platform accepting untrusted third-party servers.

---

## 2. Architecture Overview

### 2.1 Two-Layer Security Model

```
┌─────────────────────────────────────────────────────────┐
│  Policy Layer (MGP Protocol)                            │
│                                                         │
│  ┌─────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐ │
│  │ §2 Caps │  │ §3 Perms │  │ §4 Tools │  │ §5 RBAC │ │
│  │ Negot.  │  │ Declare  │  │ Metadata │  │ 3-Level │ │
│  └────┬────┘  └────┬─────┘  └────┬─────┘  └────┬────┘ │
│       │            │             │              │       │
│       └────────────┴──────┬──────┴──────────────┘       │
│                           │                             │
│                   Isolation Profile                     │
│                    (auto-derived)                       │
│                           │                             │
├───────────────────────────┼─────────────────────────────┤
│  Enforcement Layer (OS)   │                             │
│                           ▼                             │
│  ┌─────────┐  ┌──────────┐  ┌──────────┐  ┌─────────┐ │
│  │ L0 Seal │  │ L1 Rsrc  │  │ L2 FS    │  │ L3 Net  │ │
│  │ HMAC    │  │ Limits   │  │ Sandbox  │  │ Control │ │
│  └─────────┘  └──────────┘  └──────────┘  └─────────┘ │
│                                            ┌─────────┐ │
│                                            │ L4 Proc │ │
│                                            │ Restrict│ │
│                                            └─────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 2.2 Data Flow

```
1. Server binary / script exists on disk
       │
2. [L0] Magic Seal verification (HMAC-SHA256)
       │ FAIL → reject, log SEAL_VERIFICATION_FAILED
       ▼
3. [MGP §2] Capability negotiation (initialize handshake)
       │ Server declares: permissions_required, trust_level, extensions
       ▼
4. [MGP §3] Permission approval (interactive / auto / deny)
       │ DENIED → reject connection
       ▼
5. Kernel derives Isolation Profile from:
       │  - trust_level (§2.3)
       │  - permissions_required (§3)
       │  - [servers.isolation] overrides (mcp.toml)
       ▼
6. [L1-L4] OS isolation applied to spawned process
       │  - Resource limits (L1)
       │  - Filesystem scope (L2)
       │  - Network restrictions (L3)
       │  - Process spawn limits (L4)
       ▼
7. Server operates within enforced boundaries
       │
8. [MGP §4] Kernel validates tool calls BEFORE forwarding
       │  (Defense-in-Depth: kernel-side validators)
       ▼
9. [MGP §6] All actions audit-logged with trace_id
```

### 2.3 Design Principles

1. **MGP Drives OS** — Isolation profiles are derived from MGP metadata, not
   configured independently. `trust_level` alone determines default restrictions.
2. **Fail-Closed** — If isolation profile generation fails, the server is not started.
3. **Cross-Platform Degradation** — OS mechanisms vary by platform. The kernel uses
   the strongest available mechanism, never skips enforcement entirely.
4. **No Docker Dependency** — All isolation uses OS-native primitives. No container
   runtime required. This preserves the personal AI OS deployment model.
5. **Override Capability** — `[servers.isolation]` in mcp.toml can relax or tighten
   defaults per server. Only `core` trust_level servers may override to `unrestricted`.

---

## 3. Trust Levels and Isolation Profiles

### 3.1 Trust Level Definitions

MGP §2 defines `trust_level` in the server handshake response. This document extends
its semantics to OS-level enforcement:

| trust_level | Description | Who | Magic Seal |
|---|---|---|---|
| `core` | Kernel-integral servers. Full system access. | First-party only | Required (signed by project key) |
| `standard` | General-purpose servers. Sandboxed with reasonable limits. | First-party or vetted third-party | Required |
| `experimental` | Untested or in-development servers. Strict limits. | Any (with approval) | Optional (warn if missing) |
| `untrusted` | Unknown origin. Maximum restriction. | Any | Not required (assumed adversarial) |

### 3.2 Default Isolation Profile Matrix

| Parameter | `core` | `standard` | `experimental` | `untrusted` |
|---|---|---|---|---|
| **L1: Memory limit** | Unrestricted | 512 MB | 256 MB | 128 MB |
| **L1: CPU time / call** | Unrestricted | 60 s | 30 s | 10 s |
| **L2: Filesystem** | Unrestricted | `sandbox` | `readonly` | `none` |
| **L3: Network** | Unrestricted | `proxy_only` | `none` | `none` |
| **L4: Max child procs** | Unrestricted | 1 | 0 | 0 |
| **Tool validators** | Optional | Applied | Enforced (strict) | Enforced (strict) |
| **Audit level** | Standard | Standard | Verbose | Verbose |

### 3.3 Filesystem Scope Values

| Value | Behavior |
|---|---|
| `unrestricted` | No filesystem restrictions. Only for `core` servers. |
| `sandbox` | Read/write limited to `data/mcp-sandbox/{server_id}/`. Read-only access to server's own script directory. |
| `readonly` | Read-only access to sandbox directory. No write anywhere. |
| `none` | No filesystem access. All file operations blocked. |

### 3.4 Network Scope Values

| Value | Behavior |
|---|---|
| `unrestricted` | No network restrictions. Only for `core` servers. |
| `proxy_only` | Outbound HTTP only via LLM Proxy (`127.0.0.1:{llm_proxy_port}`). Raw sockets blocked. |
| `allowlist` | Outbound HTTP to explicitly listed hosts only (via SafeHttpClient whitelist). |
| `none` | No network access. All outbound connections blocked. |

### 3.5 Permission-Driven Overrides

When a server declares `permissions_required` (MGP §3), the kernel adjusts the
isolation profile accordingly:

```
permissions_required: ["network.outbound"]
  → L3 upgraded from "none" to "proxy_only" (if trust_level allows)

permissions_required: ["filesystem.write"]
  → L2 upgraded from "readonly" to "sandbox" (if trust_level allows)

permissions_required: ["shell.execute"]
  → L4 upgraded from 0 to 1 (if trust_level allows)
```

The upgrade is capped by trust_level. An `untrusted` server declaring
`network.outbound` will NOT receive network access — the permission is denied
at §3 approval stage.

---

## 4. Layer Specifications

### 4.0 Layer 0: Magic Seal (Binary Trust)

**Purpose:** Verify that the server binary/script has not been tampered with before
any protocol interaction occurs.

**Mechanism:**
- HMAC-SHA256 signature over server entry point file
- Signature stored in `mcp.toml` or separate `.seal` file
- Kernel verifies signature at spawn time, before `initialize` handshake

**Configuration:**
```toml
[[servers]]
id = "mind.cerebras"
command = "python"
args = ["mcp-servers/cerebras/server.py"]
seal = "sha256:a1b2c3d4e5f6..."   # HMAC of server.py

[servers.mgp]
trust_level = "standard"
```

**Behavior by trust_level:**

| trust_level | Seal missing | Seal invalid |
|---|---|---|
| `core` | Block startup | Block startup |
| `standard` | Block startup | Block startup |
| `experimental` | Warn, allow | Block startup |
| `untrusted` | Allow (assumed adversarial) | Block startup |

**Development mode:** `CLOTO_ALLOW_UNSIGNED=true` bypasses seal verification for all
trust levels. A prominent warning is logged.

### 4.1 Layer 1: Resource Limits

**Purpose:** Prevent resource exhaustion attacks (infinite loops, memory bombs, fork
bombs) from affecting kernel stability.

**Implementation:**

| Platform | Mechanism | Granularity |
|---|---|---|
| Linux | `setrlimit(2)` via `Command::pre_exec()` | Per-process |
| Linux (future) | cgroups v2 | Per-process group |
| Windows | Job Objects (`CreateJobObjectW` + `AssignProcessToJobObject`) | Per-job (includes children) |
| macOS | `setrlimit(2)` via `Command::pre_exec()` | Per-process |

**Enforced limits:**

| Resource | API | Notes |
|---|---|---|
| Virtual memory | `RLIMIT_AS` / `JOB_OBJECT_LIMIT_PROCESS_MEMORY` | Hard cap, OOM on exceed |
| CPU time | `RLIMIT_CPU` / `JOB_OBJECT_LIMIT_PROCESS_TIME` | SIGXCPU (Linux) / termination (Windows) |
| Open files | `RLIMIT_NOFILE` / inherits from kernel | Prevents fd exhaustion |
| Child processes | `RLIMIT_NPROC` / `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` | Fork bomb prevention |

**MGP Error Integration:**

When a resource limit is hit, the kernel:
1. Detects process termination or error
2. Logs `ISOLATION_VIOLATION` audit event (§6)
3. Returns MGP error to the calling agent:

```json
{
  "error": {
    "code": 3001,
    "message": "MCP server exceeded memory limit (512 MB)",
    "data": {
      "_mgp": {
        "category": "resource",
        "retryable": false,
        "details": {
          "server_id": "tool.terminal",
          "resource": "memory",
          "limit_mb": 512,
          "isolation_layer": "L1"
        }
      }
    }
  }
}
```

### 4.2 Layer 2: Filesystem Isolation

**Purpose:** Prevent servers from reading sensitive files (`.env`, SSH keys, database)
or writing outside their sandbox.

**Implementation:**

| Platform | Mechanism | Scope |
|---|---|---|
| Linux | Mount namespace (unprivileged userns) or `chroot` | Hard isolation |
| Linux (fallback) | `chdir` + `RLIMIT_FSIZE` | Soft isolation |
| Windows | Restricted Token + directory ACL | Medium isolation |
| macOS | `sandbox-exec` profile | Hard isolation |
| All (minimum) | `chdir` to sandbox + env var guidance | Soft isolation (Phase 1) |

**Sandbox directory structure:**
```
data/mcp-sandbox/
  ├── tool.terminal/       # Read-write for tool.terminal
  │   ├── workspace/       # Working directory
  │   └── tmp/             # Temporary files
  ├── mind.cerebras/       # Read-write for mind.cerebras
  └── ...
```

**Phase 1 (soft):** `chdir` to sandbox directory, set `HOME` and `TMPDIR` env vars.
Server can still escape via absolute paths, but accidental file access is prevented.

**Phase 2+ (hard):** Mount namespace or platform-specific mechanism prevents any
access outside the sandbox directory tree.

### 4.3 Layer 3: Network Control

**Purpose:** Prevent data exfiltration, C2 communication, and unauthorized API access.

**Implementation strategy:**

The kernel already provides an **LLM Proxy** (MGP §13.4, `managers/llm_proxy.rs`)
on `127.0.0.1:{llm_proxy_port}`. This proxy:
- Injects API keys (servers never see credentials)
- Routes to configured LLM providers
- Logs all requests

**`proxy_only` enforcement:**

| Platform | Mechanism |
|---|---|
| Linux | seccomp filter: allow `connect(2)` only to `127.0.0.1:{proxy_port}` |
| Linux (fallback) | Environment-only: set `HTTP_PROXY`, `HTTPS_PROXY`, unset API key env vars |
| Windows | Windows Filtering Platform (WFP) rules per-process, or env-only fallback |
| All (Phase 1) | Environment-only: `CLOTO_LLM_PROXY` env var, no raw credential env vars |

**`allowlist` enforcement:**

Uses the existing `SafeHttpClient` host whitelist (capabilities.rs). The allowed
hosts list is injected as `CLOTO_ALLOWED_HOSTS` env var. Hard enforcement requires
seccomp or WFP (Phase 3).

### 4.4 Layer 4: Process Spawn Restriction

**Purpose:** Prevent servers from launching arbitrary child processes (reverse shells,
cryptominers, privilege escalation).

**Implementation:**

| Platform | Mechanism |
|---|---|
| Linux | `prctl(PR_SET_NO_NEW_PRIVS, 1)` + seccomp (`execve` filtered) |
| Linux (soft) | `RLIMIT_NPROC = 1` (includes self) |
| Windows | Job Object `JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 1` |

**Note:** `max_child_processes = 0` means the server itself can run but cannot spawn
any children. `max_child_processes = 1` allows one child (needed for servers that
shell out to tools like `git` or `pip`).

---

## 5. Configuration

### 5.1 mcp.toml Extension

The `[servers.isolation]` section allows per-server override of the default
isolation profile derived from `trust_level`:

```toml
[[servers]]
id = "tool.terminal"
command = "python"
args = ["mcp-servers/terminal/server.py"]

[servers.mgp]
extensions = ["permissions", "tool_security", "lifecycle"]
permissions_required = ["shell.execute", "filesystem.write"]
trust_level = "standard"

# Optional: override default isolation for this trust_level
[servers.isolation]
memory_limit_mb = 1024        # Override: 1GB instead of default 512MB
cpu_time_limit_secs = 120     # Override: 2min instead of default 60s
filesystem_scope = "sandbox"  # Matches default for "standard"
network_scope = "proxy_only"  # Matches default for "standard"
max_child_processes = 3       # Override: needs to spawn git, pip, etc.
```

**Validation rules:**
- `core` servers: any value allowed
- `standard` servers: cannot set `unrestricted` for any parameter
- `experimental` servers: cannot exceed `standard` defaults
- `untrusted` servers: cannot override any parameter (always maximum restriction)

### 5.2 Environment Variables

| Variable | Default | Description |
|---|---|---|
| `CLOTO_ALLOW_UNSIGNED` | `false` | Skip Magic Seal verification (dev mode) |
| `CLOTO_DEFAULT_TRUST_LEVEL` | `standard` | Default for servers without explicit trust_level |
| `CLOTO_ISOLATION_ENABLED` | `true` | Master switch for OS isolation (disable for debugging) |
| `CLOTO_SANDBOX_DIR` | `data/mcp-sandbox` | Base directory for server sandboxes |

---

## 6. MGP Error Code Integration

When OS-level isolation blocks an operation, the kernel translates the OS signal
into the appropriate MGP §14 error code:

| OS Event | MGP Code | Category | Retryable |
|---|---|---|---|
| `SIGXCPU` / CPU time exceeded | 3003 TIMEOUT | resource | No |
| OOM / memory limit exceeded | 3001 RESOURCE_EXHAUSTED | resource | No |
| `EACCES` / filesystem denied | 1010 VALIDATION_BLOCKED | security | No |
| `ECONNREFUSED` / network denied | 1000 PERMISSION_DENIED | security | No |
| `EPERM` / fork denied | 1000 PERMISSION_DENIED | security | No |
| Server crash (any signal) | 2000 SERVER_NOT_READY | lifecycle | Yes (auto_restart) |

All isolation violations generate an audit event (MGP §6):

```json
{
  "method": "notifications/mgp.audit",
  "params": {
    "event_type": "ISOLATION_VIOLATION",
    "actor": { "type": "server", "id": "tool.terminal" },
    "target": { "resource": "memory", "limit_mb": 512 },
    "result": "BLOCKED",
    "details": {
      "isolation_layer": "L1",
      "os_mechanism": "setrlimit",
      "trust_level": "standard",
      "platform": "linux"
    }
  }
}
```

---

## 7. Cross-Platform Support Matrix

| Layer | Linux | Windows | macOS |
|---|---|---|---|
| **L0: Magic Seal** | HMAC-SHA256 | HMAC-SHA256 | HMAC-SHA256 |
| **L1: Memory** | `RLIMIT_AS` | Job Object | `RLIMIT_AS` |
| **L1: CPU** | `RLIMIT_CPU` | Job Object | `RLIMIT_CPU` |
| **L1: Open files** | `RLIMIT_NOFILE` | Inherited | `RLIMIT_NOFILE` |
| **L2: FS (hard)** | Mount namespace | Restricted Token + ACL | `sandbox-exec` |
| **L2: FS (soft)** | `chdir` + env | `chdir` + env | `chdir` + env |
| **L3: Net (hard)** | seccomp `connect` filter | WFP rules | `sandbox-exec` |
| **L3: Net (soft)** | `HTTP_PROXY` env | `HTTP_PROXY` env | `HTTP_PROXY` env |
| **L4: Process** | `RLIMIT_NPROC` / seccomp | Job Object | `RLIMIT_NPROC` |

**Minimum guarantee (all platforms, Phase 1):**
- L0: HMAC verification (pure Rust, no OS dependency)
- L1: Kernel-side timeout via `tokio::time::timeout` on tool calls
- L2: `chdir` + environment variable guidance
- L3: Environment variable guidance (`CLOTO_LLM_PROXY`)
- L4: None (Phase 1 does not restrict process spawning)

---

## 8. Implementation Phases

### Phase 1: Foundation (Magic Seal + Soft Isolation)

**Scope:** Cross-platform, no privileged operations required.

**Changes:**

| File | Change |
|---|---|
| `mcp_protocol.rs` | Add `IsolationProfile` struct, `seal` field to `McpServerConfig` |
| `mcp_transport.rs` | HMAC verification before spawn; `chdir` + env injection |
| `mcp.rs` | `trust_level → IsolationProfile` derivation logic |
| `mcp.toml` | Add `[servers.isolation]` parsing |
| `lib.rs` | Add `CLOTO_SANDBOX_DIR` config |

**Deliverables:**
- Magic Seal HMAC-SHA256 verification (block unsigned `core`/`standard` servers)
- Soft filesystem isolation (`chdir` to sandbox, `HOME`/`TMPDIR` override)
- Soft network isolation (`CLOTO_LLM_PROXY` env, no raw API key env vars)
- `trust_level → IsolationProfile` auto-derivation
- `[servers.isolation]` override parsing and validation
- CLI: `cloto seal generate <server-id>` and `cloto seal verify <server-id>`

### Phase 2: OS Enforcement (Resource Limits)

**Scope:** Platform-specific, no privileged operations on Linux/macOS. Windows
requires no special privileges for Job Objects.

**Changes:**

| File | Change |
|---|---|
| `mcp_transport.rs` | `apply_resource_limits()` in `pre_exec` (Linux/macOS) or Job Object (Windows) |
| `mcp.rs` | Detect resource-limit kills, translate to MGP errors |
| `managers/scheduler.rs` | Timeout enforcement for cron-dispatched tool calls |

**Deliverables:**
- Memory limits (`RLIMIT_AS` / Job Object)
- CPU time limits (`RLIMIT_CPU` / Job Object)
- Child process limits (`RLIMIT_NPROC` / Job Object)
- MGP §14 error translation for resource violations
- `ISOLATION_VIOLATION` audit events
- Dashboard: isolation status indicator per server

### Phase 3: Hard Isolation (Linux-Specific)

**Scope:** Linux-only advanced isolation. Windows/macOS remain at Phase 2 level.

**Changes:**

| File | Change |
|---|---|
| `mcp_transport.rs` | Unprivileged user namespace + mount namespace setup |
| `mcp_transport.rs` | seccomp BPF filter for `connect`, `execve`, `socket` |
| `capabilities.rs` | Network allowlist enforcement via seccomp |

**Deliverables:**
- Mount namespace filesystem isolation (hard sandbox)
- seccomp filter for network control (`connect` to proxy only)
- seccomp filter for process spawn control (`execve` blocked or allowlisted)
- `PR_SET_NO_NEW_PRIVS` on all MCP server processes
- No root/sudo required (unprivileged user namespaces)

**Platform fallback:** Windows and macOS continue using Phase 2 mechanisms.
Future macOS support may use `sandbox-exec` profiles.

---

## 9. Interaction with Existing MGP Sections

| MGP Section | Interaction |
|---|---|
| §2 Capability Negotiation | `trust_level` from handshake drives isolation profile selection |
| §3 Permission Declarations | `permissions_required` upgrades isolation profile within trust_level bounds |
| §4 Tool Security Metadata | `validator` field is enforced by kernel regardless of OS isolation (Defense-in-Depth) |
| §5 Access Control | RBAC is enforced at protocol level; OS isolation is an independent second layer |
| §6 Audit Trail | `ISOLATION_VIOLATION` events integrate into existing audit infrastructure |
| §7 Code Safety | Code validation occurs before OS-level enforcement (belt and suspenders) |
| §11 Lifecycle | `auto_restart` respects isolation profile — restarted server gets same restrictions |
| §13.4 LLM Proxy | Proxy is the only allowed network endpoint for `proxy_only` servers |
| §14 Error Handling | OS-level blocks are translated to standard MGP error codes |
| §16 Tool Discovery | Dynamically created servers inherit `experimental` trust_level (maximum restriction) |

---

## 10. Security Invariants

The following invariants MUST hold at all times:

1. **No unsigned `core` or `standard` server runs in production.**
   (`CLOTO_ALLOW_UNSIGNED=false` in production builds)

2. **Isolation profiles are immutable after spawn.**
   A running server cannot modify its own resource limits or filesystem scope.

3. **`trust_level` cannot be self-elevated.**
   The kernel determines the effective trust_level from the seal signature and
   mcp.toml config. The server MAY declare `trust_level` in its handshake response
   (MGP §2.3), but this value is informational only. If the server-declared level
   exceeds the kernel-determined level, the kernel silently downgrades it.
   A server claiming `core` without a valid core-level seal is downgraded to
   `untrusted`.

4. **OS isolation failure is a fatal error.**
   If `setrlimit`, Job Object creation, or namespace setup fails, the server is
   not started. The kernel does not fall back to running without isolation.
   Exception: `CLOTO_ISOLATION_ENABLED=false` (development only, logged as warning).

5. **Audit trail is unconditional.**
   `ISOLATION_VIOLATION` events are logged regardless of trust_level, even for
   `core` servers. Observability is never sacrificed.

6. **Cross-platform minimum guarantee.**
   Every platform MUST provide at least soft isolation (Phase 1). Hard isolation
   is platform-specific and additive, never a substitute for the soft baseline.

---

## Appendix A: Threat Model

| Threat | Layer | Mitigation |
|---|---|---|
| Tampered server binary | L0 | Magic Seal HMAC rejects modified files |
| Memory bomb (allocate 64GB) | L1 | RLIMIT_AS / Job Object kills process |
| Infinite loop (100% CPU) | L1 | RLIMIT_CPU / Job Object kills process |
| Fork bomb | L1+L4 | RLIMIT_NPROC + seccomp `execve` filter |
| Read ~/.ssh/id_rsa | L2 | Mount namespace / sandbox-exec denies access |
| Write to kernel database | L2 | Mount namespace / restricted token denies access |
| Raw socket to attacker C2 | L3 | seccomp `socket` filter / WFP rule blocks |
| DNS exfiltration | L3 | seccomp `connect` filter (hard) / proxy-only env (soft) |
| Spawn reverse shell | L4 | seccomp `execve` filter / Job Object process limit |
| Lie about trust_level | §10 inv.3 | Kernel determines trust from seal, not server claim |
| Bypass tool validator | L2+L3+L4 | OS-level blocks even if validator is circumvented |
| Resource starvation of kernel | L1 | Per-process limits, kernel is not resource-limited |

## Appendix B: Glossary

| Term | Definition |
|---|---|
| **Isolation Profile** | The set of OS-level restrictions applied to a single MCP server process. Derived from `trust_level` + `permissions_required` + `[servers.isolation]` overrides. |
| **Magic Seal** | HMAC-SHA256 signature of a server's entry point file, used to verify binary integrity before spawn. |
| **Soft Isolation** | Isolation via environment variables and working directory. Prevents accidental access but can be circumvented by a determined attacker. |
| **Hard Isolation** | Isolation via OS kernel mechanisms (namespaces, seccomp, Job Objects). Cannot be circumvented by the isolated process. |
| **LLM Proxy** | Internal HTTP proxy (MGP §13.4) that injects API keys and routes LLM requests. The only network endpoint accessible to `proxy_only` servers. |
| **trust_level** | A classification of server trustworthiness (`core` > `standard` > `experimental` > `untrusted`) that determines the default isolation profile. |

---

*Document History:*
- 2026-03-05: Initial design (approved). MGP + OS isolation integrated architecture.
- 2026-03-06: Audit fixes — trust_level unified to 4-level taxonomy (§2, Glossary), error codes corrected (§6: 3002→3001, 1004→1010, 1001→1000), extension names updated to `permissions`+`tool_security`+`lifecycle` (§5.1), Security Invariant 3 revised to "cannot be self-elevated" (§10).
