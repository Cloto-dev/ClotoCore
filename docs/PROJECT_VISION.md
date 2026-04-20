# ClotoCore Project Vision

> **"Neuro-Sama for Everyone"**
> A platform where anyone can build and operate advanced AI characters through a GUI

---

## 1. What is ClotoCore?

ClotoCore is a Rust-based open-source platform that makes it possible to build and operate
highly advanced AIs -- comparable to Neuro-Sama -- through a high-quality GUI,
using the combination of **AI Containers** and **plugin sets**.

It is not a chatbot. It is not an AI assistant.
It is a platform that lets you create **an AI partner with personality, capabilities,
and the ability to build a relationship with the user** --
on your own machine, with your own data, with your own hands.

---

## 2. Competitive Analysis: OpenClaw

| Aspect | OpenClaw | ClotoCore |
|--------|----------|-----------|
| **Language** | TypeScript | Rust |
| **UI** | Messaging platforms (WhatsApp, Discord, etc.) | GUI Dashboard + Tauri desktop |
| **Design philosophy** | Chat-based personal assistant | Plugin-composable AI container |
| **Security** | Broad local permissions | Sandbox and permission isolation |
| **Extension** | TypeScript / WASM / Skills | MCP server plugins (any language) |
| **License** | Apache 2.0 | BSL 1.1 → MIT (2028) |

### ClotoCore's Differentiators

1. **Rust** -- Performance, memory safety, low resource usage
2. **Security-first** -- SafeHttpClient, MCP process isolation, permission separation
3. **GUI-first** -- Dashboard enables operation by non-technical users
4. **AI Containers** -- A unique concept of packaged personality and capability sets

---

## 3. Target Users

### Tier 1: Casual Users

Users who enjoyed GPT-4o and are seeking an AI partner.

**What they want:**
- AI with personality, emotional connection, visuals
- An interface usable without technical knowledge
- Privacy (the assurance that data stays local)

**What ClotoCore provides:**
- One-click installation of preset "AI Containers"
- GUI-based parameter tuning (personality, voice, appearance)
- Real-time conversation from the dashboard

**Message:**
> *"Your own AI partner, on your own PC. Your data never leaves."*

### Tier 2: Technical Users

Developers and researchers seeking an advanced framework.

**What they want:**
- An extensible framework
- Rust's safety and performance
- Freedom in plugin development

**What ClotoCore provides:**
- MCP (Model Context Protocol)-based extension model (write MCP Servers in any language)
- Event-driven architecture
- Security sandbox

**Message:**
> *"Security-first. ClotoCore is designed with permission isolation and sandboxing."*

---

## 4. Core Concept: AI Containers

An AI Container is a distributable unit that packages together
**a plugin set + personality definition + capability set**.

```
AI Container = Plugin Set + Personality Definition + Capability Set

Example: "Neuro-style VTuber" Container
├── reasoning: DeepSeek (conversation engine)
├── vision: Camera/screen recognition plugin
├── personality: Character definition
├── voice: TTS/STT plugin
└── avatar: Live2D/VRM integration plugin

Example: "Research Assistant" Container
├── reasoning: Claude / GPT-4o
├── tools: File search, web search
├── personality: Academic, accuracy-focused
└── memory: Long-term memory plugin
```

### Design Principles Learned from Neuro-Sama

The following principles are derived from Neuro-Sama's architecture (C# + Python,
coordination of multiple AI subsystems) and applied to ClotoCore:

1. **Coordination of multiple AI subsystems** -- Achieved through event-based inter-plugin communication
2. **Real-time interaction** -- Achieved through event-driven architecture
3. **Personality consistency** -- Achieved through the AI Container's personality definition
4. **Capability separation** -- Conversation, vision, and voice separated as independent plugins

---

## 5. Architecture Layer Structure

