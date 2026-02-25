# Changelog

All notable changes to ClotoCore are documented in this file.

Format follows [Keep a Changelog](https://keepachangelog.com/).
Versioning follows the project's phase scheme: Alpha (A), Beta (βX.Y = 0.X.Y), Stable (1.X.Y).

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

### Documentation

- Rewrite CHANGELOG to version-based format (Keep a Changelog)
- Add v0.2.0 release scope document
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
- SQLite persistence with 21 migrations
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
