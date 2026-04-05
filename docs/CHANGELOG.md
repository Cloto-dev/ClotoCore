# Changelog

All notable changes to ClotoCore are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows the project's phase scheme: Alpha (A), Beta (βX.Y = 0.X.Y), Stable (1.X.Y).

---

## [0.6.3-beta.5] — 2026-04-05

### Added
- **Kernel Health Check System** — self-diagnostic scan with auto-repair for database integrity issues
  - Quick Scan: 5 checks (DB connection, orphaned chat messages, orphaned trusted commands, orphaned permission requests, audit chain integrity)
  - Standard Repair: automatic cleanup of orphaned records
  - Startup scan: runs automatically on boot (configurable via `CLOTO_HEALTH_SCAN_ON_STARTUP`, default: on)
  - API: `GET /api/health/scan`, `POST /api/health/repair`
- **Settings > Health tab** — dashboard UI for scan results, manual scan/repair buttons
- Japanese translation for Health tab

---

## [0.6.3-beta.4] — 2026-04-05

### Security
- **Tar path traversal prevention** — marketplace install now validates extracted paths stay within target directory (zip-slip mitigation)
- **Missing authentication** — `get_agent_access` endpoint now requires API key (was unauthenticated unlike all other endpoints)
- **Code validator bypass** — blocked pattern matching now uses case-insensitive comparison, preventing `Eval()`/`EXEC()` bypass
- **Revoked key check logging** — lock acquisition failure during revoked key check is now logged instead of silently skipped

### Fixed
- **UTF-8 panic** — tool hint truncation now uses char-based indexing instead of byte slicing, preventing panic on multi-byte characters
- **Negative chat limit** — `limit` parameter now clamped to minimum 1, preventing `usize` wrapping on negative input
- **Iteration overflow** — agentic loop counter uses `saturating_add` to prevent theoretical u8 overflow
- **Script path inconsistency** — dynamic MCP server script restoration now uses same path (`data/mcp_scripts/`) as creation
- **Attachment storage path** — uses `state.data_dir` instead of relative path, consistent with VRM/avatar storage
- **Non-transactional agent deletion** — `delete_agent` now wraps all DB operations in a transaction for consistency
- **DB timeout gaps** — added `db_timeout` to all cron (8 functions) and LLM (5 functions) DB operations
- **Audit log timeout** — `write_audit_log` transaction now wrapped with configurable timeout
- **Mutex poison recovery** — `StreamAssembler` and `ToolIndex`/`SessionToolCache` now recover from poisoned mutexes instead of panicking

### Added
- `MgpCapabilities` helper usage documented in quickstart guide
- Coming Soon placeholders for non-functional log UI elements

---

## [0.6.3-beta.3] — 2026-04-04

