# Multi-Agent Delegation — v0.5.x Design

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-03
**Target:** v0.5.x

---

## 1. Overview

A multi-agent coordination system that enables ClotoCore agents to query and
delegate tasks to other agents. Implemented as a hybrid architecture combining
Agent-as-Tool (tool exposure) and Event-Driven Delegation (event-based delegation).

### 1.1 Design Principles

- **Core Minimalism**: The Kernel handles only delegation routing and safety controls
- **Event-First**: Internal implementation is loosely coupled via the event bus
- **Permission Isolation**: Delegatee agents operate with their own permissions only (no permission inheritance)
- **Loop Prevention**: Prevents infinite loops through delegation depth limits and call chain tracking

### 1.2 Architecture Summary

```
User → Agent A (agentic loop)
         │
         ├── tool call: ask_agent("agent_b", "prompt")
         │     │
         │     └── Kernel: DelegationRequested event
         │           │
         │           ├── Permission check (A→B delegation allowed?)
         │           ├── Depth check (max_delegation_depth)
         │           ├── Chain tracking (circular reference detection)
         │           └── Agent B: handle_delegation() → response
         │                 │
         │                 └── DelegationCompleted event
         │                       │
         │                       └── Tool result returned to Agent A
         │
         └── Agent A: synthesize final response with B's input
```

### 1.3 Solved Problems

| Problem | Solution |
|---------|----------|
| Agent-to-agent loop | Delegation depth limit + chain tracking |
| Permission escalation | Delegatee uses own grants only (no inheritance) |
| Context leakage | Only prompt is passed (no conversation history) |
| Agent ID spoofing | Kernel enforces source agent identity |
| Resource exhaustion | Per-delegation token budget + timeout |
| Circular delegation | Call chain recorded, same agent cannot appear twice |

---

## 2. Use Cases

### UC-1: Character Interaction

**Layer:** Casual / Entertainment
**Priority:** Medium

Similar to Neuro-Sama and Evil Neuro, multiple AI characters converse with each other.
When the user instructs "Have Sapphy and the Assistant discuss X," a multi-turn
dialogue occurs between agents.

**Flow:**

```
User: "Have Sapphy and Researcher discuss the future of Rust"
  │
  ├── Agent A (Sapphy): "I think Rust's safety is its greatest strength.
  │   What do you think, Researcher?"
  │     │
  │     └── ask_agent("researcher", "Share your opinion on the future of Rust.
  │           I'm arguing that safety is its key strength")
  │           │
  │           └── Agent B (Researcher): "Looking at the data, it has been
  │                 the most loved language for 8 consecutive years in the
  │                 Stack Overflow survey. However..."
  │
  ├── Agent A: Responds with a counterargument or agreement based on
  │   Researcher's opinion
  │     │
  │     └── ask_agent("researcher", "Regarding that point...")
  │
  └── Agent A: Summarizes the discussion and reports to the user
```

**Requirements:**
- Allow multi-turn delegation (within max_delegation_depth)
- Each agent generates responses maintaining its own personality
- Both agents' statements are visible to the user (via SSE events)

**UI Considerations:**
- Multiple agent icons/names displayed in the chat screen
- "Querying Agent B..." indicator shown during delegation
- Each turn of the discussion viewable as a timeline

---

### UC-2: Specialist Consultation

**Layer:** Casual / Practical
**Priority:** High

When the main character agent receives a question outside its area of expertise,
it queries a specialist agent behind the scenes and responds in its own character voice.

**Flow:**

```
User: "What should I have for dinner tonight?"
  │
  └── Agent A (Sapphy — general personality):
        │
        ├── "Let me ask someone who knows about cooking!"
        │
        ├── ask_agent("chef", "The user is asking for dinner suggestions.
        │     It's March; please suggest 3 easy-to-make dishes")
        │     │
        │     └── Agent B (Chef — cooking specialist):
        │           "1. Rapini aglio e olio 2. Stir-fried spring cabbage 3. ..."
        │
        └── Agent A: Rephrases Chef's suggestions in Sapphy's voice
              "I asked around! Since it's March, spring ingredients are the way to go!
               How about rapini aglio e olio, or stir-fried spring cabbage..."
```

**Requirements:**
- One-to-one single-shot delegation (the simplest pattern)
- The delegator reinterprets the delegatee's response in its own personality
- Whether the delegatee agent's existence is disclosed to the user is configurable

**Design Notes:**
- Agent A's system prompt instructs: "delegate via ask_agent for topics outside your expertise"
- Agent B returns raw specialist information (no personality needed, accuracy-focused)
- Agent A functions as a personality filter

---

### UC-3: Second Opinion

**Layer:** Casual / Quality
**Priority:** Medium

After the main agent generates its own response to the user's question,
it incorporates another agent's opinion to reinforce the final answer.

**Flow:**

