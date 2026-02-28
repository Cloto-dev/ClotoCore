# MGP Application Patterns

**Companion to:** `MGP_SPEC.md`
**Date:** 2026-02-28

This document describes reference architectures for building higher-level capabilities
on top of MGP protocol primitives. These patterns are **not part of the MGP specification** —
they are implementation guidance showing how to compose MGP's standard methods into
powerful application-level features.

Each pattern can be implemented as one or more MGP servers without protocol modifications.

---

## 1. Multi-Agent Coordination

### 1.1 Problem

Multiple agents need to collaborate — delegating tasks, sharing results, and coordinating
work — but MGP defines no agent-to-agent messaging protocol.

### 1.2 Why This Is Not a Protocol Extension

Agent coordination is fully expressible as MGP tool calls through the kernel:

- The kernel already routes tool calls to the correct server
- Access control (§5) already governs which agents can call which tools
- Tool discovery (§16) already enables agents to find capabilities dynamically

Adding protocol-level agent messaging would duplicate existing primitives and violate
MGP's design principle of core minimalism.

### 1.3 Architecture

```
┌─────────────┐     ┌─────────────────────────┐     ┌─────────────┐
│   Agent A   │────>│   MGP Kernel            │────>│   Agent B   │
│             │     │                         │     │             │
│  tools/call │     │  ┌───────────────────┐  │     │  think()    │
│  delegate() │────>│  │  Coordinator      │  │────>│  store()    │
│             │     │  │  MGP Server       │  │     │  recall()   │
│  discover() │     │  │                   │  │     │             │
│             │     │  │  - delegate_task  │  │     │             │
└─────────────┘     │  │  - query_agents   │  │     └─────────────┘
                    │  │  - collect_results│  │
                    │  └───────────────────┘  │
                    └─────────────────────────┘
```

The Coordinator is a standard MGP server that exposes coordination tools.

### 1.4 Coordinator Server Tools

#### delegate_task

Delegate a task to another agent and wait for the result.

```json
{
  "name": "delegate_task",
  "description": "Delegate a task to another agent by capability match",
  "inputSchema": {
    "type": "object",
    "properties": {
      "task": {
        "type": "string",
        "description": "Natural language description of the task"
      },
      "target_agent": {
        "type": "string",
        "description": "Specific agent ID, or null for capability-based routing"
      },
      "required_capabilities": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Capabilities the target agent must have"
      },
      "timeout_ms": {
        "type": "number",
        "description": "Maximum wait time for result"
      },
      "priority": {
        "type": "string",
        "enum": ["low", "normal", "high", "critical"]
      }
    },
    "required": ["task"]
  },
  "security": {
    "risk_level": "moderate",
    "permissions_required": ["memory.write"],
    "side_effects": ["process"]
  }
}
```

**Example call:**
```json
{
  "task": "Analyze the sentiment of these 500 customer reviews",
  "required_capabilities": ["data_analysis", "nlp"],
  "timeout_ms": 60000,
  "priority": "normal"
}
```

**Example response:**
```json
{
  "status": "completed",
  "delegated_to": "agent.analyst",
  "result": {
    "positive": 312,
    "neutral": 108,
    "negative": 80,
    "summary": "Overall positive sentiment (62.4%)"
  },
  "duration_ms": 12400
}
```

#### query_agents

Query available agents and their capabilities.

```json
{
  "name": "query_agents",
  "inputSchema": {
    "type": "object",
    "properties": {
      "filter": {
        "type": "object",
        "properties": {
          "capabilities": { "type": "array", "items": { "type": "string" } },
          "status": { "type": "string", "enum": ["enabled", "disabled", "all"] },
          "type": { "type": "string", "enum": ["ai", "container", "all"] }
        }
      }
    }
  }
}
```

**Example response:**
```json
{
  "agents": [
    {
      "id": "agent.cloto_default",
      "name": "Cloto Assistant",
      "type": "ai",
      "enabled": true,
      "capabilities": ["reasoning", "memory", "tool_use"],
      "engine": "mind.cerebras",
      "current_load": 0
    },
    {
      "id": "agent.analyst",
      "name": "Data Analyst",
      "type": "ai",
      "enabled": true,
      "capabilities": ["data_analysis", "nlp", "statistics"],
      "engine": "mind.deepseek",
      "current_load": 2
    }
  ]
}
```

