# MGP Tool Latency Audit

> Audit date: 2026-03-11
> Scope: All 13 MCP/MGP servers, ~40 tools
> Purpose: Identify tools that agents should avoid or conditionally bypass due to latency, computational cost, or inappropriate autonomous use

## Overview

ClotoCore's MGP tool ecosystem includes tools with latency ranging from sub-millisecond
(pure computation) to several minutes (GPU-bound generation). When agents autonomously
select tools via `mgp.tools.discover`, latency-unaware selection can degrade user
experience and waste resources.

This document classifies all tools by their latency profile and provides bypass
guidelines for agent system prompts and tool discovery ranking.

---

## Tier S: Bypass Strongly Recommended

Tools that should **never** be called without explicit user intent.
High latency, heavy resource consumption, or irreversible side effects.

| Server | Tool | Latency | Reason |
|--------|------|---------|--------|
| **imagegen** | `generate_image` | 30s - 5min | Full GPU occupation (Stable Diffusion). Only call on explicit user request for image generation. |
| **research** | `deep_research` | 5 - 15s | Internal loop: LLM x3 + web search x3 per iteration, up to 3 iterations. Use `web_search` first; escalate only if insufficient. |
| **stt** | `transcribe` | Realtime - 3x | 1 min audio = 20-60s processing. First call includes model download delay. Only call when audio input is provided. |
| **capture** | `analyze_image` | 5 - 15s | PaddleOCR + Ollama llava in parallel. If only text extraction is needed, configure `VISION_OCR_MODE=ocr` to skip vision model. |

### Agent guideline

> Do not call Tier S tools unless the user explicitly requests the action
> (e.g., "generate an image of...", "research this topic in depth",
> "transcribe this audio", "analyze this screenshot").

---

## Tier A: Conditional Bypass Recommended

Tools with moderate latency where a lighter alternative often exists.
Agents should prefer the alternative and escalate only when needed.

| Server | Tool | Latency | Alternative / Guideline |
|--------|------|---------|-------------------------|
| **websearch** | `fetch_page` | 1 - 3s / page | Check if `web_search` snippets already contain the needed information before fetching full pages. |
| **cpersona** | `update_profile` | 1 - 2s | Triggers internal LLM call for fact extraction. Batch at conversation end rather than calling per-turn. |
| **cpersona** | `archive_episode` | 1 - 2s | LLM summarization + keyword extraction. Call at conversation boundaries, not mid-conversation. |
| **cpersona** | `recall` (vector) | ~500ms | Embedding API call + cosine search. Cache hit is fast (~50ms), but miss requires full embedding computation. Consider whether `list_memories` (simple SQLite query) suffices. |
| **ollama** | `think` / `think_with_tools` | Variable | Local inference; latency depends entirely on hardware. On CPU-only systems, can take tens of seconds. Prefer `cerebras` for speed-critical reasoning. |

### Agent guideline

> Prefer lighter alternatives first. Use Tier A tools when the lighter
> alternative proves insufficient or when the user's request specifically
> requires the capability (e.g., "fetch that page", "recall related memories").

---

## Tier B: Diagnostic / Administrative

Tools intended for system diagnostics or operator actions.
Agents should not call these autonomously during normal conversation.

| Server | Tool | Latency | Reason |
|--------|------|---------|--------|
| **websearch** | `search_status` | ~100ms | Provider health check. No conversational use case. |
| **ollama** | `list_models` | ~1s | Model inventory. Only on explicit user query ("what models are available?"). |
| **ollama** | `switch_model` | ~100ms | Model switching is an operational decision. Agent should not autonomously change models. |
| **stt** | `list_models` | ~100ms | Same as above. |
| **imagegen** | `list_models` | ~1s | Same as above. |
| **gaze** | `get_tracker_status` | ~1ms | Debug/monitoring tool. |
| **gaze** | `start_tracking` | ~1-2s | Camera lifecycle management. Agents must not autonomously activate hardware sensors. |
| **gaze** | `stop_tracking` | ~100ms | Same as above. |

### Agent guideline

> Do not call Tier B tools unless the user explicitly asks for diagnostic
> information or system management actions. These tools exist for operators
> and debugging, not for conversational AI flow.

---

## Tier C: Safe for Autonomous Use

Low-latency tools with no significant resource cost. Agents may call freely.

