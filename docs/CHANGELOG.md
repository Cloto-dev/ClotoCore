# Changelog

All notable changes to ClotoCore are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows the project's phase scheme: Alpha (A), Beta (βX.Y = 0.X.Y), Stable (1.X.Y).

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