#### collect_results

Collect results from multiple delegated tasks.

```json
{
  "name": "collect_results",
  "inputSchema": {
    "type": "object",
    "properties": {
      "task_ids": {
        "type": "array",
        "items": { "type": "string" }
      },
      "wait_strategy": {
        "type": "string",
        "enum": ["all", "any", "majority"],
        "description": "all=wait for all, any=return first, majority=wait for >50%"
      },
      "timeout_ms": { "type": "number" }
    },
    "required": ["task_ids"]
  }
}
```

### 1.5 Coordination Patterns

#### Fan-Out / Fan-In

```
Agent A: "Summarize these 3 documents"
  │
  ├─ delegate_task(doc1) → Agent B
  ├─ delegate_task(doc2) → Agent C
  ├─ delegate_task(doc3) → Agent B  (load balancing)
  │
  ▼ collect_results([task1, task2, task3], wait_strategy: "all")
  │
  ▼ Agent A combines summaries into final report
```

#### Chain

```
Agent A: "Translate this, then summarize"
  │
  ├─ delegate_task("translate to English") → Agent B (translator)
  │    └─ result: translated text
  │
  ├─ delegate_task("summarize this text", input: translated) → Agent C (summarizer)
  │    └─ result: summary
  │
  ▼ Agent A presents final result
```

#### Specialist Routing

```
Agent A receives user request
  │
  ├─ query_agents(capabilities: ["code_review"])
  │    └─ found: Agent D (code specialist)
  │
  ├─ delegate_task("review this PR") → Agent D
  │    └─ result: review comments
  │
  ▼ Agent A presents review to user
```

### 1.6 MGP Primitives Used

| Primitive | Usage |
|-----------|-------|
| Tool calls (MCP base) | Calling coordinator tools |
| Access Control (§5) | Controlling which agents can delegate to which |
| Tool Discovery (§16) | Finding the coordinator server and its tools |
| Audit Trail (§6) | Logging delegation events |
| Streaming (§12) | Streaming progress of long-running delegated tasks |

---

## 2. Context Management

### 2.1 Problem

Even with tool discovery (§16) reducing tool definition overhead, conversations accumulate
context from chat history, file contents, intermediate results, and tool outputs. Without
management, this fills the context window and degrades performance.

### 2.2 Why This Is Not a Protocol Extension

Context management is a client/kernel responsibility that operates on data, not protocol
messages. It can be implemented as:

- A summarization MGP server (LLM-based text compression)
- A memory MGP server (long-term storage and retrieval)
- Kernel-side logic using the context budget system from §16.8

### 2.3 Architecture

```
┌─────────────────────────────────────────────────┐
│                  MGP Kernel                     │
│                                                 │
│  ┌──────────────────────────────────────────┐   │
│  │          Context Manager                  │   │
│  │                                           │   │
│  │  ┌─────────┐  ┌──────────┐  ┌─────────┐ │   │
│  │  │ Active  │  │ Summary  │  │ Evicted │ │   │
│  │  │ Context │  │ Buffer   │  │ Archive │ │   │
│  │  │         │  │          │  │         │ │   │
│  │  │ Recent  │  │ Older    │  │ Old     │ │   │
│  │  │ messages│  │ messages │  │ messages│ │   │
│  │  │ + tools │  │ (summar- │  │ (in     │ │   │
│  │  │ + files │  │  ized)   │  │ memory  │ │   │
│  │  │         │  │          │  │ server) │ │   │
│  │  └─────────┘  └──────────┘  └─────────┘ │   │
│  └──────────────────────────────────────────┘   │
│         │                │               │       │
│         ▼                ▼               ▼       │
│  ┌────────────┐  ┌─────────────┐  ┌──────────┐ │
│  │ LLM Engine │  │ Summarizer  │  │ Memory   │ │
│  │ MGP Server │  │ MGP Server  │  │ MGP Srvr │ │
│  └────────────┘  └─────────────┘  └──────────┘ │
└─────────────────────────────────────────────────┘
```

### 2.4 Context Tiers

