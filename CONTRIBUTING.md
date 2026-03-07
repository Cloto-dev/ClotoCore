# Contributing to ClotoCore

Thank you for your interest in contributing to ClotoCore.

## Before You Start

Please read these documents before making changes:

- [Architecture](docs/ARCHITECTURE.md) — System design, event flow, security model
- [Development](docs/DEVELOPMENT.md) — 8 critical guardrails that constrain safe changes
- [MCP Plugin Architecture](docs/MCP_PLUGIN_ARCHITECTURE.md) — How plugins work

## Development Setup

ClotoCore is a multi-language project: **Rust** (core kernel), **Python** (MCP servers), and **TypeScript** (dashboard).

```bash
git clone https://github.com/Cloto-dev/ClotoCore.git
cd ClotoCore
cp .env.example .env

# Rust
cargo build
cargo test

# Dashboard
npm --prefix dashboard ci
npm --prefix dashboard run build

# Python MCP servers
cd mcp-servers
python -m venv .venv
.venv/Scripts/pip install -r requirements.txt   # Windows
# .venv/bin/pip install -r requirements.txt      # Linux/macOS
cd ..
```

For faster development builds (skips icon embedding):

```bash
export CLOTO_SKIP_ICON_EMBED=1
cargo build
```

## Running Tests

All tests must pass before submitting a pull request:

```bash
# Rust (90 tests)
cargo test --workspace
cargo clippy --workspace -- -D warnings

# Python MCP (45 tests)
cd mcp-servers && .venv/Scripts/python -m pytest tests/ -v

# Dashboard
cd dashboard && npm run build
```

## Code Style

### Rust
- `cargo fmt` — mandatory formatting
- `cargo clippy -- -D warnings` — zero warnings policy
- Function length limit: 100 lines (enforced by clippy `too_many_lines`)

### Python
- Follow existing patterns in `mcp-servers/`
- Use `ToolRegistry` and `auto_tool()` from `common/mcp_utils.py` for new MCP servers
- Use validators from `common/validation.py` for argument extraction

### TypeScript
- Use `useApi()` hook for API calls (not raw `api.*` with manual apiKey)
- Follow existing component patterns in `dashboard/src/`

### General
- Write code and comments in English
- Add tests for new functionality
- Keep commits focused and descriptive

## Adding an MCP Server

1. Create `mcp-servers/<name>/server.py`
2. Use `ToolRegistry` from `common/mcp_utils.py` for tool registration
3. Add the server to `mcp.toml`
4. Add tests to `mcp-servers/tests/`
5. Document in the MCP Servers table in `README.md`

See existing servers (e.g., `terminal/server.py`) for reference.

## Pull Requests

- Keep PRs small and focused on a single change
- Include a clear description of what changed and why
- Ensure all checks pass: `cargo test`, `cargo clippy -- -D warnings`, `pytest`, `npm run build`
- Reference any related issues in the PR description

## Reporting Issues

Use [GitHub Issues](https://github.com/Cloto-dev/ClotoCore/issues) to report bugs or request features. Include:

- Steps to reproduce (for bugs)
- Expected vs actual behavior
- ClotoCore version and OS

For security vulnerabilities, see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions will be licensed under the same [BSL 1.1 license](LICENSE) as the project.