| Server | Tool | Latency | Notes |
|--------|------|---------|-------|
| **agent_utils** | `get_current_time` | < 1ms | Pure computation. |
| **agent_utils** | `calculate` | < 1ms | AST-based safe eval, no arbitrary code execution. |
| **agent_utils** | `date_math` | < 1ms | Date arithmetic and difference calculation. |
| **agent_utils** | `random_number` | < 1ms | CSPRNG or PRNG. |
| **agent_utils** | `generate_uuid` | < 1ms | UUID v4 generation. |
| **agent_utils** | `convert_units` | < 1ms | Unit conversion (length, weight, temperature, time, data). |
| **agent_utils** | `encode_decode` | < 1ms | base64, URL, hex, HTML entity encoding/decoding. |
| **agent_utils** | `hash` | < 1ms | MD5, SHA1, SHA256, SHA512. |
| **cerebras** | `think` | ~500ms | Fastest LLM provider. Preferred for speed-critical reasoning. |
| **cerebras** | `think_with_tools` | ~500ms | Same, with tool-calling capability. |
| **deepseek** | `think` | ~2s | Cloud-based reasoning. Acceptable latency for quality-critical tasks. |
| **deepseek** | `think_with_tools` | ~2s | Same, with tool-calling capability. |
| **claude** | `think` | ~3s | Highest quality reasoning. Acceptable when quality > speed. |
| **claude** | `think_with_tools` | ~3s | Same, with tool-calling capability. |
| **cron** | `create_cron_job` | ~100ms | Thin HTTP proxy to kernel CRON API. |
| **cron** | `list_cron_jobs` | ~100ms | Same. |
| **cron** | `delete_cron_job` | ~100ms | Same. |
| **cron** | `toggle_cron_job` | ~100ms | Same. |
| **cron** | `run_cron_job_now` | ~100ms | Same. |
| **gaze** | `get_gaze` | ~1ms | Returns cached gaze coordinates. |
| **gaze** | `is_user_present` | ~1ms | Returns cached face detection result. |
| **cpersona** | `store` | ~50ms + embed | Fast with embedding cache hit. |
| **cpersona** | `list_memories` | ~10ms | Simple SQLite query. |
| **cpersona** | `list_episodes` | ~10ms | Simple SQLite query. |
| **cpersona** | `delete_memory` | ~10ms | Simple SQLite delete. |
| **cpersona** | `delete_episode` | ~10ms | Simple SQLite delete. |
| **cpersona** | `delete_agent_data` | ~50ms | Bulk delete, kernel-initiated only. |
| **websearch** | `web_search` | 200ms - 2s | Primary information retrieval. Snippets often sufficient without `fetch_page`. |
| **capture** | `capture_screen` | ~200ms | Screenshot only, no analysis. |

---

## Escalation Patterns

Recommended patterns for agents to minimize latency while maintaining quality.

### Information Retrieval

```
web_search (snippets)
  └─ insufficient? ──→ fetch_page (full content)
                          └─ insufficient? ──→ deep_research (agentic RAG)
```

### Image Understanding

```
capture_screen (screenshot only)
  └─ need text? ──→ analyze_image (OCR mode)
                      └─ need scene understanding? ──→ analyze_image (hybrid/vision mode)
```

### Memory Retrieval

```
list_memories (fast SQLite)
  └─ need semantic search? ──→ recall (vector mode)
                                 └─ need profile context? ──→ recall (with profile injection)
```

### LLM Provider Selection (speed vs quality)

```
cerebras (~500ms, fast bulk processing)
  └─ need reasoning? ──→ deepseek (~2s, strong reasoning)
                           └─ need highest quality? ──→ claude (~3s, best overall)
```

---

## Implementation Recommendations

### 1. System Prompt Injection

Inject tier guidelines into agent system prompts so that LLM-based agents
naturally avoid high-latency tools. Example clause:

> "Before calling a tool, consider its latency tier. Prefer Tier C tools
> (< 1s) over Tier A (1-3s) when both can accomplish the task. Never call
> Tier S tools (> 5s) without explicit user request."

### 2. Discovery Score Weighting

Extend `mgp.tools.discover` (in `mcp_tool_discovery.rs`) to incorporate
a latency cost factor into relevance scoring:

```
final_score = keyword_score * (1.0 - latency_penalty)
```

Where `latency_penalty` is derived from tool metadata (e.g., `_mgp.latency_tier`).
This causes the discovery system to naturally rank faster tools higher.

### 3. Event-Driven CPersona Operations

Move `update_profile` and `archive_episode` from agent-initiated tool calls
to kernel event handlers triggered on conversation boundaries:

- `conversation.end` event → auto-archive episode
- `conversation.end` event → auto-update profile (if new facts detected)

This eliminates in-conversation latency entirely for these operations.

---

## Appendix: Server Timeout Configuration Reference

All timeouts are configurable via environment variables in `crates/core/src/config.rs`:

| Config Key | Default | Max | Applies To |
|------------|---------|-----|------------|
| `PLUGIN_EVENT_TIMEOUT_SECS` | 120s | 300s | Per-plugin event execution |
| `TOOL_EXECUTION_TIMEOUT_SECS` | (configurable) | - | Per-tool call |
| `MCP_REQUEST_TIMEOUT_SECS` | (configurable) | - | MCP method calls |
| `MEMORY_TIMEOUT_SECS` | (configurable) | - | Memory server retrieval |
| `LLM_PROXY_TIMEOUT_SECS` | (configurable) | - | LLM proxy requests |
| `DB_TIMEOUT_SECS` | (configurable) | - | Database operations |
| `MAX_EVENT_DEPTH` | 10 | 50 | Cascade depth limit |
