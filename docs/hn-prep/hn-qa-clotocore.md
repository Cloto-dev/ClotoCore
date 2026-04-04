# HN想定Q&A — ClotoCore

---

## 技術・アーキテクチャ系

### Q: Why Rust? Isn't that overkill for an AI agent framework?

**A:** When you're running arbitrary plugin code (MCP servers), memory safety isn't optional. The kernel manages plugin lifecycles, injects sandboxed network capabilities, and mediates all inter-plugin communication through an event bus. Rust's ownership model makes it much easier to reason about these boundaries. We also get great performance — the kernel adds minimal overhead on top of the LLM latency that dominates response time.

### Q: What's MCP? Why build on it?

**A:** Model Context Protocol — an open standard by Anthropic for connecting AI models to external tools. Think USB-C for AI: a standard interface so any tool works with any model. Claude Desktop, Claude Code, and several other hosts support it. By building on MCP, ClotoCore plugins are compatible with the broader ecosystem — cpersona (our memory server) works standalone in Claude Desktop with zero modification.

Spec: https://modelcontextprotocol.io/

### Q: What's MGP and how does it differ from MCP?

**A:** MGP (Multi-Agent Gateway Protocol) extends MCP with event-driven communication. MCP is request-response (host calls a tool, gets a result). MGP adds: plugins can emit events, subscribe to event streams, and participate in callback-based communication. The Discord bridge uses MGP — when someone @mentions the AI on Discord, an MGP event injects the message into the agent loop, the agent processes it like any other input, and the response routes back through MGP to Discord. The agent doesn't know it's talking to Discord.

### Q: How does the sandbox/security model work?

**A:** Several layers:

1. **Capability injection** — Plugins cannot open sockets or make HTTP requests on their own. The kernel provides pre-authorized HttpClient instances with host whitelisting. A plugin configured to access `api.openai.com` literally cannot reach `example.com`.
2. **3-level RBAC** — Access control at capability type (e.g., "reasoning"), server (e.g., "mind.deepseek"), and individual tool levels. An agent can be granted access to `mind.deepseek` but blocked from `tool.terminal`.
3. **Human-in-the-loop** — Sensitive operations (shell commands, file access, new network hosts) trigger permission requests that appear in the dashboard. Admin approves or denies in real-time.
4. **DNS rebinding protection** — Network requests are validated against actual resolved IPs, not just hostnames.
5. **Append-only audit log** — All permission decisions are logged to SQLite.

### Q: How does the event bus work?