```
User: "Is the performance of this code okay?"
  │
  └── Agent A (Main):
        │
        ├── [Initial self-analysis] "There's an O(n^2) loop here..."
        │
        ├── ask_agent("reviewer", "I've flagged an O(n^2) performance issue
        │     in the following code. Are there any other issues I've missed?
        │     Code: ...")
        │     │
        │     └── Agent B (Reviewer):
        │           "The O(n^2) observation is correct. Additionally, there's
        │            a memory allocation occurring inside the loop..."
        │
        └── Agent A: Integrates both analyses into the final response
              "In addition to the O(n^2) issue, another problem was found.
               The memory allocation inside the loop is also..."
```

**Requirements:**
- Agent A generates its own answer first, then delegates
- The delegation prompt includes Agent A's analysis (context sharing)
- Agent A holds integration responsibility for the final answer

**Design Notes:**
- Higher-level concept of ConsensusOrchestrator (engine coordination)
- ConsensusOrchestrator = sends the same prompt to multiple engines
- UC-3 = Agent B supplements based on Agent A's analysis (asymmetric)

---

### UC-4: Task Decomposition (MCP Permission-Based Task Splitting)

**Layer:** Technical / Productivity
**Priority:** High

Complex tasks are split and delegated to multiple specialist agents based on
MCP server access permissions. Each agent uses only the tools granted to it.

**Flow:**

```
User: "Look up the latest Rust security advisories and check if our Cargo.toml is affected"
  │
  └── Agent A (Coordinator — no MCP tools):
        │
        ├── ask_agent("researcher", "Investigate the latest Rust security
        │     advisories. List the CVE numbers, affected crates, and
        │     patch versions")
        │     │
        │     └── Agent B (Researcher — websearch MCP granted):
        │           [web_search tool] → "RustSec Advisory: CVE-2026-XXXX..."
        │
        ├── ask_agent("developer", "Check whether the following security
        │     advisories affect our Cargo.toml.
        │     Advisories: ... Please read the Cargo.toml and verify")
        │     │
        │     └── Agent C (Developer — terminal MCP granted):
        │           [terminal: cat Cargo.toml] → [terminal: cargo audit]
        │           → "Affected: crate X version Y.Z"
        │
        └── Agent A: Integrates investigation results and impact analysis into a report
```

**Requirements:**
- The Coordinator holds no tools itself (purely an orchestration role)
- Each worker agent uses only its granted MCP servers
- Delegations can execute in parallel (for independent subtasks)
- The Coordinator integrates results into the final report

**Design Notes:**
- Natural realization of the principle of least privilege: each agent is granted only the minimum required permissions
- When new task types arise, extension is as simple as granting additional MCP servers
- The existing `mcp_access_control` table infrastructure can be used as-is

**Parallel Delegation:**
```
Agent A ─┬── ask_agent("researcher", ...) ──► Agent B ──┐
         │                                               ├── Agent A: synthesize
         └── ask_agent("developer", ...)  ──► Agent C ──┘
```

---

### UC-5: Review / Verification

**Layer:** Technical / Quality
**Priority:** Medium

One agent's output is verified by another agent specialized in review.
Applied to tasks where quality assurance is critical, such as code generation,
translation, and analysis.

**Flow:**

```
User: "Write an authentication middleware"
  │
  └── Agent A (Developer — terminal MCP granted):
        │
        ├── [Code generation] Creates auth_middleware.rs
        │
        ├── ask_agent("reviewer", "Please review the following authentication
        │     middleware code. Identify security vulnerabilities, missed edge
        │     cases, and performance issues.
        │     Code: ```rust ... ```")
        │     │
        │     └── Agent B (Reviewer — no MCP tools, reasoning-focused):
        │           "Issues: 1. Vulnerable to timing attacks (should use
        │            constant-time comparison) 2. Token expiration check is..."
        │
        ├── [Agent A: Revises the code based on review feedback]
        │
        └── Agent A: "I've applied the review feedback. Changes: ..."
```

**Requirements:**
- The Reviewer is read-only (no code modification permissions)
- Fixes based on review results are performed by the original agent
- Multiple rounds of review cycles are possible

**Design Notes:**
- A specialized form of UC-2 (Specialist Consultation) where the delegatee specializes in "verification"
- The Reviewer agent is assigned a reasoning-heavy engine (DeepSeek)
- The Developer agent is assigned a tool-capable engine (Cerebras)
- Design that matches engine strengths to agent roles

---

### UC-6: Cross-Engine Collaboration

**Layer:** Technical / Advanced
**Priority:** Low

Agents with different LLM engines collaborate to leverage each engine's
strengths for task processing. An agent-level extension of ConsensusOrchestrator.

**Flow:**

