# MGP — Discovery & Intelligence

> Part of the [MGP Specification](MGP_SPEC.md) (v0.6.0-draft, 2026-03-06)
> This document covers §15-§16. For overview and architecture, see [MGP_SPEC.md](MGP_SPEC.md).

**Section Map:** §1 [MGP_SPEC.md](MGP_SPEC.md) · §2-§7 [MGP_SECURITY.md](MGP_SECURITY.md) · §11-§14 [MGP_COMMUNICATION.md](MGP_COMMUNICATION.md) · §15-§16 [MGP_DISCOVERY.md](MGP_DISCOVERY.md) · §17-§20 [MGP_GUIDE.md](MGP_GUIDE.md)

---

## 15. Discovery — Configuration + Kernel Tool Layer

### 15.1 Overview

MGP defines server discovery through **static configuration** (protocol-level concept)
and **runtime registry tools** (Layer 4 kernel tools).

### 15.2 Server Advertisement

MGP servers MAY advertise themselves via a well-known configuration file or registry
kernel tools.

#### Configuration File Discovery

Clients look for MGP servers in these locations (in order):

1. `./mcp.toml` — Project-local configuration (MGP fields in `[servers.mgp]` section)
2. `~/.config/cloto/mcp.toml` — User-level configuration
3. `$MCP_CONFIG_PATH` — Environment variable override

**Format (mcp.toml):**

```toml
[[servers]]
id = "mind.cerebras"
command = "python"
args = ["mcp-servers/cerebras/server.py"]
transport = "stdio"

[servers.mgp]
extensions = ["permissions", "tool_security", "lifecycle", "streaming"]
permissions_required = ["network.outbound"]
trust_level = "standard"
restart_policy = "on_failure"

[servers.env]
CEREBRAS_API_KEY = "${CEREBRAS_API_KEY}"
```

The `[servers.mgp]` section is OPTIONAL. If omitted, the server is treated as standard MCP.
The file format is backward compatible with MCP configuration files — the `mgp` section
is ignored by MCP-only clients.

### 15.3 Capability Advertisement

Connected servers advertise their capabilities via the `initialize` response (§2). For
pre-connection discovery, the `mcp.toml` configuration provides the same information
without establishing a transport connection.

### 15.4 Registry — Kernel Tools

For distributed environments and runtime discovery, the kernel exposes registry tools.

#### mgp.discovery.list

**Tool Name:** `mgp.discovery.list`
**Category:** Kernel Tool (Layer 4)

Query connected and registered servers.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "filter": {
      "type": "object",
      "properties": {
        "extensions": { "type": "array", "items": { "type": "string" } },
        "permissions": { "type": "array", "items": { "type": "string" } },
        "status": { "type": "string", "enum": ["connected", "disconnected", "all"] }
      }
    }
  }
}
```

**Output:**
```json
{
  "servers": [
    {
      "id": "mind.cerebras",
      "status": "connected",
      "mgp_version": "0.1.0",
      "extensions": ["permissions", "tool_security", "lifecycle", "streaming"],
      "tools": ["think", "analyze"],
      "trust_level": "standard"
    }
  ]
}
```

#### mgp.discovery.register

**Tool Name:** `mgp.discovery.register`
**Category:** Kernel Tool (Layer 4)

Register a server created at runtime (e.g., by agents).

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string", "description": "Server identifier" },
    "command": { "type": "string" },
    "args": { "type": "array", "items": { "type": "string" } },
    "transport": { "type": "string", "enum": ["stdio", "http"] },
    "mgp": {
      "type": "object",
      "properties": {
        "extensions": { "type": "array", "items": { "type": "string" } },
        "permissions_required": { "type": "array", "items": { "type": "string" } },
        "trust_level": { "type": "string", "enum": ["core", "standard", "experimental", "untrusted"] }
      }
    },
    "created_by": { "type": "string" },
    "justification": { "type": "string" }
  },
  "required": ["id", "command", "transport"]
}
```

Dynamic registrations with `trust_level: "experimental"` or `"untrusted"` are subject to stricter validation
(code safety framework, limited permissions) than `standard` or `core` servers.

#### mgp.discovery.deregister

**Tool Name:** `mgp.discovery.deregister`
**Category:** Kernel Tool (Layer 4)

