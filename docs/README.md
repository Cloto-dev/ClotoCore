# Exiv Documentation

Documentation for Exiv design, development, and quality management.

## Quick Links

- **[ARCHITECTURE.md](ARCHITECTURE.md)** - Design principles (Manifesto), security framework, plugin communication, Project Oculi
- **[PROJECT_VISION.md](PROJECT_VISION.md)** - Project identity, competitive positioning, target users, AI Container concept
- **[DEVELOPMENT.md](DEVELOPMENT.md)** - Development guardrails (DO NOT list) and refactoring status
- **[CHANGELOG.md](CHANGELOG.md)** - Project change history (Phase 1 → Phase 6 + Bug Fixes)

## Current Status (2026-02-18)

**Version:** 0.1.0
**Phase:** 6 Complete
**Code Quality:** 90+/100 (A)
**Design Principles Compliance:** 95+/100 (A)
**Test Coverage:** 180+ tests

## Document Index

| File | Description | Last Updated |
|------|-------------|--------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Design principles, security framework, plugin architecture | 2026-02-18 |
| [PROJECT_VISION.md](PROJECT_VISION.md) | Project vision, competitive positioning, roadmap | 2026-02-18 |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Development guidelines, guardrails, coding standards | 2026-02-18 |
| [CHANGELOG.md](CHANGELOG.md) | Comprehensive change history across all phases | 2026-02-18 |
| [SCHEMA.md](SCHEMA.md) | Database schema reference (8 tables, migration history) | 2026-02-19 |
| [MCP_SERVER_UI_DESIGN.md](MCP_SERVER_UI_DESIGN.md) | MCP Server Management UI design (Master-Detail, access control tree) | 2026-02-23 |

## MCP Server Documentation

| Server | Location | Description |
|--------|----------|-------------|
| exiv-mcp-terminal | `mcp-servers/terminal/` | Terminal tool execution with sandbox security |
| exiv-mcp-deepseek | `mcp-servers/deepseek/` | DeepSeek reasoning engine integration |
| exiv-mcp-cerebras | `mcp-servers/cerebras/` | Cerebras fast inference engine |
| exiv-mcp-ks22 | `mcp-servers/ks22/` | KS2.2 persistent memory with FTS5 + vector search |
| exiv-mcp-embedding | `mcp-servers/embedding/` | Vector embedding generation (OpenAI API / local ONNX) |

## Getting Started

```bash
# Clone repository
git clone https://github.com/Exiv-ai/Exiv.git
cd Exiv

# Build and run (development)
cargo build
cargo run --package exiv_core

# Build and run (release - optimized)
cargo build --release
cargo run --package exiv_core --release

# Run tests
cargo test

# Fast development builds (skip icon embedding)
export EXIV_SKIP_ICON_EMBED=1
cargo build
```

## API Endpoints

See the main [README](../README.md#api) for the full endpoint reference.

## Contributing

See [DEVELOPMENT.md](DEVELOPMENT.md) for coding standards, guardrails, and development workflow.

## License

See project root LICENSE file.