ClotoCore realizes "Neuro-Sama for Everyone" through five progressive layers.
Each layer is independent, and upper layers can be realized by adding MCP servers.
No major Kernel modifications are required.

```
┌─────────────────────────────────────────────────────┐
│  Layer 5: Frontend Experience                        │
│  Live2D/VRM avatar, TTS/STT, streaming integration   │
│  → MCP: output.avatar, voice.stt (cloto-mcp-servers repo) │
├─────────────────────────────────────────────────────┤
│  Layer 4: Emotion & Personality Engine               │
│  Internal emotional state, personality consistency,   │
│  mood fluctuation, spontaneous speech                │
│  → MCP: persona.emotion (integrated with CPersona)      │
├─────────────────────────────────────────────────────┤
│  Layer 3: Real-Time Event-Driven                     │
│  Instant response to chat, visual input, voice input │
│  → Existing: SSE + MessageReceived event             │
│  → MCP: vision.capture, @playwright/mcp              │
├─────────────────────────────────────────────────────┤
│  Layer 2: Autonomous Trigger Layer                   │
│  Heartbeat (periodic checks), Cron (scheduled exec)  │
│  → Kernel: managers/scheduler.rs (follows existing   │
│    patterns)                                         │
│  → State persistence: CPersona MCP                       │
├─────────────────────────────────────────────────────┤
│  Layer 1: Core Infrastructure              [Done]    │
│  Rust Kernel, Agentic Loop, MCP, CPersona memory         │
│  Access control, YOLO mode, Dashboard UI             │
└─────────────────────────────────────────────────────┘
```

### Layer Details

| Layer | Status | Implementation | Kernel Changes |
|-------|--------|----------------|----------------|
| **L1: Core Infrastructure** | Done | -- | -- |
| **L2: Autonomous Triggers** | Designed | `tokio::interval` + Cron job DB | Minimal (add event type + scheduler) |
| **L3: Real-Time Driven** | Partially implemented | Add MCP servers | None |
| **L4: Emotion Engine** | Not started | Add MCP servers | None |
| **L5: Avatar Integration** | Designed | Add MCP servers (Sapphy V2 VRM) | None |

### Design Principle: Layer Extension via MCP

The Kernel focuses on "routing + access control + agentic loop,"
with all capability extensions separated out as external MCP servers.

```
Kernel (Rust)
  │
  ├── MCP: mind.deepseek     (reasoning engine)        ← L1
  ├── MCP: mind.cerebras     (reasoning engine)        ← L1
  ├── MCP: memory.cpersona       (long-term memory)        ← L1
  ├── MCP: tool.terminal     (shell execution)         ← L1
  ├── MCP: tool.browser      (@playwright/mcp)         ← L3
  ├── MCP: sense.vision      (camera/screen recognition) ← L3
  ├── MCP: sense.voice       (STT voice input)         ← L3
  ├── MCP: persona.emotion   (emotional state mgmt)    ← L4
  ├── MCP: output.tts        (text-to-speech)          ← L5
  └── MCP: output.avatar     (VRM control — Sapphy V2) ← L5
```

With this design:
- Capabilities can be added without modifying the Kernel code
- Each MCP server can be implemented in any language (Python, TypeScript, Rust, etc.)
- The `mcp_access_control` table automatically applies per-agent permission management
- The community can develop and share their own MCP servers

### Technical Correspondence with Neuro-Sama