**A:** Async event bus backed by tokio. Plugins communicate exclusively through events — never direct calls. When a plugin emits an event, the bus routes it to all subscribed handlers. Events can cascade (plugin A's output triggers plugin B), with configurable max depth (default 10) to prevent infinite loops. This keeps plugins fully decoupled — you can swap, add, or remove plugins without touching others.

### Q: How does consensus mode work?

**A:** Multiple reasoning engines (e.g., DeepSeek + Cerebras) receive the same prompt independently. Their proposals are collected, then a synthesis engine merges them into a final response. Configurable minimum proposals and timeout. Useful when you want to cross-validate reasoning or combine different models' strengths.

### Q: The memory system — how is it different from just using a vector DB?

**A:** cpersona is purpose-built for AI agent memory, not generic RAG. Key differences: (1) 3-layer hybrid search (vector + FTS5 + keyword) with RRF merge — catches things pure vector search misses (exact names, IDs, code snippets). (2) Confidence scoring with dynamic time decay that adapts to your corpus time range. (3) Three memory types (declarative, episodic, profile) with different lifecycle semantics. (4) Zero LLM dependency — it's a pure data server. (5) Single SQLite file, MIT license, works standalone outside ClotoCore.

### Q: Why 17 servers? That seems like a lot of surface area.

**A:** Each server is independently optional. The kernel is a stage — you compose only what you need. A research assistant might use 4 servers (reasoning + memory + websearch + terminal). A VTuber AI might use 8 (reasoning + memory + voice + avatar + vision + discord + cron + embedding). The variety exists because the platform is designed for very different agent architectures.

---

## 開発プロセス系

### Q: "$230 total dev cost" — what does that actually mean?

**A:** $230 in Claude subscriptions (Pro → Max) for Claude Code, plus $2 in DeepSeek API for runtime testing. Cerebras free tier for additional testing. The coding itself was done entirely through Claude Code under the subscription — not per-call API credits. That covers ~3 months of active development.

This isn't about being frugal — it's evidence that the barrier to building non-trivial systems has shifted. I design the architecture, review every change, and make all design decisions. Claude Code handles implementation. 51K LOC across Rust and TypeScript, 351 tests, 17 MCP servers, full GUI dashboard.

### Q: "Built with Claude Code" — how much was AI-written?

**A:** Most of the implementation code was generated by Claude Code under my direction. I write the specs, define the architecture, review every change, and make design decisions. Claude Code translates those into working code. The test suite (351 tests) and real-world usage validate correctness. The architecture decisions — event bus design, capability injection model, sandbox approach, MGP protocol design — those are mine.

### Q: Solo developer — can you maintain a 51K LOC project?

**A:** The architecture is designed for this. Plugins are independent — adding a new MCP server doesn't touch the kernel. The kernel itself is ~34K LOC but highly modular (event bus, plugin manager, HTTP API, rate limiter are separate modules). Claude Code handles routine maintenance. I've applied to MITOU (a Japanese government-backed innovation program) for additional support. And the BSL 1.1 → MIT license means the community can fork regardless.

### Q: Why not just contribute to an existing framework like LangChain or AutoGen?

**A:** Different design philosophy. LangChain/AutoGen are orchestration libraries — you write Python/TypeScript code that chains LLM calls. ClotoCore is a platform — you compose agents from plugins through a GUI, with a security model that assumes plugins are untrusted. The closest comparison might be something like Unity for AI agents: a runtime + editor where the building blocks are MCP servers instead of game components.

---

## ビジネス・ライセンス系

### Q: BSL 1.1 — why not just MIT/Apache?

**A:** The kernel is BSL 1.1 to prevent large companies from commercializing the platform without contributing back, while remaining free for individuals, small teams, consultants, educators, and internal tools. It converts to MIT automatically on 2028-02-14. cpersona (the memory server) and MGP (the protocol spec) are MIT today — no restrictions for adoption.

### Q: How do you plan to make money?

**A:** Short term: freelance MCP integration work and consulting. Medium term: commercial licensing for enterprises that need ClotoCore for large-scale deployment. Long term: the platform itself. GitHub Sponsors is available for anyone who wants to support development.

### Q: What's the roadmap?

**A:** Near term: Graph memory for cpersona (entity relationship extraction), expanding the MCP marketplace with community contributions, and improving installer stability for non-developers. Medium term: multi-agent orchestration (agents delegating to other agents), and mobile companion app. The vision doc has the full picture: https://github.com/Cloto-dev/ClotoCore/blob/main/docs/PROJECT_VISION.md

---

## 批判・懐疑系

### Q: How does this compare to OpenClaw?

**A:** Different design philosophy. OpenClaw optimizes for rapid deployment and ecosystem size — 325K+ stars, 13K+ skills. ClotoCore optimizes for security boundaries and controlled execution.

Concrete differences:
- OpenClaw plugins run in-process with the gateway (no sandbox). In ClotoCore, plugins are isolated processes with capability injection — they literally cannot open sockets unless the kernel provides a pre-authorized client.
- OpenClaw's single JSON config means one bad edit breaks all agents. ClotoCore uses per-agent database persistence with typed RBAC.
- OpenClaw's marketplace had 1,184+ malicious skills identified. ClotoCore's marketplace uses Magic Seal binary verification and trust levels.
- OpenClaw defaults to binding on all interfaces. ClotoCore defaults to 127.0.0.1 with DNS rebinding protection.

I respect what OpenClaw has achieved in adoption. The security issues aren't surprising given the scale — they're inherent to the "run everything in-process with host permissions" architecture. ClotoCore chose a different trade-off: slower to adopt, harder to break.

### Q: Can I install it without building from source?

**A:** Pre-built installers exist (Windows .exe, Linux .deb/.AppImage, macOS .dmg) but they're experimental. The setup wizard handles Python venv creation and MCP server download automatically, but environment differences may cause issues. Building from source is more reliable for now. You need Python 3.10+ in PATH for the MCP servers regardless of install method.

### Q: This looks like a lot of half-built features. Is any of it production-ready?

**A:** Fair concern. The core loop (kernel + reasoning + memory + dashboard) is stable and I use it daily. The avatar/voice/vision plugins are functional but less polished — they're proof-of-concept for the platform's extensibility. The 351 tests cover the kernel and memory system thoroughly. I'd call the kernel and cpersona production-ready; the peripheral plugins are working prototypes.

### Q: Why would anyone use this over just calling the Claude API directly?

**A:** If your use case is "send a prompt, get a response," you don't need ClotoCore. The platform is for when you need: persistent memory across sessions, multiple reasoning engines, tool access with security controls, agent personality management, scheduled tasks, or a GUI to manage all of it. It's the difference between making a single API call and running a persistent AI agent.

### Q: 50K LOC written by AI — how do you trust the code quality?

**A:** Same way you trust code from any contributor: tests, code review, and running it. 351 tests across the Rust kernel and Python MCP servers. A formal code quality audit identified 65 findings (2 critical, 19 high) — all critical and high items have been fixed. The Rust compiler itself catches a large class of bugs. And I review every change before merging. AI-generated code isn't magical — it needs the same quality process as human code.

### Q: The Neuro-Sama comparison seems like a stretch.

**A:** It's an inspiration, not a comparison. Neuro-Sama demonstrated that an AI with personality, memory, and real-time interaction can be genuinely compelling. ClotoCore provides the infrastructure to build systems in that direction. The VTuber use case is real — we have VRM avatar support, VOICEVOX TTS, gaze tracking, and Discord I/O. Whether any community creation reaches Neuro-Sama's level of entertainment is up to the builders.