Remove a dynamically registered server.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "id": { "type": "string", "description": "Server to deregister" },
    "reason": { "type": "string" }
  },
  "required": ["id"]
}
```

---

# Part III: Intelligence Layer

---

## 16. Dynamic Tool Discovery & Active Tool Request — Kernel Tool Layer

### 16.1 Overview

The most significant structural limitation of MCP is that **all tool definitions must be
injected into the LLM's context before use**. This creates two compounding problems:

1. **Context Overhead**: At ~400-500 tokens per tool, 50 tools consume 20,000-25,000 tokens.
   Large ecosystems exceed 200,000 tokens — degrading model reasoning quality and crowding
   out actual task context.

2. **Passive Tool Consumption**: The LLM can only select from tools it already knows about.
   If a tool is not in the prompt, it cannot be used — regardless of how relevant it is.
   This forces a fundamentally passive model where agents wait to be told what tools exist.

MGP solves both problems at the protocol level with two complementary mechanisms:

- **Dynamic Tool Discovery** (Mode A): The LLM searches for tools based on intent
- **Active Tool Request** (Mode B): The LLM autonomously identifies capability gaps and
  requests tools during task execution

Together, these reduce context usage by up to 99% while enabling fully autonomous tool
acquisition without explicit user instruction.

**Strategic Significance:** Dynamic Tool Discovery is MGP's primary structural
differentiator from MCP (see §1.6 Migration Policy). MCP's architecture requires all
tool schemas to be injected into the LLM context before use and has no planned mechanism
for runtime tool search or autonomous tool acquisition. This structural gap is unlikely
to be addressed by MCP in the near term because it requires a kernel/orchestrator layer
that is not part of MCP's direct client-server model.

### 16.2 Capability Declaration

```json
{
  "mgp": {
    "version": "0.6.0",
    "extensions": ["tool_discovery"]
  }
}
```

When `tool_discovery` is negotiated, the client MAY omit most tool definitions from the
LLM context. Instead, the LLM receives a single meta-tool (`mgp.tools.discover`) and
optionally a small set of pinned core tools.

### 16.3 Context Reduction Model

```
L0 (Standard MCP):  All tools in context           ~150,000 tokens
L1 (Category):      Category index only              ~5,000 tokens
L2 (Discovery):     Meta-tool + on-demand results    ~1,000 tokens
L3 (Hybrid):        Pinned tools + discovery cache   ~2,000 tokens
```

MGP clients SHOULD implement L3 (Hybrid) for optimal balance between performance and
autonomous capability. L2 is the minimum for `tool_discovery` compliance.

### 16.4 Tool Index

The kernel maintains a searchable index of all tools across all connected servers. The index
contains:

```json
{
  "tool_id": "filesystem.read_file",
  "server_id": "tool.terminal",
  "name": "read_file",
  "description": "Read the contents of a file at the given path",
  "categories": ["filesystem", "read"],
  "keywords": ["file", "read", "open", "content", "text"],
  "security": {
    "risk_level": "moderate",
    "permissions_required": ["filesystem.read"]
  },
  "embedding": [0.012, -0.034, ...]   // OPTIONAL — see below
}
```

The index supports three search strategies:

| Strategy | Method | Best For |
|----------|--------|----------|
| Keyword | Exact and fuzzy keyword matching | Precise tool names |
| Semantic | Embedding vector similarity | Natural language intent |
| Category | Hierarchical category filtering | Browsing available capabilities |

#### Semantic Search is Optional

The `embedding` field in the Tool Index is **OPTIONAL**. Implementations that do not
provide embedding vectors cannot use the `semantic` search strategy, but this does not
affect protocol compliance.

**Keyword + Category search alone is sufficient for `tool_discovery` extension
compliance.** A conforming implementation MUST support at least keyword search and
category filtering. Semantic search is an enhancement for improved natural-language
matching but is not required.

For implementations that want semantic search without running a local embedding model:

| Approach | Description |
|----------|-------------|
| **Server-side** | Each MGP server generates embeddings for its own tools and includes them in `tools/list` responses |
| **Dedicated service** | An Embedding MGP server (e.g., `tool.embedding`) generates embeddings on demand via a tool call |
| **Pre-computed** | Embeddings are computed at build/deploy time and stored in the tool index configuration |
| **None** | Keyword + category search only. No embedding model required. |

When `strategy: "semantic"` is requested but the kernel has no embeddings available,
it SHOULD fall back to keyword search and include `"fallback_strategy": "keyword"` in
the response metadata.

#### Implementation Status & Design Decisions

**Current implementation (v0.6.0):**

| Strategy | Status | Implementation |
|----------|--------|----------------|
| Keyword | Implemented | String matching with relevance scoring (name 1.0, description 0.5, keywords 0.3, category 0.2) |
| Category | Implemented | Server prefix and tool name prefix extraction |
| Semantic | Not implemented | Falls back to keyword with `"fallback_strategy": "keyword"` |

**Measured context reduction (v0.6.0, keyword-only):**

| Scenario | Total tools | Full injection | Session cache | Reduction |
|----------|-------------|----------------|---------------|-----------|
| Typical task (8 tools needed) | 41 (11 servers) | 8,405 tokens | 1,709 tokens | **79.7%** |
| Heavy task (20 tools needed) | 60 (6 servers) | 12,744 tokens | 4,248 tokens | **66.7%** |

**Semantic search design (future):**

The kernel MUST NOT embed AI models internally (§1.1 Core Minimalism: *"The Kernel is
the stage, not the actor"*). When semantic search is implemented, it will use the
`tool.embedding` MCP server as a dedicated service:

```
tool.embedding connected:
  ├─ Agent-facing: embed_text, embed_batch, similarity_search (general purpose)
  └─ Kernel-facing: semantic search precision enhancement (optional optimization)