| Tier | Content | Token Budget | Eviction |
|------|---------|-------------|----------|
| **Active** | Current turn messages, active tool schemas, recent results | 60% of window | Never (current turn) |
| **Summary** | Compressed older messages, conversation summary | 25% of window | Re-summarize when full |
| **Archive** | Full history stored in memory server (KS22 etc.) | 0% (external) | Never (persistent) |

### 2.5 Summarizer Server Tools

#### summarize

Compress text while preserving key information.

```json
{
  "name": "summarize",
  "inputSchema": {
    "type": "object",
    "properties": {
      "text": { "type": "string" },
      "max_tokens": { "type": "number" },
      "preserve": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Key topics/entities to preserve in summary"
      },
      "style": {
        "type": "string",
        "enum": ["bullets", "narrative", "structured"],
        "description": "Output format"
      }
    },
    "required": ["text", "max_tokens"]
  }
}
```

#### extract_key_facts

Extract structured facts for long-term storage.

```json
{
  "name": "extract_key_facts",
  "inputSchema": {
    "type": "object",
    "properties": {
      "text": { "type": "string" },
      "categories": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Fact categories: decisions, requirements, preferences, errors"
      }
    },
    "required": ["text"]
  }
}
```

**Example response:**
```json
{
  "facts": [
    { "category": "decision", "content": "Using MGP instead of extending MCP", "confidence": 0.95 },
    { "category": "requirement", "content": "Must maintain MCP backward compatibility", "confidence": 0.99 },
    { "category": "preference", "content": "User prefers Japanese for discussions", "confidence": 0.90 }
  ]
}
```

### 2.6 Kernel-Side Context Management Flow

```
New message arrives
  │
  ├─ Active context + new message > 60% of window?
  │    │
  │    Yes ──► Move oldest active messages to Summary tier
  │           │
  │           ├─ tools/call → summarize(oldest_messages, max_tokens: 500)
  │           │
  │           ├─ tools/call → extract_key_facts(oldest_messages)
  │           │    └─ tools/call → store(facts)  → Memory server
  │           │
  │           └─ Replace full messages with summary in context
  │
  ├─ Summary tier > 25% of window?
  │    │
  │    Yes ──► Re-summarize (compress summary of summaries)
  │           └─ Archive original summary to memory server
  │
  └─ Proceed with LLM call using managed context
```

### 2.7 MGP Primitives Used

| Primitive | Usage |
|-----------|-------|
| Tool calls (MCP base) | Calling summarizer and memory tools |
| Context Budget (§16.8) | Enforcing token limits per tier |
| Tool Discovery (§16) | Finding summarizer/memory servers |
| Lifecycle (§11) | Health checks on critical context services |

---

## 3. Federation

### 3.1 Problem

Multiple MGP-compatible systems (ClotoCore instances, third-party implementations) need to
share servers and tools across network boundaries.

### 3.2 Why This Is Not a Protocol Extension

A federation proxy is just another MGP server to the local kernel. It forwards tool calls
to remote instances over HTTP/WebSocket, but from the kernel's perspective, it looks like
any other connected server.

### 3.3 Architecture

```
┌──────────────────┐           ┌──────────────────┐
│  Instance A      │           │  Instance B      │
│                  │           │                  │
│  ┌────────────┐  │  HTTPS    │  ┌────────────┐  │
│  │ Federation │  │◄────────►│  │ Federation │  │
│  │ Proxy      │  │           │  │ Proxy      │  │
│  │ MGP Server │  │           │  │ MGP Server │  │
│  └─────┬──────┘  │           │  └─────┬──────┘  │
│        │         │           │        │         │
│  ┌─────▼──────┐  │           │  ┌─────▼──────┐  │
│  │   Kernel   │  │           │  │   Kernel   │  │
│  └────────────┘  │           │  └────────────┘  │
│                  │           │                  │
│  Local Servers:  │           │  Local Servers:  │
│  - mind.cerebras │           │  - mind.deepseek │
│  - memory.ks22   │           │  - tool.browser  │
│  - tool.terminal │           │  - tool.database │
└──────────────────┘           └──────────────────┘

Instance A can call mind.deepseek (on B) via federation proxy.
Instance B can call memory.ks22 (on A) via federation proxy.
```

### 3.4 Federation Proxy Server Tools

