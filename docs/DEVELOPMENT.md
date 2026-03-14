# ClotoCore Development Guide

A unified document covering the guardrails (constraint rules) that developers must follow, and the current status of ongoing refactoring efforts.

---

## 1. Refactoring Guardrails (What NOT to Do)

Always review this list before making code changes and adhere to the constraints.

### 1.1 Security Hardening: Event Envelopes

**Goal**: Wrap `ClotoEvent` in a Kernel-managed envelope to prevent issuer tampering.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Create an `EventEnvelope` struct | Do not add `issuer_id` to `ClotoEvent` itself | Plugins could spoof the ID |
| Verify `issuer` in `EventProcessor` | Do not hard-code privilege checks like `if plugin_id == "admin"` | Violates Principle #2 (Capability over Concrete Type) |
| Modify `on_event` arguments for plugins | Do not allow plugins to rewrite `issuer` | Compromises data integrity after sealing |
| Adjust SSE output | Do not break the existing JSON format | A classic example of the "infinite loop" that breaks the Dashboard |
| Construct REST API responses | Do not directly write `serde_json::json!({ "status": "..." })` | Must go through the response helper (`ok_data`) (ARCHITECTURE.md §0.6) |
| Modify `dispatch_event` signature | Do not allow plugins to call `dispatch` directly inside `on_event` | Event dispatching that bypasses the Kernel is a vector for spoofing |

### 1.2 Cascading Protection: Event Depth Tracking

**Goal**: Prevent resource exhaustion caused by infinite loops or excessive event chaining.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Add `depth: u8` to `EnvelopedEvent` | Do not add `depth` to `ClotoEvent` | Plugins could spoof the depth value |
| Check the upper limit in `dispatch_event` | Do not hard-code the limit value | Use `AppConfig.max_event_depth` for configurability |
| Set `parent.depth + 1` on re-dispatch | Do not fix `depth` at 0 for all events | Cascading would become undetectable |
| Log an error on discard | Do not silently drop events | Debugging would become impossible |

### 1.3 State Management: Lock Aggregation

**Goal**: Simplify internal plugin state management and guarantee atomicity during config updates.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Group related settings into a single `struct` | Do not use separate `RwLock` for each config value | Prevents inconsistent state during updates |
| Use `Arc<RwLock<ConfigStruct>>` | Do not create deep nesting like `Arc<RwLock<Option<Arc<...>>>>` | Reduces readability and increases deadlock risk |
| Perform atomic config updates in `on_event` | Do not insert `await` or acquire other locks mid-update | Prevents deadlocks and loss of atomicity |

### 1.4 Storage & Memory: Chronological Consistency

**Goal**: Ensure that memory recall always retrieves the latest context in the correct order.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Include a sortable timestamp in the key | Do not remove AgentID from the beginning of the key | Range queries would fail and memories from other agents would mix in |
| Use fixed-length timestamp strings | Do not convert raw time values directly to strings | Lexicographic sort would break (e.g., "100" < "9"). Zero-padding is required |
| Reverse messages in `recall` | Do not return them in oldest-first order from the Kernel | LLMs expect context where "newer is further down" |

### 1.5 UI/UX: Clarity of Agency

**Goal**: Maintain a UI/UX where users do not confuse "Agents (conversation partners)" with "Tools (functions)."

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Categorize plugins | Do not display `Tool`-category plugins in the agent list | Prevents increased cognitive load |
| Store agent definitions in the DB | Do not register function-only plugins in the `agents` table | Agents should be limited to "personas" |

### 1.6 Physical Safety: HAL Rate Limiting

**Goal**: Prevent AI runaway during HAL physical operations.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Implement mouse/keyboard operations | Do not execute `InputControl` without rate limiting | Prevents "physical DoS" that could make the entire OS inoperable |
| Allow dangerous operations | Do not perform irreversible operations without explicit user approval | Prevents data loss due to hallucination |

### 1.7 External Process: MCP Resource Control

**Goal**: Prevent resource exhaustion and zombie processes when launching external processes via MCP.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Launch external processes | Do not launch without PID management and termination handling | Zombie processes would continue to consume memory and ports |
| Execute MCP tools | Do not call external tools without timeout settings | A hang could halt the entire Kernel |

### 1.8 Privacy & Biometrics: Camera Usage

**Goal**: Protect privacy during webcam use.

| Step | DO NOT | Reason |
| :--- | :--- | :--- |
| Start the camera | Do not start in the background without user consent | Prevents unauthorized recording and privacy violations |
| Process facial images | Do not save or externally transmit raw facial video | Prevents biometric data leakage. Stream coordinate data only |
| Share gaze data | Do not stream gaze data to non-permitted domains | "What someone is looking at" is itself sensitive information |

---

## 2. Current Refactoring Status

### Phase 5: Post-Audit Security & Performance Hardening (2026-02-13)

**Trigger:** CODE_QUALITY_AUDIT.md (Score: 65/100)

