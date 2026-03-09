# MCP Server Auto-Registration Design

**Status**: Planned (not yet implemented)
**Created**: 2026-03-07

## Problem

ClotoCore migrations grant MCP access permissions (`server_grant` entries) to the
default agent (`agent.cloto_default`) for bundled servers such as `memory.cpersona`,
`tool.terminal`, etc. However, the MCP servers themselves must be manually registered
in the `mcp_servers` table. Without server registration, access permissions are
meaningless.

This also affects agent config import: if a referenced MCP server is not registered
on the target system, the import skips the access entry with a warning.

## Goal

Automatically register MCP servers from **trusted sources** without manual intervention.

## Trusted Source Model

| Source | Auto-Registration | Description |
|--------|-------------------|-------------|
| `bundled` | Always | Servers shipped with ClotoCore (`mcp-servers/`) |
| `marketplace` | Always (future) | Servers downloaded from ClotoCore Marketplace |
| `user` | Manual only | Custom servers added by the user |

### Agent Config Import Behavior

When importing an agent config (`.cloto-agent.json`):

- If a referenced server is `bundled` or `marketplace` → auto-register if missing
- If a referenced server is `user` or unknown → skip with warning (current behavior)

## Design Considerations

### 1. Bundled Server Registry

A mechanism to declare which MCP servers are bundled with ClotoCore and their
startup configurations (command, args, env).

Options:
- **Rust constant/config**: Hardcode bundled server definitions in the kernel
- **Manifest file**: A `bundled-servers.json` or similar declarative file

### 2. Registration Timing

When should auto-registration occur?

- **Kernel startup**: Check on every boot, register missing bundled servers
- **Initial setup**: Register during first-run setup wizard only
- **Migration**: One-time SQL migration (current approach for access grants)

### 3. Database Schema Extension

Add a `source` column to `mcp_servers` to track provenance:

```sql
ALTER TABLE mcp_servers ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
-- Values: 'bundled', 'marketplace', 'user'
```

This enables:
- UI differentiation (bundled servers shown differently)
- Import logic to check if a server is auto-registerable
- Preventing user deletion of bundled servers (or warning)

### 4. Marketplace Integration (Future)

When the ClotoCore Marketplace is implemented:

- Downloaded server packages include a manifest with command/args/description
- Installed packages are registered with `source = 'marketplace'`
- Agent config import can auto-register marketplace servers if the package is
  installed locally
- Marketplace packages may include a signature for trust verification

## Bundled Servers (Current)

Servers in `mcp-servers/` that should be auto-registered:

| Server ID | Directory | Description |
|-----------|-----------|-------------|
| `memory.cpersona` | `mcp-servers/cpersona/` | Memory backend |
| `tool.terminal` | `mcp-servers/terminal/` | Terminal access |
| `mind.cerebras` | `mcp-servers/cerebras/` | Cerebras LLM engine |
| `mind.deepseek` | `mcp-servers/deepseek/` | DeepSeek LLM engine |
| `tool.embedding` | `mcp-servers/embedding/` | Embedding service |

Additional servers referenced in migrations but not in `mcp-servers/`:

| Server ID | Notes |
|-----------|-------|
| `tool.cron` | Cron job management (built-in?) |
| `tool.websearch` | Web search |
| `tool.research` | Research tool |
| `tool.agent_utils` | Agent utilities |

> These need to be verified: are they built-in kernel features, external
> dependencies, or planned servers?

## Export/Import Integration

### Current Export Format

```json
{
  "cloto_agent_export": 1,
  "mcp_access": [
    { "server_id": "tool.terminal", "permission": "allow" }
  ]
}
```

### Future Export Format (with auto-registration support)

```json
{
  "cloto_agent_export": 2,
  "mcp_access": [
    {
      "server_id": "tool.terminal",
      "permission": "allow",
      "source": "bundled"
    }
  ]
}
```

The `source` field allows the import logic to decide whether auto-registration
is permitted without including potentially dangerous command/args in the export file.