#### federated_call

Call a tool on a remote instance.

```json
{
  "name": "federated_call",
  "inputSchema": {
    "type": "object",
    "properties": {
      "instance": { "type": "string", "description": "Remote instance URL or ID" },
      "server_id": { "type": "string" },
      "tool_name": { "type": "string" },
      "arguments": { "type": "object" }
    },
    "required": ["instance", "server_id", "tool_name", "arguments"]
  },
  "security": {
    "risk_level": "dangerous",
    "permissions_required": ["network.outbound"],
    "side_effects": ["network"]
  }
}
```

#### federated_discover

Discover tools available on remote instances.

```json
{
  "name": "federated_discover",
  "inputSchema": {
    "type": "object",
    "properties": {
      "instances": { "type": "array", "items": { "type": "string" } },
      "query": { "type": "string" },
      "strategy": { "type": "string", "enum": ["keyword", "semantic", "category"] }
    },
    "required": ["query"]
  }
}
```

**Example response:**
```json
{
  "results": [
    {
      "instance": "instance-b.local",
      "server_id": "tool.browser",
      "tool_name": "fetch_webpage",
      "relevance_score": 0.91,
      "latency_ms": 45
    }
  ]
}
```

### 3.5 Transparent Federation

For seamless integration, the federation proxy can register remote tools as if they were
local, using Dynamic Registration (§15.4):

```
Federation proxy starts
  │
  ├─ Connect to remote instances
  │
  ├─ Fetch remote tool lists
  │
  ├─ Register each remote tool locally via mgp.discovery.register
  │    with server_id prefix: "remote.instance_b.tool.browser"
  │
  └─ Local agents discover remote tools via mgp.tools.discover
     as if they were local (federation is transparent)
```

### 3.6 Security Considerations

- Federation proxy MUST validate remote instance identity (TLS + API key)
- Remote tool calls inherit the local agent's access control (§5)
- Audit events (§6) MUST include the remote instance in the `target` field
- The federation proxy SHOULD apply `network_restricted` validator (§4.5) to prevent
  SSRF attacks through remote tool calls

### 3.7 MGP Primitives Used

| Primitive | Usage |
|-----------|-------|
| Tool calls (MCP base) | Proxy tool execution |
| Discovery (§15, §16) | Registering remote tools, searching across instances |
| Security (§3, §4) | Permission validation for cross-instance calls |
| Lifecycle (§11) | Monitoring remote instance health |
| Streaming (§12) | Forwarding streamed responses from remote tools |
| Error Handling (§14) | Translating remote errors (code 5000-5002) |

---

## 4. Audit Service

### 4.1 Problem

The MGP protocol defines the audit event **format** and **standard event types** (§6), but
does not specify how events are stored, queried, or analyzed. Without a dedicated service,
audit events are only logged to stdout or local files, limiting visibility and traceability.

### 4.2 Why This Is Not a Protocol Extension

Audit storage and analysis are implementation concerns:

- The kernel emits `notifications/mgp.audit` using the standard format (§6.3)
- Where those events go (database, log file, external service) is a deployment decision
- Querying and analytics are domain-specific features, not protocol primitives

### 4.3 Architecture

```
┌─────────────────────────────────────────────────────┐
│                    MGP Kernel                       │
│                                                     │
│  Tool executed → emit notifications/mgp.audit ──────┼──► Audit MGP Server
│  Permission granted → emit notifications/mgp.audit ─┼──► (receives all events)
│  Validation failed → emit notifications/mgp.audit ──┼──►
│                                                     │
└─────────────────────────────────────────────────────┘
                                                        │
                                              ┌─────────▼──────────┐
                                              │  Audit MGP Server  │
                                              │                    │
                                              │  Tools:            │
                                              │  - query_audit_log │
                                              │  - get_audit_stats │
                                              │  - export_audit    │
                                              │                    │
                                              │  Storage:          │
                                              │  SQLite / External │
                                              └────────────────────┘
```

The kernel forwards `notifications/mgp.audit` events to the Audit server via the standard
MCP notification mechanism. Since the kernel acts as an **MCP client** to all connected
servers (including the Audit server), audit event delivery follows the standard
Client → Server notification path — no special transport or relay mechanism is required.
The Audit server persists them and exposes query tools.