tool.embedding disconnected:
  ├─ Agent-facing: unavailable
  └─ Kernel-facing: keyword fallback (degraded but functional)
```

Key architectural constraints:
- `tool.embedding` serves dual roles (agent tool + kernel optimization) but the kernel
  MUST NOT depend on it. All kernel functions must operate without `tool.embedding`.
- Semantic search improves **selection quality** (finding the right tools), not
  **context reduction rate** (which is controlled by the session cache token budget).
- Adding semantic search increases discovery latency (<1ms keyword → 50-200ms embedding)
  but this is negligible relative to LLM inference time (2,000-15,000ms).
- Implementation priority is low while total tool count remains under ~100, as keyword +
  category search provides sufficient precision at this scale.

### 16.5 Mode A: Dynamic Tool Discovery

The LLM searches for tools based on a natural language description of what it needs.
This is the **user-intent-driven** mode — the LLM translates the user's request into a
tool search.

#### mgp.tools.discover

**Tool Name:** `mgp.tools.discover`
**Category:** Kernel Tool (Layer 4)

Search for tools based on natural language description.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Natural language description of needed capability" },
    "strategy": { "type": "string", "enum": ["keyword", "semantic", "category"], "default": "keyword" },
    "max_results": { "type": "number", "default": 5 },
    "filter": {
      "type": "object",
      "properties": {
        "categories": { "type": "array", "items": { "type": "string" } },
        "risk_level_max": { "type": "string", "enum": ["safe", "moderate", "dangerous"] },
        "status": { "type": "string", "enum": ["connected", "all"] }
      }
    }
  },
  "required": ["query"]
}
```

**Output:**
```json
{
  "tools": [
    {
      "name": "read_file",
      "server_id": "tool.terminal",
      "description": "Read the contents of a file at the given path",
      "relevance_score": 0.95,
      "inputSchema": {
        "type": "object",
        "properties": {
          "path": { "type": "string", "description": "File path to read" }
        },
        "required": ["path"]
      },
      "security": {
        "risk_level": "moderate",
        "permissions_required": ["filesystem.read"],
        "validator": "sandbox"
      }
    },
    {
      "name": "grep",
      "server_id": "tool.terminal",
      "description": "Search file contents using pattern matching",
      "relevance_score": 0.72,
      "inputSchema": { "..." : "..." },
      "security": { "..." : "..." }
    }
  ],
  "total_available": 47,
  "search_strategy": "keyword",
  "query_time_ms": 12
}
```

The response includes **full tool schemas** for the top results, allowing the LLM to
immediately call any discovered tool without a second round trip.

#### Flow Diagram

```
User: "このファイルの中身を見せて"
  │
  ▼
LLM context: [mgp.tools.discover meta-tool] + [user message]
  │
  ▼ LLM decides it needs file-reading capability
  │
  ▼ tools/call → mgp.tools.discover({ query: "read file contents" })
  │
  ▼ Kernel searches tool index
  │
  ▼ Returns: read_file (0.95), grep (0.72), cat (0.68)
  │
  ▼ LLM selects read_file, calls it with { path: "..." }
  │
  ▼ Result returned to user
```

