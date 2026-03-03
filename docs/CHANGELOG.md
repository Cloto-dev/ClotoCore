# Changelog

All notable changes to ClotoCore are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows the project's phase scheme: Alpha (A), Beta (βX.Y = 0.X.Y), Stable (1.X.Y).

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
- Correct MCP server naming convention (core.ks22 → memory.ks22)
- Clean up commit history (157 → 1 commit, author unified)

---

## [0.1.0] — 2026-02-26 (β1)

Initial release of ClotoCore — an AI agent orchestration platform built on
a Rust kernel with MCP-based plugin architecture.

### Core Architecture

- Event-driven Rust kernel with actor-model plugin system
- MCP (Model Context Protocol) as the sole plugin interface
- 5 MCP servers: Cerebras, DeepSeek, Embedding, KS22 Memory, Terminal
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
