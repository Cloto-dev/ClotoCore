# Memory Capability Contract

This document defines what an MCP server must implement to act as a **memory
provider** for a ClotoCore agent, and which operations are required versus
optional. The goal is that memory is a *swappable capability*: cpersona is one
implementation, and any third-party server that conforms to this contract can
back an agent's long-term memory without kernel changes.

The kernel never calls a memory provider by name. Memory operations route through
the **capability dispatcher** (`crates/core/src/managers/capability_dispatcher.rs`):
a handler asks for `CapabilityType::Memory` + an operation name, and the dispatcher
resolves the agent's memory server and forwards the call. See also
`docs/MCP_PLUGIN_ARCHITECTURE.md` (Pattern-C) and `docs/ARCHITECTURE.md` §1
(capability over concrete type).

## How a server declares it is a memory provider

A server is classified as a memory provider in one of two ways:

1. **Manifest-driven (preferred).** In its MGP `initialize` response the server
   declares the operations it provides per capability:

   ```json
   {
     "capabilities": {
       "mgp": {
         "version": "0.6.3",
         "tools_for_capability": {
           "Memory": ["store", "recall", "list_memories", "archive_episode", "set_recall_precision"]
         }
       }
     }
   }
   ```

   The kernel maps each listed tool to `CapabilityType::Memory`. Unknown
   capability names are ignored — the kernel never invents capabilities.

2. **Heuristic fallback.** A server that does not declare `tools_for_capability`
   is classified by a `memory.` server-id prefix, or by tool name against the
   well-known memory operation set (`classify_tool` in
   `capability_dispatcher.rs`). cpersona uses this path (its id is `cpersona`
   after the bug-388 rename, so its tools are classified by name).

A manifest-driven server **must** list every operation it supports — including
the optional ones below — or feature detection will report them as unavailable.

## Required operations

A conforming memory server **must** implement:

| Operation        | Purpose                                  |
| ---------------- | ---------------------------------------- |
| `store`          | Persist a message / memory.              |
| `recall`         | Retrieve relevant memories for a query.  |
| `list_memories`  | Enumerate stored memories.               |

## Optional, feature-detected operations

These are **optional**. The kernel feature-detects each via
`has_capability_tool(CapabilityType::Memory, <op>)` and the dashboard surfaces
support through the `capabilities` object returned by `GET /api/memories`. UI
controls for an optional op are shown/enabled only when the active memory server
advertises it. A provider that omits an optional op is still fully conformant.

| Operation             | Purpose                                                        |
| --------------------- | -------------------------------------------------------------- |
| `update_memory`       | Edit a stored memory's content.                                |
| `lock_memory`         | Protect a memory from deletion/eviction.                       |
| `unlock_memory`       | Remove a lock.                                                 |
| `list_episodes`       | Enumerate episodic summaries.                                  |
| `delete_episode`      | Remove an episode.                                             |
| `archive_episode`     | Summarize and archive a conversation as an episode.           |
| `delete_memory`       | Remove a memory.                                               |
| `delete_agent_data`   | Remove all data for an agent.                                  |
| `update_profile`      | Update the agent's profile/summary.                            |
| `set_recall_precision`| Tune the agent's recall precision (recall tuning). See below.  |

### Recall tuning: `set_recall_precision`

Adjusts how strict an agent's recall is — the precision/recall trade-off of the
provider's quality gate — on a **per-agent** basis.

- **Args**: `{ "agent_id": string, "precision": "strict" | "balanced" | "lenient", "beta"?: number }`.
  - `precision` selects a named operating point. An empty string (with `beta` ≤ 0)
    clears the per-agent override and returns the agent to the provider's default.
  - `beta` (optional) is a raw specificity weight that overrides the named level
    when `> 0`. The ClotoCore UI sends only `precision`; `beta` is for advanced /
    programmatic use.
- **Semantics**: the provider owns the precision state. It is expected to apply
  the change immediately (e.g. recalibrate its gate) and persist it, so the
  effect is live without a restart. `strict` favors precision (fewer
  contaminants, more misses); `lenient` favors recall (fewer misses, more
  contaminants); `balanced` is the default operating point.
- **ClotoCore path**: `POST /api/agents/:id/recall-precision { precision }` →
  `handlers::set_recall_precision` → `call_capability_tool(Memory, "set_recall_precision", …)`.
  The endpoint returns 400 if the active memory server does not advertise the op.

#### Read-back (planned)

There is currently **no** standard read-back op, so the ClotoCore UI treats
precision as **write-only**: the control starts at `balanced` and only sends a
request when the operator changes it. Precision state is **not** mirrored into
`agent.metadata` — that would put provider-owned state into kernel data the
kernel ignores. Read-back is planned as a future **optional** op,
`get_recall_precision(agent_id) -> { precision, beta }`, which a provider may add
and the UI will feature-detect to switch precision to a read→edit→save control.

## Conformance checklist (third-party memory server)

1. Implement `store`, `recall`, `list_memories`.
2. Optionally implement any operations from the optional table; advertise each
   one you implement (manifest `tools_for_capability.Memory`, or rely on the name
   heuristic).
3. Match the documented argument shapes for any optional op you implement so the
   ClotoCore handlers and UI can drive it unchanged.
4. Register under a server id the kernel can resolve as a memory provider
   (a `memory.` prefix, or a manifest declaration).