### Fixed
- **MCP server toggle race condition** (Issue #65) — `stop_server()` now waits for child process exit before returning, preventing DB lock conflicts on restart
- **Safe integer casts** — `i64→i32` (cron), `usize→u8` (delegation chain), `u64→u8` (cron generation) now use `try_from` instead of `as` casts
- **Error logging** — Cargo.toml read errors in marketplace and RwLock poison recovery now logged instead of silenced
- **CVE-2026-33672** — picomatch 2.3.1→2.3.2, 4.0.3→4.0.4 (glob matching method injection)

### Added
- [MCP/MGP Server Quickstart](docs/QUICKSTART_MCP_SERVER.md) — two-path guide for new server developers
- `CLOTO_YOLO_EXCEPTIONS` documented in README configuration table
- Tauri dev note in Quick Start section

### Changed
- README: test badge 351→234, security section +3 items, documentation links updated
- CHANGELOG: removed internal marketing references from beta.1 and beta.2 entries
- CLAUDE.md: clarified issue registry as anti-hallucination tool

### Dependencies
- jsdom 28.1.0→29.0.1 (dev)

---

## [0.6.3-beta.2] — 2026-04-04

### Security Hardening (8 layers)
- **L2**: Kernel tool RBAC — `mgp.*`/`gui.*` tools now checked via `resolve_tool_access(server_id="kernel")`; default Allow, explicit Deny entries restrict specific agents
- **L3**: YOLO mode exceptions — `CLOTO_YOLO_EXCEPTIONS` env var (default: `filesystem.write,network.outbound`); excepted permissions require approval even in YOLO mode
- **L5**: Merkle chain audit log — `chain_hash` column on `audit_logs` table; each entry hashed with SHA-256(previous_hash | canonical_data) for tamper detection
- **L8**: Runtime host whitelist audit — `add_host()` logs a warning when a new host is added
- **L9**: Event depth hardening — `MAX_EVENT_DEPTH` cap lowered from 50 to 25; warning logged at depth > 5
- **L11**: MGP permission declarations — avatar and discord servers declare `permissions_required: ["network.outbound"]` in initialize response (cloto-mcp-servers)
- **L12**: destructiveHint HITL gate — parse MCP `annotations.destructiveHint`, require approval for destructive tools via existing command approval flow
- **L14**: Trust level mismatch warning — kernel logs when server self-declares higher trust than config allows

### Added
- `McpTool.annotations` field for MCP tool annotation parsing
- `McpClientManager::is_tool_destructive()` helper
- `CLOTO_YOLO_EXCEPTIONS` environment variable
- Planned breaking changes section in PROJECT_VISION (§10)
- DB migration: `audit_logs.chain_hash` column

### Changed
- YOLO permission flow refactored to partition-based logic (auto-approvable vs excepted)
- `write_audit_log()` now uses single SQLite transaction for chain hash consistency

### Fixed
- 3 stale issue-registry bugs marked as fixed (bug-311, bug-314, bug-343)

### Documentation
- Security layer audit report (15 layers verified against code)
- GitHub Sponsors FUNDING.yml

---

## [0.6.3-beta.1] — 2026-04-03

### Added
- **Streamable HTTP** transport for remote MCP server connections
- Agentic loop for `ask_agent` tool execution chains
- Discord conversation context injection into LLM calls
- Discord callback metadata forwarding to agent messages
- CPersona **memory channel** support for channel-based context separation
- Actions panel with inter-agent dialogue visibility
- CRON job execution display in Actions Dialogues
- Engine selector on CRON job creation form
- Memory **export/import** UI in MemoryCore
- Marketplace changelog display on update-available cards
- MGP badge and glow effect on MCP server cards
- Agent processing glow indicator in sidebar
- Speaker name display on memory cards
- **IO category** for bidirectional MCP servers in dashboard
- Process relaunch on error boundary restart
- Marketplace actions locked in dev mode by default
- Pre-compute archive/profile via CFR engine in background
- Generalized `tool_hint` for direct tool execution bypass
- `CapabilityType::Speech` with capability-based **auto-speak**
- Installer (Experimental) section in README with setup wizard fix

### Changed
- `io.discord.karin` renamed to `io.discord`
- Per-agent Discord server entry template
- Dialogues tab bar replaced with **vertical scroll list**
- Agent description limit increased from 1000 to 5000 bytes

### Fixed
- MCP venv: parallel pip install replaced with **single invocation**
- Stale Python venv detection and automatic recreation
- pip install timeout and `--no-input` flags
- Python venv and cargo build timeouts
- Duplicate tool names in LLM tool schemas
- Null reference in Discord callback metadata
- Avatar **cache-bust** on re-upload
- Avatar vision analysis skipped when agent lacks Vision access
- CRON dialogue response pairing
- Memory card text size and description textarea height
- Export/import icon semantics corrected
- Speech tool schema exclusion limited to "speak" tool only
- Setup wizard download URL fixed (points to cloto-mcp-servers releases)
- Setup wizard server/venv paths corrected for production layout
- `detect_project_root()` recognizes `cloto-mcp-servers/` directory

### Security
- Authentication, cryptography, MCP server creation, and Tauri capabilities **hardened**
- GitHub Actions pinned to commit SHAs
- CVE-2026-33055, CVE-2026-33056 (tar crate update)
- Access control magic strings replaced with typed enums
- `aria-label` added to all interactive dashboard components

### Documentation
- Code quality audit report (65 findings — 2 critical, 19 high, all fixed)
- Documentation-codebase **integrity audit** (3 critical, 7 high, 5 medium fixed)
- CPersona design document updated to v2.4.6 (tool count, version table, architecture diagram)
- MGP specification kernel tool count corrected (17→25)
- GUI component map updated
- Test count corrected: 351 (234 Rust + 117 Python)

---

## [0.6.3-alpha.11] — 2026-03-21

### Changed
- MGP renamed from "Model General Protocol" to "**Multi-Agent Gateway Protocol**"
- Release assets consolidated from 34 to 22 (SHA256SUMS.txt replaces per-file checksums)

### Fixed
- Kernel startup failure no longer **silently ignored** — `start_kernel()` refactor with Tauri error dialog
- LLM proxy bind failure reported in background — no longer blocks HTTP server startup
- MCP deferred boot **race condition** resolved — `Arc<Notify>` replaces `yield_now()`
- Tauri tray icon panic prevented
- EventManager **mutex poisoning** cascade eliminated across 13 sites
- McpAccessControlTab infinite re-render loop
- Cross-platform MCP path normalization
- MCP config parse error visibility improved
- Faster **graceful shutdown** — concurrent drain with 10-second cap
- pip install timeout and `--no-input` to prevent setup hangs
- Stale venv auto-detection — compares Python major.minor, auto-recreates on mismatch
- Text size and color rule violations in dashboard

### Security
- `aws-lc-sys` updated (RUSTSEC-2026-0044, RUSTSEC-2026-0048)
- `rustls-webpki` updated (RUSTSEC-2026-0049)

---

## [0.6.3-alpha.10] — 2026-03-20

### Fixed
- **Empty MCP server list** after NSIS installation — `mcp.toml` now embedded in binary via `include_str!` and extracted to `data/mcp.toml` on first launch with snapshot pattern
- Removed broken Tauri `resources` bundling (Tauri v2 transforms `../` into literal `_up_` directories)

---

## [0.6.3-alpha.9] — 2026-03-20

### Fixed
- **Empty MCP server list** after installation — `mcp.toml` bundled as Tauri resource for first-launch discovery
- `exe_dir/mcp.toml` added as production fallback path
- `CLOTO_MCP_SERVERS` fallback probes multiple candidate directories (bundled, sibling repo, legacy layout)
- Always-true assertion removed from security forging test

---

## [0.6.3-alpha.8] — 2026-03-19

### Changed
- `.env.example` updated with Ollama config
- Outdated `CODE_QUALITY_REPORT.md` removed

### Fixed
- Dashboard: gate **console statements** behind `import.meta.env.DEV`
- Dashboard: improve catch block type safety, extract magic numbers, remove dead CSS
- Rename legacy `karin` color to `cloto`
- All **clippy warnings** resolved
- Warn logging added to silent I/O errors in system handler
- Benchmark helpers updated to match current `AppState` struct
- `ask_agent` tool description improved
- CI: cargo fmt violations and missing assertion fixed

---

## [0.6.3-alpha.7] — 2026-03-17

### Added
- **Rust MCP server** support in marketplace — servers with `runtime: "rust"` built with `cargo build --release`, with toolchain detection and build progress streaming
- Startup timing log (`startup: X.Xs`)
- Rust badge on marketplace cards
- MCP startup performance analysis report

### Changed
- MCP server connections **parallelized** — `connect_server_configs()` uses `join_all` for concurrent connections
- Parallel **venv dependency sync** — pip install runs concurrently for all servers
- Background venv sync — `ensure_mcp_venv()` moved off critical startup path
- **Startup time reduced from ~40s to ~7s**
- `output.avatar` removed from `mcp.toml` (marketplace-only distribution)

### Fixed
- Sidebar "Agents" nav not returning to agent selection
- Agent config screen highlighting wrong **sidebar** item
- Sidebar agent click not working after returning from chat/config
- Config screen persisting when navigating away
- Marketplace install blocked for config-loaded servers
- Cargo build failing in data dir due to parent **workspace detection**
- CI: cargo fmt, flaky seal key test, issue registry

---

## [0.6.3-alpha.6] — 2026-03-16

### Changed
- `mcp.toml` **portability**: `${CLOTO_MCP_SERVERS}` env var with sibling-repo fallback
- `resolve_servers_dir_from_config()` resolves relative paths against project root

### Fixed
- Update checker detects **pre-release** versions via `/releases` API
- Pre-release segment version comparison (alpha.4 vs alpha.5)
- Setup wizard Python pre-check with download link and **retry button**
- Hardcoded developer-machine paths removed from history

---

## [0.6.3-alpha.5] — 2026-03-16

### Added
- **VRM thumbnail extraction** and avatar offer dialog
- i18n: Japanese translations for VRM dialog and settings sections
- CFR default enabled for new routing rules
- Engine selection **persistence** per agent in localStorage

### Changed
- Deferred save pattern unified for all agent config
- `output.avatar` migrated to `cloto-mcp-servers` repository

### Fixed
- VRM upload metadata **COALESCE** race condition
- Null injection prevention in metadata
- Marketplace refresh bypasses server cache
- Duplicate refresh buttons unified

---

## [0.6.3-alpha.4] — 2026-03-16

### Added
- **Agent state persistence** across navigation (thinking steps, chat, SSE)
- Code block header bar with language label, copy, and download
- Artifact panel collapse/expand with **sessionStorage** persistence
- MCP server lifecycle feedback (spinner + checkmark)

### Fixed
- CI: MSI target exclusion, clippy errors, biome lint, sentinel assertion
- Workspace-wide **cargo fmt**

---

## [0.6.3-alpha.3] — 2026-03-16

### Added
- **Auto-update** check on startup (Tauri only, configurable)
- Discord-style update indicator in header
- Tool hint display shows actual command name

### Changed
- Unified **card styles** across all dashboard components

### Fixed
- Avatar upload/deletion **race condition**
- Avatar upload spinner hang with 30-second timeout
- Semver comparison for pre-release versions

### Documentation
- CPersona v2.5/v3.0 roadmap added

---

## [0.6.3-alpha.2] — 2026-03-16

### Added
- **MCP Server Marketplace** (Phase 1 & 2) — catalog, install, batch install with SSE progress
- **Setup flow unification** with SSE progress streaming
- Tier-1 rate limiting and startup dependency sync
- Biome formatter, lefthook pre-commit hooks, sentinel script
- 14 unit tests for marketplace and setup

### Fixed
- `resolve_servers_dir_from_config` **TOML parsing** failure (critical)
- Avatar deletion and AgentConsole TDZ crash
- Install path resolution fixes

---

## [0.6.3-alpha.1] — 2026-03-13

### Added
- **CPersona background task queue** (Phase 5) ported from predecessor
- SSE **SequencedEvent** with `Last-Event-ID` replay for reliable event streaming
- Cron `source_type` field to distinguish user vs system messages
- VRM thinking pose auto-application on agent thinking state
- `AgentThinking` event emission before all LLM calls
- Host OS info injected into agent system prompt

### Fixed
- **WebView2** startup crash (`ERR_CONNECTION_REFUSED`) on release builds
- `useLongPress` stale closure in long-press handler
- Chat message deletion not including system rows
- Web search false-positive health check

### Documentation
- VOICEVOX credit added to README
- CPersona **2.x versioning** adopted (inheriting KS2.x lineage)
- 8 internal audit docs moved to `.dev-notes/`
- Comprehensive `CPERSONA_MEMORY_DESIGN.md` update (20 fixes)

---

## [0.6.2] — 2026-03-11

### Added
- **VRM Avatar System** (Layer 1) — full procedural animation pipeline (breathing, blinking, micro-sway, gaze drift, agent state transitions, default pose with smooth transitions)
- **VRMA pose system** with direct quaternion application, smooth slerp-based transitions, drag-and-drop loading
- VRM **expression mapper** — cross-avatar compatibility layer with fallback chains
- **mgp-avatar MCP server** — VOICEVOX TTS with automatic viseme extraction for real-time lip sync
- MGP `set_pose` tool for avatar pose control (relaxed, attentive, thinking, arms_crossed)
- Auto-speak final LLM response with configurable bypass
- VRMA thinking pose preset (Blender-authored)
- Eye narrowing during thinking state
- Middle-click orbit rotation in VRM viewer
- Extended **bone controls** (neck, spine, head, hand)
- Core architecture refactor (Phase 1–8): **CapabilityDispatcher**, metadata JSON migration, unified state consolidation, process lifecycle safety, permission events, config-driven capabilities, agent type filtering
- **OS-level isolation** (MGP §8-10) for MCP server sandboxing
- MGP tool discovery **latency tier** scoring (Tier C/B/A/S)

### Fixed
- VOICEVOX `accent_phrases` field name for viseme extraction
- Pre-phoneme compensation for accurate **lip sync** timing
- Audio cutoff prevention and viseme desynchronization
- Bypass agentic loop for TTS (prevent prompt readback)
- VOICEVOX pipeline and sandbox path resolution

---

## [0.6.1] — 2026-03-09

### Added
- `ask_agent` kernel tool for inter-agent delegation
- `AgentThinking` SSE events for LLM intermediate reasoning display
- `gui.map` and `gui.read` kernel tools for dynamic UI documentation
- Agent config presets with deferred avatar save
- MGP dead code integrated into runtime (Phase 0-5)
- GitHub Issue Sync workflow: auto-create/close GitHub Issues from `qa/issue-registry.json`

### Changed
- Rename `memory.ks22` to `memory.cpersona` across entire codebase
- Auto-grant MCP access on agent creation
- Settings screen text sizes increased for readability

### Fixed
- Comprehensive data cleanup on agent deletion (bug-231)
- Dashboard API key delivery in Tauri mode and FK violations in MCP access control
- `useAgents` cache race condition, wizard UX improvements, and 6 pre-existing TS errors
- Dashboard UI/UX improvements and LLM thinking event support

### Documentation
- Backfill CHANGELOG entries for v0.6.0-alpha.4 through v0.6.0 stable (MGP content)

---

## [0.6.0] — 2026-03-08

### Added
- **MGP (Multi-Agent Gateway Protocol) Tier 1-4 implementation complete**
  - Tier 1: Security primitives — protocol-level access control and audit trails
  - Tier 2: Observability — monitoring, metrics, and diagnostic capabilities
  - Tier 3: Bidirectional communication — server→kernel notifications and tool discovery
  - Tier 4: Intelligence Layer — context management, adaptive behavior, and compliance
- 17 MGP kernel tools in `mgp.*` namespace (access control, audit, lifecycle, streaming, discovery)
- MGP server creation with coordinator pattern
- Priority boot sequence for MGP servers
- Tool discovery stress tests and context reduction measurements

### Fixed
- MGP Tier 1-3 spec compliance (bug-182 to bug-222)
- Missing Tier 4 tool schemas registered; `tool_history` sanitization hardened
- MGP kernel tool execution and LLM provider integration
- Stale connection status threshold removed for immediate disconnect detection
- Linux Tauri build deps: added libgbm-dev, libegl-dev, libxcb1-dev
- macOS CI: upgrade xcap 0.0.13 → 0.8
- Linux CI: switch to ubuntu-24.04 for libspa 0.9.2 compatibility

### Documentation
- MGP implementation roadmap added
- MGP documentation updated to reflect Tier 1-4 completion

---

## [0.6.0-beta.3] — 2026-03-07

### Added
- First-run setup wizard
- Agent config export/import

### Fixed
- Hide export button for default agent (Cloto Assistant)

---

## [0.6.0-beta.2] — 2026-03-07

### Added
- Modular i18n with react-i18next (EN + JA)
- Filesystem-based language packs with extended translations and text readability enforcement

### Removed
- Container agent type from dashboard

---

## [0.6.0-beta.1] — 2026-03-07

### Added
- Semantic cache for research server
- TTL-based LRU cache for query embeddings in KS22

### Removed
- Predecessor project references from codebase

---

## [0.6.0-alpha.5] — 2026-03-07

### Changed
- Codebase reduced by ~1,400 LOC with structural improvements

### Fixed
- 5 LOW bugs resolved, Python MCP test base added, 2 reclassified as wontfix

### Removed
- Orphaned `runtime_plugins` and `agent_plugins` tables dropped

---

## [0.6.0-alpha.4] — 2026-03-06

### Added
- Cross-platform Tauri desktop app support (Linux + macOS)
- macOS code signing and notarization configuration
- Configurable settings extracted from hardcoded values

### Fixed
- Version prerelease label auto-generation from package version

---

## [0.6.0-alpha.3] — 2026-03-06

### Fixed
- 8 MEDIUM bugs resolved, improved Python MCP server quality
- Graceful shutdown now broadcasts to all tasks (not just one listener)
- MCP stderr log noise suppressed

---

## [0.6.0-alpha.2] — 2026-03-05

### Removed
- CLI crate (`cloto_system` binary removed)
- Status UI page

### Fixed
- MCP server restore bug on kernel restart

### Security
- Authentication added to read-only APIs (agents, plugins, metrics, memories)
- YOLO mode audit log: all auto-approved actions recorded
- Revoked API keys now expire with TTL cleanup

---

## [0.6.0-alpha.1] — 2026-03-05

### Added
- MGP specification v0.6.0-draft: structural audit, architectural revision, split into maintainable part files
- SearXNG self-hosted search via Docker Compose
- Multi-provider search fallback chain for MCP
- Reliable chat message persistence with retry logic

### Changed
- Replace Inno Setup installer with Tauri NSIS installer (Windows)
- Dashboard: extract shared UI components and utility hooks

### Fixed
- MGP integrity scan findings resolved (S1-S3, I1-I3, X1)
- Windows console windows appearing from MCP server child processes
- Kernel images blocked by CSP `img-src` directive
- Release pipeline: Ed25519 signing, artifact paths, macOS runner, cosign verification

---

## [0.5.11] — 2026-03-04

### Changed
- Unified REST API response envelope (`{ "data": ... }` / `{ "error": ... }`)
- Auto-generate Tauri API key on first launch

---

## [0.5.10] — 2026-03-04

### Added
- Multi-user identity propagation across the full pipeline (chat, agentic loop, MCP tools, memory)

---

## [0.5.9] — 2026-03-04

### Fixed
- Memory contamination causing time hallucination in agent responses

---

## [0.5.8] — 2026-03-04

### Changed
- Dashboard UI/UX refinements: retry fix, MemoryCore design unification, engine selector polish

---

## [0.5.7] — 2026-03-04

### Added
- CRON autonomy security: recursion depth control and audit log guarantee

---

## [0.5.5] — 2026-03-04

### Added
- Gemini-style engine switcher in chat input bar

---

## [0.5.4] — 2026-03-04

### Added
- `tool.cron` MCP server: stateless CRON job management via kernel REST API (create, list, delete, toggle, run now)
- `tool.agent_utils` MCP server: 8 deterministic utility tools (time, math, date arithmetic, random, UUID, unit conversion, encode/decode, hash)
- Default MCP server grants for Cloto Assistant: memory.cpersona, tool.cron, tool.terminal, tool.websearch, tool.research, tool.agent_utils
- Cydonia 24B v4.3 (TheDrummer) Q4_K_M Ollama model support with ChatML template

### Fixed
- Default engine routing: Cloto Assistant was incorrectly using mind.deepseek instead of mind.cerebras (migration WHERE condition bug)
- ONNX embedding server: missing `token_type_ids` input caused all-MiniLM-L6-v2 inference to fail, breaking memory recall
- Response latency reduced from ~7.4s to ~2s (engine fix + embedding fix)

### Changed
- Ollama default model changed from glm-4.7-flash to cydonia
- Code cleanup: reduced ~600 lines across DB layer, handlers, and docs

---

## [0.4.22] — 2026-03-03

### Added
- CFR (Cost-First Router): high-speed engine tries first, escalates to high-quality engine on `[[ESCALATE]]`
- Auto-fallback: retriable errors (429/5xx/connection) automatically switch to fallback engine
- Routing rule extensions: `cfr`, `escalate_to`, `fallback` fields (backward-compatible)
- Dashboard UI: CFR toggle, escalation target, fallback selector in routing rule builder

---

## [0.4.21] — 2026-03-03

### Added
- Command approval system: HITL gate for terminal commands (Yes/Trust/No)
  - Kernel intercepts `execute_command` before MCP dispatch (YOLO mode bypasses)
  - DB-persisted exact match trust ("Yes") + session-scoped command name trust ("Trust")
  - Inline approval card in chat with 60s countdown timer
  - Tauri OS notification when approval pending and user is away
  - API endpoints: `POST /api/commands/:id/{approve,trust,deny}`
  - `trusted_commands` DB table + `CommandApprovalRequested/Result` events

### Changed
- Chat persistence moved from frontend to kernel (backend-complete)
  - User messages persisted in `handle_message()` before processing
  - Agent responses persisted before SSE `ThoughtResponse` emission
  - Frontend `postChatMessage` calls removed (no more fire-and-forget)
- LLM error handling improved across all layers
  - L1 (Proxy): HTTP status → user-friendly message + error code (`auth_failed`, `rate_limited`, etc.)
  - L2 (MCP Python): `LlmApiError` class replaces raw `raise_for_status()`, structured error response
  - L3 (Kernel): `format_engine_error()` adds actionable guidance per error code
  - L4 (Dashboard): `[Error]` messages displayed as amber error cards instead of plain text
  - Internal URLs (`127.0.0.1:8082`) no longer exposed to users
- Reset button long-press reduced from 2s to 1.5s
- Thinking state recovery: 30s timeout + `visibilitychange` listener to handle missed SSE responses

---

## [0.4.20] — 2026-03-03

### Added
- Dashboard update checker: "Check for Updates" button in Settings → About
- GitHub API-based version comparison with release notes display
- "Update Now" via Tauri shell plugin (desktop mode only)
- Tauri Native Auto-Update design (integrated into `docs/INSTALLER_DISTRIBUTION.md` § 6) for future implementation

---

## [0.4.19] — 2026-03-03

### Changed
- Extract password verification helper to `handlers/utils.rs` (deduplicate 2x20-line blocks)
- Python MCP server factory: `create_llm_mcp_server()` + `load_llm_provider_config()` reduce cerebras/deepseek to ~27 lines each
- Split `AgentPluginWorkspace.tsx` into `AvatarSection`, `ProfileSection`, `ServerAccessSection` components

---

## [0.4.18] — 2026-03-03

### Changed
- Split monolithic `db.rs` (1,732 lines) into 7 domain modules (`db/{audit,permissions,chat,mcp,api_keys,cron,llm}.rs`)
- Extract `mcp_tool_validator.rs` (~200 lines) from `managers/mcp.rs`
- Centralize validation constants and MIME helpers into `handlers/utils.rs`
- Remove unused npm packages (`clsx`, `tailwind-merge`)
- Remove false-positive `#[allow(dead_code)]` annotations (7 items)
- Remove unused code: `Tick` variant, `selected_agent()`, `create_slow_plugin()`

### Added
- Multi-Agent Delegation design document (`docs/MULTI_AGENT_DESIGN.md`) for v0.5.x

---

## [0.4.17] — 2026-03-03

### Fixed
- Agent card buttons unclickable when avatar background is set (`pointer-events-none` on overlay image)

---

## [0.4.16] — 2026-03-03

### Added
- PaddleOCR hybrid vision: OCR + llava combined analysis with A/B test support (hybrid/vision/ocr modes)
- Agent card avatar background in agent selection screen (blurred, hover effect)
- Default agent protection: name, description, avatar changes blocked for Cloto Assistant

### Changed
- Unified grid background: all 6 screens use `InteractiveGrid` (Canvas) with bottom fade
- Agent config UI: larger avatar preview (96px), bigger buttons, Remove button with red tint
- Agent card buttons enlarged (text-xs, size-14 icons)
- Chat avatar icons fill parent container (size 32-40px with overflow-hidden)
- Sidebar avatar icons enlarged to 24px
- MCP server grant/revoke: one-click on row (no separate button needed)
- Cloto Assistant description updated to reflect full capabilities

### Fixed
- Avatar broken image after delete (local `hasAvatar` state tracking)
- Backend-injected metadata fields polluting save (has_avatar, avatar_description excluded)
- Agent ID sanitization: URL-unsafe characters replaced with underscore
- Duplicate `api` import in AgentTerminal

---

## [0.4.15] — 2026-03-02

### Added
- KS2.2 Phase 2: Vector embedding search (ONNX MiniLM, cosine similarity) activated via mcp.toml config
- KS2.2 Phase 3: LLM-powered memory extraction — profile fact mining and episode summarization via Cerebras
- Auto-download ONNX model on first embedding server startup
- Memory/episode delete API (`DELETE /api/memories/:id`, `DELETE /api/episodes/:id`)
- Memory Core dashboard: delete buttons on memory cards and episode entries
- Auto `update_profile` trigger after episode archival

### Fixed
- Tauri: `mcp.toml` not found due to absolute path fallback not resolving to project root
- Tauri: venv Python not resolved due to `detect_project_root` not shared across modules

---

## [0.4.14] — 2026-03-02

### Added
- Auto-setup MCP Python venv on first kernel startup (`mcp_venv.rs`)
- Auto-resolve `python` command to venv Python in MCP transport (no venv activation needed)
- Cerebras tool calling: `gpt-oss-120b` now exposes `think_with_tools`
- Missing `pyproject.toml` for ollama, websearch, research MCP servers

### Fixed
- Agents using Cerebras engine could not use MCP tools (terminal, etc.) due to `supports_tools=False`

---

## [0.4.13] — 2026-03-02

### Added
- Agent avatar: image upload/serve/delete API (`POST/GET/DELETE /api/agents/:id/avatar`)
- Avatar vision analysis: auto-analyze via vision.capture MCP, description injected into LLM system prompt
- Agent rename: editable name/description fields in agent settings UI
- Clipboard paste: Ctrl+V image attachment support in chat input
- Window maximize on startup (Tauri)
- DB migration: `avatar_path`, `avatar_description` columns on agents table

### Fixed
- Cursor dot remnant when mouse leaves window (add `mouseleave`/`blur` handlers)
- Mermaid diagram text visibility on GitHub dark theme (`color:#333`)

### Quality
- YOLO mode issues registered (bug-170, 171, 172)

---

## [0.4.8] — 2026-03-01

### Added
- Engine routing: rule-based 3-layer engine selection (override > routing rules > default)
- MCP access control: wire up `resolve_tool_access()` 3-level priority resolution
- Episode auto-archival: `maybe_archive_episode()` triggers after 10+ unarchived messages
- McpClient notification handling: Server→Kernel JSON-RPC notification support (MGP §13 foundation)
- CI: `verify-issues` job in GitHub Actions
- CI: Branch Protection with required status checks
- Discord Bridge design document (`docs/DISCORD_BRIDGE_DESIGN.md`)
- MGP spec §19.5 `transport_websocket` extension, §19.6 External Event Bridge Pattern

### Fixed
- XSS: DOMPurify sanitization on `dangerouslySetInnerHTML`
- API key storage moved from localStorage to sessionStorage
- Unsafe `any` types replaced with proper React event types
- JSON parse guard (`safeJsonParse`) in api.ts
- Error state exposed from useAgents hook
- All clippy errors resolved (18 fixes)
- Test baseline updated, dashboard `--passWithNoTests`

### Security
- `default_policy` changed from `opt-in` to `opt-out` for MCP servers
- `save_mcp_server()` preserves `default_policy` on reconnect
- rollup HIGH severity path traversal fix

---

## [0.2.0] — 2026-02-26 (β2)

> Theme: Bug fixes, security hardening, performance improvements, documentation, and refinements

### Bug Fixes

- Resolve all open issues in issue registry (115/115 closed)
- Update 5 obsolete bug entries referencing deleted components
- Add error context to test assertions (`unwrap()` → `expect()`)

### Code Quality

- Suppress `clippy::too_many_lines` for Tauri entry point
- All `cargo clippy --workspace` warnings resolved
- All 90 tests passing, 0 ignored

### Security

- Install and run `cargo audit` — 0 vulnerabilities, 16 warnings (all GTK3 indirect deps, Linux-only)

### Documentation

- Rewrite CHANGELOG to version-based format (Keep a Changelog)
- Add v0.2.0 release scope document
- Fix 12 HIGH, 14 MEDIUM documentation inconsistencies across 9 files
- Align ARCHITECTURE.md, DEVELOPMENT.md, PROJECT_VISION.md with MCP-only architecture
- Update SCHEMA.md with 3 missing tables (runtime_plugins, revoked_keys, agent_plugins)
- Update MAINTAINABILITY.md metrics (crate count, file sizes, test count)
- Correct MCP server naming convention (core.cpersona → memory.cpersona)
- Clean up commit history (157 → 1 commit, author unified)

---

## [0.1.0] — 2026-02-26 (β1)

Initial release of ClotoCore — an AI agent orchestration platform built on
a Rust kernel with MCP-based plugin architecture.

### Core Architecture

- Event-driven Rust kernel with actor-model plugin system
- MCP (Model Context Protocol) as the sole plugin interface
- 5 MCP servers: Cerebras, CPersona Memory, DeepSeek, Embedding, Terminal
- ConsensusOrchestrator for multi-engine LLM coordination
- SQLite persistence with 24 migrations
- Rate limiting, audit logging, and permission isolation

### Dashboard

- React/TypeScript web UI with dark mode
- Agent workspace with MemoryCore design language
- MCP server management UI (Master-Detail layout)
- Real-time SSE event monitoring
- API key management with backend validation and revocation
- Tauri desktop application (multi-platform)

### CLI

- Agent management (create, list, inspect, delete)
- TUI dashboard with ratatui
- Log viewer with SSE follow mode
- Permission management commands

### Agent System

- Per-agent plugin assignment with config-seeded defaults
- Agent lifecycle management (create, delete, default protection)
- Custom skill registration with tool schema support
- Permission enforcement (visibility, revocation, runtime checks)

### Security

- API key authentication with Argon2id hashing
- Key revocation system with SHA-256 tracking
- Path traversal prevention and input validation
- CORS configuration with explicit header allowlists
- Human-in-the-loop permission approval workflow

### Infrastructure

- GitHub Actions CI/CD pipeline (5-platform build)
- Windows GUI installer (Inno Setup) with Japanese localization
- Shell and PowerShell installers with version validation
- GitHub Pages landing page with OS auto-detection
- BSL 1.1 license (converts to MIT on 2028-02-14)