### 4.4 Audit Server Tools

#### query_audit_log

Search and filter audit events.

```json
{
  "name": "query_audit_log",
  "inputSchema": {
    "type": "object",
    "properties": {
      "event_type": { "type": "string", "description": "Filter by event type" },
      "agent_id": { "type": "string", "description": "Filter by actor agent" },
      "server_id": { "type": "string", "description": "Filter by target server" },
      "result": { "type": "string", "enum": ["SUCCESS", "FAILURE", "BLOCKED"] },
      "since": { "type": "string", "description": "ISO-8601 start timestamp" },
      "until": { "type": "string", "description": "ISO-8601 end timestamp" },
      "trace_id": { "type": "string", "description": "Filter by trace ID" },
      "limit": { "type": "number", "description": "Max results (default 50)" }
    }
  }
}
```

**Example — find all blocked tool calls in the last hour:**
```json
{
  "event_type": "TOOL_BLOCKED",
  "since": "2026-02-27T11:00:00Z",
  "limit": 20
}
```

#### get_audit_stats

Aggregate statistics over a time range.

```json
{
  "name": "get_audit_stats",
  "inputSchema": {
    "type": "object",
    "properties": {
      "since": { "type": "string" },
      "until": { "type": "string" },
      "group_by": {
        "type": "string",
        "enum": ["event_type", "agent_id", "server_id", "result", "hour"],
        "description": "Aggregation dimension"
      }
    }
  }
}
```

**Example response:**
```json
{
  "period": { "since": "2026-02-27T00:00:00Z", "until": "2026-02-27T23:59:59Z" },
  "total_events": 1247,
  "breakdown": {
    "TOOL_EXECUTED": 1102,
    "TOOL_BLOCKED": 43,
    "PERMISSION_GRANTED": 12,
    "VALIDATION_FAILED": 38,
    "SERVER_CONNECTED": 28,
    "SERVER_DISCONNECTED": 24
  }
}
```

#### export_audit

Export audit events in structured format for external analysis.

```json
{
  "name": "export_audit",
  "inputSchema": {
    "type": "object",
    "properties": {
      "format": { "type": "string", "enum": ["json", "csv", "jsonl"] },
      "since": { "type": "string" },
      "until": { "type": "string" },
      "event_types": { "type": "array", "items": { "type": "string" } }
    },
    "required": ["format"]
  }
}
```

### 4.5 Retention Policy

The Audit server SHOULD implement a configurable retention policy:

| Policy | Behavior |
|--------|----------|
| `keep_all` | Never delete events |
| `time_based` | Delete events older than N days (e.g., 90 days) |
| `size_based` | Delete oldest events when storage exceeds N MB |
| `tiered` | Full detail for N days, then summarized, then deleted |

### 4.6 MGP Primitives Used

| Primitive | Usage |
|-----------|-------|
| Audit Event Format (§6.3) | Standard event structure consumed by this server |
| Trace ID (§6.5) | Enables cross-server event correlation |
| Tool Discovery (§16) | Agents find audit tools when needed |
| Access Control (§5) | Restrict who can query audit logs |

---

## 5. Pattern Selection Guide

| Need | Pattern | Complexity |
|------|---------|------------|
| "Agent A should ask Agent B for help" | Multi-Agent (§1) | Low |
| "Keep context window under control" | Context Management (§2) | Medium |
| "Use tools from another machine" | Federation (§3) | High |
| "Track and query all security events" | Audit Service (§4) | Low |
| "All of the above" | Deploy all four as MGP servers | Each is independent |

Each pattern is implemented as an independent MGP server. They can be deployed individually
or combined. No pattern depends on another — they all build on the same MGP protocol
primitives.

---

## 5. Relationship to MGP Specification

This document demonstrates that MGP's protocol primitives (§2-7, §11-16) are **sufficient
to express complex coordination patterns** without protocol extensions. This validates
MGP's design principle of core minimalism:

> The protocol defines the minimum necessary primitives. Application-level coordination
> is built on top of, not into, the protocol.

If a future pattern reveals a genuine gap in the protocol primitives that cannot be
worked around, it will be promoted to a protocol extension in `MGP_SPEC.md`.