| Neuro-Sama (C# + Python) | ClotoCore (Rust + MCP) |
|--------------------------|----------------------|
| Coordination of multiple AI subsystems | Kernel orchestrates MCP server fleet |
| Real-time interaction | SSE events + MessageReceived + Heartbeat |
| Personality consistency | AI Container personality definition + persona.emotion MCP |
| Capability separation | 1 capability = 1 MCP server (process isolation) |
| Simulating continuous "consciousness" | L2 Heartbeat + L4 emotional state persistence |
| Spontaneous speech and actions | Cron triggers + emotion engine threshold evaluation |

---

## 6. Roadmap

### Layer Roadmap

| Version | Layer | Milestone |
|---------|-------|-----------|
| v0.3.x | L1 complete | Agentic loop, MCP, CPersona, Chat UX, Dashboard |
| v0.4.x | L2 added | Heartbeat/Cron scheduler, autonomous triggers |
| v0.5.x | L3 enhanced | Browser automation (Playwright MCP), Vision input |
| v0.6+ | L4 started | Emotional state management, spontaneous speech |
| v1.0+ | L5 started | TTS/STT, avatar integration, streaming integration |

### Phase A: Short-term (1-2 months) -- "Build something showable"

1. **Define the AI Container specification** -- Design a packaging format in JSON/TOML
2. **Create one demo container** -- DeepSeek + simple personality definition + dashboard conversation
3. **30-second demo video** -- Flow: "Install → select container → start conversation"
4. **Landing page** -- Message: "Build your own AI partner"

### Phase B: Mid-term (3-6 months) -- "Build a community"

5. **AI Container marketplace concept** -- Users share their created containers
6. **Migration guide from OpenClaw** -- Lead with the security comparison
7. **Post on r/LocalLLaMA, Hacker News** -- Angle: Rust-based + security-first
8. **Open a Discord community**

### Phase C: Long-term -- "Grow the ecosystem"

9. **SDK documentation for plugin developers**
10. **Guide for AI Container creators** -- Accessible even to non-programmers
11. **Implement the container marketplace**

---

## 7. Development Philosophy: Don't Do It All Alone

Polish the core 20%, and design the remaining 80% to be handled by the community.

### What we handle

- Core runtime (Rust)
- AI Container specification design
- Plugin SDK
- Security model
- Vision communication

### What the community handles

- Individual plugin development (TTS, Vision, Avatar, etc.)
- AI Container creation and sharing
- Dashboard UI improvements
- Documentation translation and expansion
- Platform-specific support

---

## 8. Positioning Statement

**If OpenClaw is "an AI assistant you instruct through chat,"
ClotoCore is "an AI partner you build through a GUI."**

OpenClaw is bound to a text interface on messaging platforms.
ClotoCore provides an experience of assembling an AI with visuals, voice,
and personality through a GUI, all while ensuring security.

---

## 9. Layer 5 Avatar System: Sapphy V2

### Selected Model

- **Model name**: Sapphy V2
- **Author**: Yueou / Virtual VoidCat
- **Source**: https://booth.pm/ja/items/3939858
- **Price**: ¥5,480 (purchased individually by users)
- **Format**: VRM, FBX (Perfect Sync compatible)
- **License**: VN3 (personal commercial use allowed; contact required for corporate use)

### Model Selection Rationale

| Aspect | Evaluation |
|--------|------------|
| **Aesthetic fit** | Silver-white hair + cyan accents + SF/cybernetic -- matches ClotoCore dashboard's glass morphism + cyan color scheme |
| **Technical fit** | VRM format, 52 ARKit BlendShapes, 15 lipsync visemes, 67,542 polygons -- renderable via three.js + @pixiv/three-vrm in WebGL |
| **Expression control** | 403 BlendShapes -- rich expression control possible via MCP tool `set_expression()` |
| **License** | Commercial use allowed for individuals, modification allowed, no credit required -- no issues for development and demos during ClotoCore's BSL period |

### Architecture

```
┌─────────────────────────────────────────────────────┐
│  Tauri WebView (three.js + @pixiv/three-vrm)        │
│  └─ VRM model rendering (60fps WebGL)               │
│     ├─ BlendShape → expressions & lip sync          │
│     ├─ SpringBone → hair & clothing physics         │
│     └─ Gaze → eye tracking                          │
├─────────────────────────────────────────────────────┤
│  avatar.vrm (MCP Server — Layer 5)                  │
│  Tools:                                              │
│    ├─ set_expression(emotion, intensity)             │
│    ├─ set_mouth_shape(viseme)                        │
│    ├─ set_gaze(x, y)                                │
│    ├─ play_animation(name)                           │
│    └─ set_idle_behavior(mode)                        │
├─────────────────────────────────────────────────────┤
│  Linked MCP Servers                                  │
│    ├─ persona.emotion (L4) → emotion → set_expression│
│    ├─ output.tts (L5)     → phoneme → set_mouth_shape│
│    └─ vision.gaze_webcam  → gaze   → set_gaze       │
└─────────────────────────────────────────────────────┘
```

### Distribution Policy

The model file is a paid asset and is not included in the repository:

1. Users purchase Sapphy V2 from BOOTH
2. Place the VRM file in `data/avatar/`
3. ClotoCore auto-detects it and enables avatar rendering

`data/avatar/` is added to `.gitignore`. Setup instructions are documented in the README.

### Future Extensions

- Swappable with any VRM model (Sapphy V2 is the recommended model, not the only option)
- The community can develop their own avatar MCP servers
- Live2D support can be added as a separate MCP server (`avatar.live2d`)

---

## 10. Planned Breaking Changes

Changes listed here will break backward compatibility with existing MGP servers
or MCP integrations. They are deferred until the benefits outweigh the migration
cost, but should be anticipated by anyone building on ClotoCore.

| Target | Change | Reason | Migration Impact |
|--------|--------|--------|------------------|
| **Magic Seal** | HMAC-SHA256 → Ed25519 asymmetric signatures | HMAC shared-key model conflates signer and verifier. Community server distribution requires each author to sign independently without sharing the kernel's secret. | MGP breaking: signature format changes, all sealed servers must be re-signed, key management shifts from shared secret to public/private key pairs. |
| **Magic Seal** | Mandatory signatures for all trust levels | Currently Core/Standard servers can be unsigned. Requiring signatures at all levels closes the gap where a compromised unsigned server inherits elevated trust. | MGP breaking: existing unsigned servers will be rejected until signed. |
| **Code Safety** | Pattern-based validation → AST analysis (tree-sitter) | Pattern matching misses obfuscated dangerous patterns and produces false positives on safe code. AST analysis provides structural understanding. | MCP potentially breaking: stricter validation may reject servers that pass current pattern checks. |
| **MCP Server Invocation** | File-path resolution (Method D) → Python package invocation (Method C: `python -m cloto_mcp_servers.<name>`) | Eliminates path configuration in `mcp.toml`, simplifies installation, enables proper versioned distribution via PyPI. | Non-breaking for users (paths still work as fallback), but server developers should prepare for package-based distribution. |

### Timeline Guidance

- **Pre-1.0**: Breaking changes may land in any minor version with migration notes in CHANGELOG
- **Post-1.0**: Breaking changes require a major version bump (SemVer)
- **Magic Seal Ed25519**: Planned for the community marketplace launch (Phase B)
- **AST analysis**: Planned when third-party server submissions begin

---

## 11. Future Optimizations

Potential improvements under consideration. These are idea-level sketches,
not committed work -- listed here so that design discussions can reference
them when relevant pressure arises.

| Target | Idea | Trigger |
|--------|------|---------|
| Frontend bulk operations | Parallelize independent API calls (e.g. agent update + batch MCP access) with `Promise.all` to reduce perceived save latency. The batch agent-access endpoint already collapses N grants into a single request, so parallelization is only worthwhile when multiple unrelated requests stack up during one save. | When user-visible save latency becomes a UX concern, or when new bulk patterns emerge that make rate-limit headroom thin again. |
| Dynamic MCP server surface in Dashboard | Derive `ENGINE_IDS`, `ALL_SELECTABLE_SERVER_IDS`, `MANUAL_START_SERVERS`, and `SERVER_PRESETS` from `mcp_servers` / `llm_providers` tables (plus a new `presets` + `preset_servers` table) instead of hardcoded TypeScript arrays. Translation keys (`engine_*`, `server_*`) would fall back to the server's `display_name` / `description` when no locale entry exists, so new servers become visible in the SetupWizard without editing the dashboard. | When community-contributed MCP servers start arriving through the marketplace (Phase B), since requiring dashboard edits per server would block Tier 1 users from installing them. |
| Reasoning-model tool-loop robustness | The current fallback chain (`reasoning_content` → `<tool_call>` parser; `</think>` assistant prefill for iter 2+) covers Qwen3 / DeepSeek-R1 today, but remains model-agnostic only by accident. Follow-ups worth considering when coverage gaps surface: (a) an `llm_providers.provider_quirks` JSON column so each provider can declare prefill style, EOS-token suppression ids, and retry policy instead of hard-coded heuristics, (b) `logit_bias` on the upstream EOS token when the provider API exposes it to physically prevent mid-`<tool_call>` truncation, and (c) an automatic iter-internal retry that re-sends with simplified instructions when fallback parsing yields zero valid calls and `content` is still empty. | When reasoning-model adoption broadens (users pointing Cloto Assistant at DeepSeek-R1, o3, or new Qwen variants) and the current parser + prefill proves insufficient for a specific model's emission quirks. |
| Real-time MCP server status push | Add a `McpServerStatusChanged { server_id, status }` variant to `ClotoEventData` and emit it at all 10 status transition points in `McpClientManager` (`connect_server`, `stop_server`, `begin_drain`, `restart_server`, `set_server_error`). Requires injecting `event_tx: mpsc::Sender<EnvelopedEvent>` into `McpClientManager` since it currently lives only in `AppState`. On the frontend, `McpServersPage` and `AgentPluginWorkspace` subscribe via the existing singleton `useEventStream` hook and call `refetch()` on receipt, replacing the current manual-refresh-only model. The SSE infrastructure (sequenced broadcast, `Last-Event-ID` replay, exponential backoff reconnection) is already mature; the only structural gap is the missing channel injection. **Known blocker discovered (2026-04-17):** servers in transitional states (Connecting, Restarting, Registered) are effectively invisible in the dashboard because `useMcpServers` fetches once on mount with a 10s TTL cache, and the Connecting→Connected transition completes within seconds during kernel boot. A shimmer animation (`@keyframes shimmer`, 1.5s sweep) was added to indicate transitional states, but it cannot fire until this push mechanism delivers status changes in real time. The shimmer condition should be re-evaluated after implementation -- it may make more sense on Connected (indicating "alive") rather than Connecting (indicating "transitioning") depending on how quickly transitions are rendered once push is live. | Immediate next step. Blocking the shimmer UX feature and causing stale status display on both MCP server list and agent config pages. |
| LLM proxy streaming unit test | Add a Rust unit test covering `crates/core/src/managers/llm_proxy.rs::proxy_handler` Phase C streaming branch. The fix detects `body.stream == true` and returns an Axum `Body::from_stream(response.bytes_stream())` instead of buffering through `response.json::<Value>()`. Candidate approach: mount the proxy router on a test server, point a `wiremock`-style mock at a fake upstream that emits a canned `text/event-stream` body, POST `{"model":"x","messages":[],"stream":true}` with `X-LLM-Provider: local`, assert the response's `content-type` is preserved and the body chunks arrive incrementally (not buffered). Also cover: non-streaming request still parses JSON; streaming request + upstream 5xx still falls through to the JSON error path. The function is currently only exercised by live LM Studio E2E, so a refactor could silently regress the stream passthrough branch. | When the proxy file is touched for any non-trivial reason (e.g. adding callback support, rate limiting, audit hooks). Low priority while the surface is frozen and E2E covers mainline. |

---

*Document created: 2026-02-16*
*Last updated: 2026-04-20*
