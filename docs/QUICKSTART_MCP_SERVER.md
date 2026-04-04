# Build Your First MCP/MGP Server for ClotoCore

This guide walks you through creating a plugin server for ClotoCore.
You have two paths: **standard MCP** (quick start) or **MGP** (recommended, deeper integration).

> **Key insight:** ClotoCore implements MGP, a strict superset of MCP.
> Any standard MCP server works out of the box. MGP adds security declarations,
> trust levels, and bidirectional communication — all opt-in.

---

## Path A: Standard MCP Server (5 minutes)

Use Anthropic's official tools. Your server will run inside ClotoCore with full
process isolation, RBAC, and audit logging — no extra code needed.

### 1. Scaffold

```bash
# Option 1: Anthropic's official template
uvx create-mcp-server my-server

# Option 2: Manual setup with FastMCP
mkdir my-server && cd my-server
pip install mcp
```

### 2. Write your server

```python
# my-server/server.py
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool
import asyncio, json

app = Server("my-server")

@app.list_tools()
async def list_tools():
    return [
        Tool(
            name="greet",
            description="Return a greeting for the given name",
            inputSchema={
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Name to greet"}
                },
                "required": ["name"],
            },
        )
    ]

@app.call_tool()
async def call_tool(name: str, arguments: dict):
    if name == "greet":
        return [TextContent(type="text", text=f"Hello, {arguments['name']}!")]
    raise ValueError(f"Unknown tool: {name}")

async def main():
    async with stdio_server() as (read, write):
        await app.run(read, write, app.create_initialization_options())

if __name__ == "__main__":
    asyncio.run(main())
```

### 3. Register in ClotoCore

Add to your `mcp.toml`:

```toml
[[servers]]
id = "tool.my-server"
display_name = "My Server"
command = "python"
args = ["path/to/my-server/server.py"]
transport = "stdio"
```

### 4. Done

Restart ClotoCore. Your server appears in the dashboard. Tools are available
to all agents (subject to RBAC policy).

**What ClotoCore provides automatically:**
- Process isolation (separate OS process)
- Per-agent RBAC (via `mcp_access_control` table)
- Audit logging (every tool call recorded)
- Timeout guard (configurable per-server)
- Health monitoring and auto-restart

---

## Path B: MGP Server (Recommended, +10 minutes)

MGP adds security declarations to the MCP `initialize` response. This unlocks
ClotoCore's permission flow, trust-based isolation, and event system.

### What you get

| Feature | MCP (automatic) | MGP (with declarations) |
|---------|-----------------|------------------------|
| Process isolation | Yes | Yes |
| RBAC | Yes | Yes |
| Audit log | Yes | Yes |
| Permission approval flow | — | Yes (HITL gate) |
| Trust-based isolation | Untrusted (max restriction) | Appropriate level |
| Event subscription | — | Yes |
| Streaming chunks | — | Yes |

### 1. Add MGP capabilities to initialize

The only change is in your server's `initialize` response — add the `mgp` object
to `capabilities`:

```python
# In your initialize handler, return:
{
    "protocolVersion": "2024-11-05",
    "capabilities": {
        "tools": {},
        "mgp": {
            "version": "0.6.0",
            "extensions": ["permissions"],
            "permissions_required": ["network.outbound"]
        }
    },
    "serverInfo": {
        "name": "my-server",
        "version": "0.1.0"
    }
}
```

### 2. MGP fields explained

| Field | Required | Description |
|-------|----------|-------------|
| `version` | Yes | MGP protocol version. Use `"0.6.0"`. |
| `extensions` | Yes | Extensions your server supports. Start with `["permissions"]`. |
| `permissions_required` | No | Permissions your server needs. The kernel gates startup on approval. |
| `trust_level` | No | Self-declared trust level (informational — kernel config overrides). |
| `server_id` | No | Unique identifier. Defaults to the `id` in `mcp.toml`. |

### 3. Common permission types

| Permission | When to declare |
|-----------|----------------|
| `network.outbound` | Your server makes HTTP requests to external APIs |
| `filesystem.read` | Your server reads files outside its sandbox |
| `filesystem.write` | Your server writes files outside its sandbox |
| `shell` | Your server executes shell commands |

### 4. Configure trust level in mcp.toml

```toml
[[servers]]
id = "tool.my-server"
display_name = "My Server"
command = "python"
args = ["path/to/my-server/server.py"]
transport = "stdio"

[servers.mgp]
trust_level = "standard"  # core | standard | experimental | untrusted
```

Trust level determines the default isolation profile:

| Trust Level | Filesystem | Network | Memory Limit | Max Processes |
|-------------|-----------|---------|-------------|--------------|
| `core` | Unrestricted | Unrestricted | None | None |
| `standard` | Sandbox | ProxyOnly | 512MB | 5 |
| `experimental` | Sandbox | ProxyOnly | 256MB | 2 |
| `untrusted` | Sandbox | None | 128MB | 0 |

### 5. Using MgpCapabilities helper (optional)

The `cloto-mcp-servers` repository provides a lightweight `MgpCapabilities` builder
for declaring MGP capabilities without manually constructing JSON:

```python
from common.mgp_utils import MgpCapabilities

mgp = MgpCapabilities()
mgp.require_permission("network.outbound")
mgp.set_trust_level("standard")

# In your initialize handler, merge into capabilities:
capabilities = {"tools": {}, **mgp.as_dict()}
```

> **Note:** A full MGP SDK is not yet available. For advanced MGP features
> (events, streaming, callbacks), implement the JSON-RPC methods directly.
> See [MGP Guide](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_GUIDE.md)
> for the staged adoption path.

### 6. Using ClotoCore's ToolRegistry (optional)

The `cloto-mcp-servers` repository provides a `ToolRegistry` helper that
eliminates boilerplate:

```python
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from common.mcp_utils import ToolRegistry, run_mcp_server
import asyncio

registry = ToolRegistry("my-server")

@registry.tool(
    name="greet",
    description="Return a greeting",
    schema={
        "type": "object",
        "properties": {"name": {"type": "string"}},
        "required": ["name"],
    },
)
async def greet(arguments: dict) -> dict:
    return {"greeting": f"Hello, {arguments['name']}!"}

if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
```

---

## Adding to cloto-mcp-servers

If you want to contribute your server to the official repository:

1. Create `servers/<name>/server.py` (use `ToolRegistry` from `common/mcp_utils.py`)
2. Add `servers/<name>/pyproject.toml`
3. Add tests to `servers/tests/`
4. Register in `registry.json`
5. Add server entry to ClotoCore's `mcp.toml`

See `servers/example/server.py` for a complete reference implementation.

---

## Further Reading

- [MGP Specification](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_SPEC.md) — Full protocol spec
- [MGP Security](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_SECURITY.md) — Permission model, RBAC, audit
- [MGP Guide](https://github.com/Cloto-dev/cloto-mcp-servers/blob/main/docs/MGP_GUIDE.md) — Implementation guide with staged adoption
- [MCP Official Docs](https://modelcontextprotocol.io/docs/develop/build-server) — Anthropic's MCP documentation
- [ClotoCore Architecture](ARCHITECTURE.md) — System design and security framework
