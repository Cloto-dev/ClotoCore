<div align="center">

# Exiv

### Build Your Own AI Partner

An open-source AI container platform written in Rust.
Sandboxed plugins, GUI dashboard, and your AI stays on your machine.

[![Version](https://img.shields.io/badge/version-0.1.0-blue)](https://github.com/Exiv-ai/Exiv/releases/latest)
[![Tests](https://img.shields.io/badge/tests-78%20passing-brightgreen)]()
[![License](https://img.shields.io/badge/license-BSL%201.1%20→%20MIT%202028-blue)](LICENSE)

[Download](#download) · [Documentation](docs/ARCHITECTURE.md) · [Vision](docs/PROJECT_VISION.md)

</div>

---

## What is Exiv?

Exiv is a platform for building advanced AI agents — not chatbots, not assistants, but **AI partners** with personality, capabilities, and memory.

Inspired by projects like [Neuro-Sama](https://www.twitch.tv/vedal987), Exiv lets anyone construct sophisticated AI systems through a plugin architecture and GUI dashboard, without writing a single line of code.

**AI Container** = Plugin Set + Personality Definition + Capability Set

```
Example: "VTuber AI" Container          Example: "Research Assistant" Container
├── reasoning: DeepSeek                  ├── reasoning: Claude / GPT-4o
├── vision: Screen capture plugin        ├── tools: File search, Web search
├── personality: Character definition    ├── personality: Academic, precise
├── voice: TTS/STT plugin               └── memory: Long-term memory plugin
└── avatar: Live2D/VRM plugin
```

## Why Exiv?

|  | Exiv | Chat-based AI frameworks |
|--|------|--------------------------|
| **Language** | Rust — memory safe, fast, low resource | TypeScript / Python |
| **Security** | Sandboxed plugins, permission isolation, host whitelisting, DNS rebinding protection | Broad local permissions |
| **Interface** | GUI dashboard + Tauri desktop app | Chat / CLI only |
| **Design** | Plugin-composed AI containers | Monolithic agents |
| **Extension** | Rust plugins + Python Bridge | Single language |

## Architecture

```mermaid
graph TB
    Client[Dashboard / HTTP Client] --> Router[Axum Router]
    Router --> Auth[API Key Auth + Rate Limit]
    Auth --> Handlers[Handlers]

    Handlers --> DB[(SQLite)]
    Handlers --> EventBus[Event Bus]

    EventBus --> Processor[Event Processor]
    Processor --> Manager[Plugin Manager]

    Manager --> P1[Rust Plugin]
    Manager --> P2[Python Bridge]
    Manager --> PN[Plugin N]

    P1 --> Cascade[Cascade Events]
    P2 --> Cascade
    Cascade --> EventBus

    P2 -.->|JSON-RPC| Python[Python Subprocess]

    SSE[SSE Stream] --> Client

    style DB fill:#f0e6ff,stroke:#333
    style Processor fill:#e6f0ff,stroke:#333
    style Manager fill:#e6ffe6,stroke:#333
    style Python fill:#fff4e6,stroke:#333
```

**Key design principles:**

- **Core Minimalism** — The kernel is a stage, not an actor. All intelligence lives in plugins.
- **Event-First** — Plugins communicate through an async event bus, never directly.
- **Capability Injection** — Plugins cannot instantiate network clients. The kernel injects pre-authorized, sandboxed capabilities.
- **Human-in-the-Loop** — Sensitive operations require explicit admin approval at runtime.

## Download

Pre-built binaries for Windows, macOS, and Linux are available on the [**Releases**](https://github.com/Exiv-ai/Exiv/releases/latest) page.

## Quick Start

### From Source

```bash
git clone https://github.com/Exiv-ai/Exiv.git
cd Exiv
cargo build --release
cargo run --package exiv_core
```

The dashboard opens at **http://localhost:8081**.

### Pre-built Binary

Download the latest release from the [**Releases**](https://github.com/Exiv-ai/Exiv/releases/latest) page, then:

```bash
# Manage
exiv_system service start
exiv_system service stop
exiv_system service status

# Update
exiv_system update
```

## Plugins

**Rust plugins** (compiled, zero-overhead):

| ID | Type | Description |
|----|------|-------------|
| `mind.deepseek` | Reasoning | Advanced reasoning via DeepSeek API |
| `mind.cerebras` | Reasoning | Ultra-high-speed reasoning via Cerebras API |
| `core.ks22` | Memory | Persistent key-value memory with chronological recall |
| `core.moderator` | Reasoning | Consensus moderator for collective intelligence |
| `hal.cursor` | HAL | High-precision cursor with fluid motion trails |
| `bridge.python` | Bridge | Universal Python Bridge with async event streaming |
| `adapter.mcp` | Skill | Model Context Protocol (MCP) client adapter |
| `vision.screen` | Vision | Screen capture and analysis module |

**Python plugins** (loaded through `bridge.python`):

| ID | Type | Description |
|----|------|-------------|
| `python.analyst` | Reasoning | Data analysis agent with external data fetching |
| `python.gaze` | Vision | Webcam-based eye tracking via MediaPipe |

### Writing a Plugin

```rust
#[exiv_plugin(
    name = "my.plugin",
    kind = "Reasoning",
    description = "My custom reasoning engine.",
    version = "0.1.0",
    permissions = ["NetworkAccess"],
    capabilities = ["Reasoning"]
)]
pub struct MyPlugin { /* ... */ }
```

The `#[exiv_plugin]` proc-macro generates manifests, factory boilerplate, and capability registration at compile time.

## Project Structure

```
crates/core/        Kernel — event bus, plugin manager, HTTP API, rate limiter
crates/shared/      SDK — traits (Plugin, ReasoningEngine, Tool) and shared types
crates/macros/      Procedural macro for plugin manifest generation
plugins/            Official plugins (8 crates)
dashboard/          React/TypeScript web UI (Tauri desktop app)
scripts/            Python bridge runtime, build tools
docs/               Architecture, vision, changelog
```

## Configuration

Copy `.env.example` to `.env` to customize. All settings have sensible defaults.

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `8081` | HTTP server port |
| `EXIV_API_KEY` | (none) | Admin API key (required in release builds) |
| `DEEPSEEK_API_KEY` | (none) | DeepSeek API key |
| `CEREBRAS_API_KEY` | (none) | Cerebras API key |
| `BIND_ADDRESS` | `127.0.0.1` | Server bind address |
| `ALLOWED_HOSTS` | (none) | Network whitelist for plugin HTTP access |
| `MAX_EVENT_DEPTH` | `10` | Maximum event cascading depth |
| `RUST_LOG` | `info` | Log level filter |

<details>
<summary>All configuration variables</summary>

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `8081` | HTTP server port |
| `DATABASE_URL` | `sqlite:{exe_dir}/data/exiv_memories.db` | SQLite database path |
| `EXIV_API_KEY` | (none) | Admin API key (required in release builds) |
| `DEEPSEEK_API_KEY` | (none) | DeepSeek API key |
| `CEREBRAS_API_KEY` | (none) | Cerebras API key |
| `CONSENSUS_ENGINES` | `mind.deepseek,mind.cerebras` | Engine IDs for consensus mode |
| `DEFAULT_AGENT_ID` | `agent.exiv_default` | Default agent for `/api/chat` |
| `EXIV_SKIP_ICON_EMBED` | (none) | Set to `1` to skip icon embedding during dev builds |
| `RUST_LOG` | `info` | Log level filter |
| `MAX_EVENT_DEPTH` | `10` | Maximum event cascading depth |
| `PLUGIN_EVENT_TIMEOUT_SECS` | `30` | Plugin event handler timeout |
| `CORS_ORIGINS` | (none) | Allowed CORS origins (comma-separated) |
| `ALLOWED_HOSTS` | (none) | Network whitelist for plugin access |
| `EXIV_UPDATE_REPO` | (none) | GitHub `owner/repo` for update distribution |
| `BIND_ADDRESS` | `127.0.0.1` | Server bind address (`0.0.0.0` for network access) |
| `MEMORY_CONTEXT_LIMIT` | `10` | Maximum memory entries returned per recall |
| `EVENT_HISTORY_SIZE` | `1000` | Maximum events kept in memory |
| `EVENT_RETENTION_HOURS` | `24` | Hours to retain events before cleanup (1-720) |

</details>

## API

<details>
<summary>Public endpoints</summary>

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/metrics` | System metrics |
| GET | `/api/plugins` | Plugin list with manifests |
| GET | `/api/plugins/:id/config` | Plugin configuration |
| GET | `/api/agents` | Agent configurations |
| GET | `/api/history` | Event history |
| GET | `/api/memories` | Memory entries |
| GET | `/api/events` | SSE event stream |
| GET | `/api/permissions/pending` | Pending permission requests |
| GET | `/api/system/version` | Current version info |
| GET | `/api/system/update/check` | Check for updates |
| POST | `/api/chat` | Send message to an agent |

</details>

<details>
<summary>Admin endpoints (requires X-API-Key header)</summary>

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/system/shutdown` | Graceful shutdown |
| POST | `/api/system/update/apply` | Apply pending update |
| POST | `/api/plugins/apply` | Bulk enable/disable plugins |
| POST | `/api/plugins/:id/config` | Update plugin config |
| POST | `/api/plugins/:id/permissions/grant` | Grant permission to plugin |
| POST | `/api/agents` | Create agent |
| PUT | `/api/agents/:id` | Update agent |
| POST | `/api/events/publish` | Publish event to bus |
| POST | `/api/permissions/:id/approve` | Approve a request |
| POST | `/api/permissions/:id/deny` | Deny a request |

</details>

## Testing

165+ tests.

```bash
cargo test                              # all tests
cargo test --package exiv_core          # kernel only
cargo test --test '*'                   # integration tests only
```

## Security

- **API key authentication** with per-IP rate limiting (10 req/s, burst 20)
- **Append-only audit log** in SQLite for all permission decisions
- **Minimal default permissions** — elevated permissions require human approval
- **Network host whitelisting** with DNS rebinding protection
- **Python sandbox** with AST-level module blocking and process isolation

See [Architecture](docs/ARCHITECTURE.md) for the full security model.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — Design principles, event flow, security model
- [Project Vision](docs/PROJECT_VISION.md) — Strategic direction and roadmap
- [Development](docs/DEVELOPMENT.md) — Coding standards, guardrails, PR process
- [Changelog](docs/CHANGELOG.md) — Development history
- [Plugin Macros](crates/macros/README.md) — `#[exiv_plugin]` usage

## License

**Business Source License 1.1** — converts to **MIT** on 2028-02-14.

You can freely use Exiv for plugin development, internal tools, consulting, education, and small-scale commercial projects. Large-scale commercial deployment (>$100k revenue, >1,000 users, >50 employees, or SaaS) requires prior approval. See [LICENSE](LICENSE) for the full terms.

## Community

- [GitHub Issues](https://github.com/Exiv-ai/Exiv/issues)
- [GitHub Discussions](https://github.com/Exiv-ai/Exiv/discussions)
- [X (Twitter)](https://x.com/exiv_ai)

## Note

Built by a solo developer from Japan. Most of the code and documentation in this project was written with the assistance of AI. If you find any issues, please open an [issue](https://github.com/Exiv-ai/Exiv/issues).