### 16.6 Mode B: Active Tool Request

The LLM autonomously detects a capability gap **during task execution** and requests new
tools without user intervention. This is the **agent-autonomy-driven** mode.

Unlike Mode A (which responds to user intent), Mode B enables proactive behavior:
the agent recognizes "I cannot complete this step with my current tools" and initiates
tool acquisition independently.

#### mgp.tools.request

**Tool Name:** `mgp.tools.request`
**Category:** Kernel Tool (Layer 4)

Request tools to fill a capability gap during task execution.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "reason": { "type": "string", "enum": ["capability_gap", "performance", "preference"] },
    "context": { "type": "string", "description": "Why the tool is needed" },
    "requirements": {
      "type": "object",
      "properties": {
        "capabilities": { "type": "array", "items": { "type": "string" } },
        "input_types": { "type": "array", "items": { "type": "string" } },
        "output_types": { "type": "array", "items": { "type": "string" } },
        "preferred_risk_level": { "type": "string", "enum": ["safe", "moderate", "dangerous"] }
      }
    },
    "task_trace_id": { "type": "string", "description": "Trace ID for audit" }
  },
  "required": ["reason", "context", "requirements"]
}
```

**Output:**
```json
{
    "status": "fulfilled",
    "tools_loaded": [
      {
        "name": "analyze_csv",
        "server_id": "tool.data_processing",
        "description": "Compute statistics on CSV data",
        "inputSchema": { "..." : "..." },
        "security": {
          "risk_level": "safe",
          "permissions_required": ["memory.read"]
        }
      }
    ],
    "tools_unavailable": [],
    "session_tools_count": 4,
    "context_tokens_added": 380
  }
}
```

**Request Status Values:**

| Status | Meaning |
|--------|---------|
| `fulfilled` | Matching tools found and loaded into session |
| `partial` | Some requirements met, others unavailable |
| `unavailable` | No matching tools found |
| `pending_approval` | Tools found but require permission approval (§3) |
| `creating` | No existing tools match — tool creation initiated if enabled (§7) |

#### The `creating` Status — Autonomous Tool Generation

**Default: DISABLED.** The `creating` status is only available when explicitly opted
in during capability negotiation:

```json
{
  "mgp": {
    "version": "0.6.0",
    "extensions": ["tool_discovery"],
    "tool_creation": { "enabled": true }
  }
}
```

When `status: "creating"` is returned, the kernel has determined that no existing tool
satisfies the requirement and tool creation is enabled. The following safety guardrails
apply:

##### Safety Guardrails

1. **Opt-in required**: The client MUST declare `tool_creation: { enabled: true }` in
   the capability negotiation (§2). Without this, `creating` status is never returned.

2. **Ephemeral by default**: Generated tools are **session-scoped** and automatically
   deregistered when the session ends. They are NOT persisted to the tool index.

3. **Trust level**: Generated tools always receive `trust_level: "experimental"`.
   They MUST NOT inherit the trust level of the requesting agent or server.

4. **Code safety validation**: All generated tool code MUST pass Code Safety Framework
   (§7) validation at the `strict` level before registration.

5. **Approval policy applies**: Under `interactive` policy, the operator MUST approve
   the generated tool before it becomes available. Under `auto_approve`, the tool is
   registered immediately after passing safety validation.

6. **Audit trail**: A `TOOL_CREATED_DYNAMIC` audit event (§6.4) MUST be emitted for
   every dynamically generated tool, including the generating agent, tool code hash,
   and safety validation result.

##### Flow

When all guardrails pass, the kernel:

1. Instructs the agent to generate tool code via the Code Safety Framework (§7)
2. Validates the code at `strict` safety level
3. Registers the tool via Dynamic Registration (§15.4) as ephemeral
4. Emits `TOOL_CREATED_DYNAMIC` audit event
5. Returns the newly created tool in a follow-up response

This closes the loop: **discover → request → create → use** — autonomous tool
lifecycle with mandatory safety controls.

#### Flow Diagram

```
LLM executing multi-step task
  │
  ├─ Step 1: Read CSV file ✓ (tool available)
  │
  ├─ Step 2: Parse data ✓ (tool available)
  │
  ├─ Step 3: Statistical analysis ✗ (no tool available)
  │    │
  │    ▼ LLM detects capability gap
  │    │
  │    ▼ tools/call → mgp.tools.request({
  │    │     reason: "capability_gap",
  │    │     context: "need statistical analysis",
  │    │     requirements: { capabilities: ["statistics"] }
  │    │  })
  │    │
  │    ▼ Kernel searches → finds tool.data_processing
  │    │
  │    ▼ Returns analyze_csv tool with full schema
  │    │
  │    ▼ LLM calls analyze_csv ✓
  │
  ├─ Step 4: Generate report ✓
  │
  ▼ Task complete