```
User: "Analyze the computational complexity of this algorithm and implement an improvement"
  │
  └── Agent A (Analyst — DeepSeek engine, reasoning-focused):
        │
        ├── [Deep reasoning] "The current complexity is O(n^3).
        │     It can be improved to O(n^2) using dynamic programming.
        │     State transition: dp[i][j] = ..."
        │
        ├── ask_agent("implementer", "Please implement the following algorithm
        │     improvement. Current: O(n^3), improved: O(n^2) DP.
        │     State transition: dp[i][j] = ... Please also create test cases")
        │     │
        │     └── Agent B (Implementer — Cerebras engine, tool-capable):
        │           [terminal: create file] → [terminal: run tests]
        │           → "Implementation complete. Tests 5/5 passed. Benchmark: 340ms → 12ms"
        │
        └── Agent A: Integrates analysis and implementation results into a report
              "Completed the improvement from O(n^3) to O(n^2).
               Measured a 28x speedup..."
```

**Requirements:**
- Each agent uses a different LLM engine
- Separation of reasoning quality and tool execution capability
- Optimal task assignment based on engine characteristics

**Design Notes:**
- Difference from ConsensusOrchestrator:
  - Consensus: sends the same prompt to multiple engines and integrates results
  - UC-6: assigns different prompts/tasks to each engine's area of expertise
- Integration with engine routing (v0.4.x):
  - Existing routing rules operate at the message level
  - UC-6 selects agents (= engines) at the task level

---

## 3. Safety Framework

### 3.1 Delegation Depth Limit

```
max_delegation_depth = 3  (configurable)

User → Agent A → Agent B → Agent C  ← OK (depth 3)
User → Agent A → Agent B → Agent C → Agent D  ← BLOCKED (depth 4)
```

### 3.2 Circular Reference Detection

```
delegation_chain: ["agent_a", "agent_b", "agent_c"]

Agent C → ask_agent("agent_a", ...)  ← BLOCKED (agent_a already in chain)
```

### 3.3 Permission Matrix

```
delegation_access_control:
┌──────────┬──────────┬─────────┐
│ source   │ target   │ allowed │
├──────────┼──────────┼─────────┤
│ sapphy   │ chef     │ true    │
│ sapphy   │ reviewer │ true    │
│ chef     │ sapphy   │ false   │ ← asymmetric by design
│ *        │ *        │ false   │ ← default deny
└──────────┴──────────┴─────────┘
```

### 3.4 Context Isolation

| Data | Passed to delegatee? |
|------|---------------------|
| Delegation prompt | Yes |
| Delegator's conversation history | No |
| Delegator's system prompt | No |
| Delegator's MCP grants | No |
| Delegator's agent_id | Yes (as metadata, read-only) |

### 3.5 Resource Limits

| Resource | Limit |
|----------|-------|
| Delegation depth | 3 (default) |
| Per-delegation timeout | 60s (default) |
| Per-delegation token budget | Configurable per agent pair |
| Concurrent delegations | 5 per agent (default) |

---

## 4. Implementation Scope

### 4.1 Kernel Changes

| Component | Change |
|-----------|--------|
| `ClotoEventData` | Add `DelegationRequested`, `DelegationCompleted` variants |
| `SystemHandler` | Add `handle_delegation()` method |
| `SystemHandler::on_event` | Allow `Agent` source with delegation context |
| `AgentManager` | Add `delegation_access_control` table queries |
| DB migrations | `delegation_access_control` table |

### 4.2 Tool Definition

```json
{
  "name": "ask_agent",
  "description": "Ask another agent to perform a task or answer a question",
  "parameters": {
    "target_agent_id": {
      "type": "string",
      "description": "The ID of the agent to delegate to"
    },
    "prompt": {
      "type": "string",
      "description": "The task or question for the target agent"
    }
  }
}
```

### 4.3 Dashboard UI

| Component | Change |
|-----------|--------|
| AgentConsole | Show delegation events inline (agent icon + name) |
| AgentPluginWorkspace | Delegation permission matrix UI |
| Settings | `max_delegation_depth`, timeout configuration |

---

## 5. Relationship to Existing Systems

| System | Relationship |
|--------|-------------|
| ConsensusOrchestrator | Orthogonal — Consensus coordinates engines, delegation coordinates agents |
| Engine Routing | Complementary — each agent uses its own routed engine |
| MCP Access Control | Extended — delegation adds agent-to-agent permission layer |
| Anti-spoofing (agent_id injection) | Preserved — delegatee cannot impersonate delegator |
| Event depth limit (max 5) | Separate — delegation depth is independent of event cascade depth |

---

## 6. Future Extensions

- **Broadcast delegation**: ask multiple agents simultaneously (UC-4 parallel)
- **Streaming delegation**: delegatee streams partial results back to delegator
- **Delegation marketplace**: community-shared specialist agent templates
- **Autonomous delegation**: agents decide when to delegate without user instruction
- **Delegation analytics**: track delegation patterns, success rates, latency

---

*Document created: 2026-03-03*