| Category | Item | Status |
|----------|------|--------|
| Security | Removed dummy API key, migrated to environment variable-based auth (`db.rs`) | Done |
| Security | Fixed auth bypass, enforced `CLOTO_API_KEY` requirement in release builds (`handlers.rs`) | Done |
| Security | ~~Python Bridge method whitelist~~ (deleted with python_bridge) | Done |
| Security | ~~Path traversal protection~~ (deleted with python_bridge) | Done |
| Security | Removed unused DISCORD_TOKEN (`.env`) | Done |
| Performance | Event history `Vec` → `VecDeque` (O(1) pop_front) | Done |
| Performance | Whitelist `Vec` → `HashSet` (O(1) lookup) | Done |
| Performance | Python Bridge background reader JoinHandle tracking | Done |
| Quality | Reduced nesting in managers.rs event dispatch | Done |
| Quality | Consolidated React imports in StatusCore.tsx | Done |
| Verification | All 11 tests passing, zero warnings | Done |

**Audit Score Impact:**
- Security (C): 55 → ~75
- Performance (D): 60 → ~80

### Phase 6: Feature Expansion & Hardening (2026-02-14)

**Trigger:** Post-Phase 5 stabilization

| Category | Item | Status |
|----------|------|--------|
| Security | Human-in-the-Loop permission approval workflow (`permission_requests` table) | Done |
| Security | Rate Limiting: per-IP 10 req/s, burst 20 (`middleware.rs`) | Done |
| Security | Audit Logging: full recording of all security events | Done |
| Security | .env file permissions 0600 (Unix) | Done |
| Security | BIND_ADDRESS default 127.0.0.1 (loopback only) | Done |
| Security | CORS origin scheme validation (allow http/https only) | Done |
| Security | cosign keyless signing (release artifacts) | Done |
| Quality | Unit Tests: handlers, db, capabilities, middleware, validation, config | Done |
| Quality | Input validation module (agent creation and config updates) | Done |
| Quality | Atomic file writes (.maintenance file) | Done |
| Feature | Self-Healing Python Bridge (auto-restart, max 3 attempts) | Done (archived — Python Bridge removed in MCP migration) |
| Feature | Build Optimization (`CLOTO_SKIP_ICON_EMBED=1`) | Done |
| Feature | All comments converted to English (international accessibility) | Done |
| Feature | Windows GUI installer (Inno Setup) | Done |
| Feature | GitHub Pages landing page (OS auto-detection) | Done |
| Infra | GitHub Actions release workflow (5 platforms + installer) | Done |

**Test Count:** 90 tests
**Audit Score:** 90+/100

### Remaining Items (Next Phase)

- [ ] Event Envelope: Kernel-managed envelope for event tampering prevention
- [ ] MCP server hot-reload: runtime MCP server reconnection

---

## 3. Versioning

ClotoCore uses a phase-based versioning scheme with three stages.

### Phases

| Phase | Display | Cargo (Semver) | Git Tag | Status |
|-------|---------|---------------|---------|--------|
| Alpha | A1, A2, ... | `0.0.1`, `0.0.2`, ... | `vA1` | Completed (A1–A7) |
| Beta | βX.Y | `0.X.Y` | `v0.X.Y` | **Current (0.6.3-alpha.1)** |
| Stable | 1.X.Y | `1.X.Y` | `v1.X.Y` | Future |

- **Alpha (A)**: Rapid prototyping. Breaking changes expected on every release.
- **Beta (βX.Y)**: Feature complete, stabilization phase. Follows the same X.Y convention as Stable under the `0.` prefix. `X` = major update, `Y` = minor update / patch. Example: β1 → β1.1 → β1.2 → β2 → β2.1.
- **Stable (1.X.Y)**: Production ready. The leading `1` is fixed unless a major architectural overhaul occurs. `X` = major update, `Y` = minor update / patch.

### System vs Plugin Versions

| Component | Versioning | Source of Truth |
|-----------|-----------|----------------|
| System (kernel) | Unified workspace version | `Cargo.toml` → `workspace.package.version` |
| Plugins | Independent per plugin | MCP server manifest (`version` field in `cloto/handshake` response) |
| Dashboard | Matches system version | `dashboard/package.json` |

Plugins maintain their own version numbers because they can evolve independently of the kernel. When creating a new plugin, start at `0.1.0`.

### Release Process

1. Bump the version in `Cargo.toml` (workspace), `dashboard/package.json`, and `dashboard/src-tauri/tauri.conf.json`
2. Commit: `chore: bump version to 0.6.4` (or appropriate version)
3. Create the release via `gh release create` (this auto-creates the git tag — do NOT create tags manually with `git tag`)
4. The GitHub Actions release workflow builds and publishes automatically

---

*Document History:*
- 2026-02-08: Initial guardrails created (Event Security, Cascading Protection, Lock Aggregation, Storage Consistency)
- 2026-02-10: Added UI/UX Clarity, Physical Safety, MCP Resource Control, Privacy & Biometrics
- 2026-02-13: Merged with REFAC_STATUS.md, added Phase 5 completion status
- 2026-02-15: Added Phase 6 completion status, updated remaining tasks