```

### 16.7 Session Tool Cache

To avoid repeated discovery calls, the kernel maintains a per-session tool cache:

| Category | Behavior |
|----------|----------|
| **Pinned tools** | Always in context (configured per agent, e.g., `think`, `store`) |
| **Session cache** | Tools used in current session — retained until session ends |
| **Discovery results** | Cached for the duration of the request — discarded after use |

#### mgp.tools.session

**Tool Name:** `mgp.tools.session`
**Category:** Kernel Tool (Layer 4)

Query the current session's loaded tools.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {}
}
```

**Output:**
```json
{
  "pinned": ["think", "store", "recall"],
  "cached": ["read_file", "analyze_csv"],
  "total_tokens": 2100,
  "max_tokens": 8000
}
```

#### mgp.tools.session.evict

**Tool Name:** `mgp.tools.session.evict`
**Category:** Kernel Tool (Layer 4)

Remove tools from the session cache to free context space.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "tools": { "type": "array", "items": { "type": "string" }, "description": "Tool names to evict" },
    "reason": { "type": "string" }
  },
  "required": ["tools"]
}
```

### 16.8 Context Budget

The kernel enforces a **context budget** for tool definitions:

```json
{
  "tool_context_budget": {
    "max_tokens": 8000,
    "pinned_reserve": 2000,
    "discovery_reserve": 3000,
    "cache_limit": 3000
  }
}
```

When the budget is exceeded, the kernel automatically evicts the least-recently-used
cached tools. Pinned tools are never evicted. Discovery results that would exceed the
budget are truncated (fewer results returned).

#### Kernel Tool Visibility

Layer 4 Kernel Tools (`mgp.access.*`, `mgp.health.*`, `mgp.events.*`, etc.) are
management tools intended for operators and administrative agents. See §1.6.3 for
the complete namespace and visibility specification. Summary:

| Kernel Tool Category | `tools/list` | LLM Context | Rationale |
|---------------------|-------------|------------|-----------|
| `mgp.tools.discover` | Yes | Yes (as meta-tool) | LLM needs this for dynamic discovery |
| `mgp.tools.request` | Yes | Yes (as meta-tool) | LLM needs this for active tool request |
| `mgp.access.*` | Yes | No | Administrative — operator/API only |
| `mgp.health.*` | Yes | No | Administrative — monitoring only |
| `mgp.lifecycle.*` | Yes | No | Administrative — operator only |
| `mgp.events.*` | Yes | No | Administrative — subscription mgmt |
| `mgp.discovery.*` | Yes | No | Administrative — server registration |
| `mgp.tools.session*` | Yes | Optional | Context management — LLM MAY use |

Kernel tools appear in `tools/list` responses (for API discoverability) but are excluded
from the LLM context budget unless explicitly pinned. Only `mgp.tools.discover` and
`mgp.tools.request` are injected into the LLM context as meta-tools.

### 16.9 Comparison with Existing Approaches

| Approach | Discovery | Multi-Step | Protocol Standard | Context Reduction |
|----------|-----------|------------|-------------------|-------------------|
| Standard MCP | None (all tools injected) | N/A | Yes | 0% |
| RAG-MCP | Pre-query semantic retrieval | No | No | ~80% |
| MCP-Zero (paper) | Active tool request | Yes | No (research) | ~95% |
| Cursor/Copilot | Hard limits (40/128 tools) | No | No | Truncation |
| **MGP §16** | **A + B combined** | **Yes** | **Yes** | **~99%** |

MGP is the first protocol to standardize both passive discovery (A) and active request (B)
as first-class protocol methods, with session management and context budgeting built in.

---

