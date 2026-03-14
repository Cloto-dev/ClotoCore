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

*Document created: 2026-02-16*
*Last updated: 2026-03-01*
