# HN Submission Draft — ClotoCore

## Title candidates

**A (recommended):**
```
Show HN: ClotoCore – A Rust AI agent platform ($230 total dev cost, 50K LOC, solo dev)
```

**B:**
```
Show HN: ClotoCore – Open-source AI agent platform in Rust with GUI dashboard
```

**C:**
```
Show HN: ClotoCore – Build AI agents with sandboxed MCP plugins, Rust kernel, and GUI
```

---

## Body draft

```
Hi HN,

I've been building ClotoCore for the past 3 months — an open-source platform
for constructing AI agents with pluggable capabilities, written in Rust.

AI agents that can execute code, access files, and make network requests are
powerful — and dangerous when poorly contained. Recent incidents with popular
agent frameworks have shown what happens when security is an afterthought:
exposed instances, malicious plugins, no sandboxing. I wanted a platform where
security is the architecture, not a patch.

The idea: instead of monolithic chatbot scripts, you compose an AI agent from
independent plugins — reasoning (DeepSeek, Claude, Ollama), memory (persistent
hybrid search), vision (screen capture, gaze tracking), voice (Whisper STT,
VOICEVOX TTS), avatar (VRM expressions), and I/O (Discord). Plugins are MCP
servers, so you can write them in any language.

The kernel (~34K LOC Rust) handles:
- Event bus for plugin communication (plugins never talk directly)
- Sandboxed capability injection (plugins can't open sockets; the kernel
  provides pre-authorized network access)
- 3-level RBAC for MCP tool access (capability → server → tool)
- API key auth, rate limiting, DNS rebinding protection
- Human-in-the-loop approval for sensitive operations

The dashboard (~17K LOC React/TypeScript) is a Tauri desktop app — agent
management, real-time event stream, chat, cron jobs, permission approvals,
all from a GUI. No CLI required.

17 MCP/MGP servers ship out of the box with 100+ tools. The memory system
(cpersona, MIT licensed) provides 3-layer hybrid search with RRF fusion,
confidence scoring, and episodic/profile memory — all without calling an
LLM internally.

Some numbers:
- ~51K total LOC (34K Rust kernel + 17K TypeScript dashboard)
- 17 MCP servers, 100+ tools
- 351 tests (Rust + Python)
- $232 total cost — $230 in Claude subscriptions (Pro + Max, for Claude Code)
  and $2 in DeepSeek API for runtime testing (Cerebras free tier also used)
- Architecture and code review are mine; implementation is mostly Claude Code

What makes it different: you build autonomous AI agents through a GUI
dashboard — no code required. Pick your reasoning engine, attach memory,
grant tool access with per-tool RBAC, set up scheduled tasks, and deploy.
Every plugin runs in a sandbox with capability injection. Every sensitive
operation requires human approval. The security model isn't bolted on —
it's the foundation everything else is built on.

Quickest way to try it: cpersona (the memory server) works standalone
in Claude Desktop or Claude Code — pip install, single SQLite file, MIT.

The original motivation was building something like Neuro-Sama — an AI
VTuber with real personality, memory, and agency. That's still the vision,
but the platform turned out to be useful for research assistants, automated
workflows, and anything that needs a persistent AI agent with real capabilities.

BSL 1.1 — functionally MIT for individual developers, small teams, and most
commercial use. Only large-scale deployment (>$100K revenue, >1,000 users,
or SaaS) needs approval. I chose BSL to protect ClotoCore as a shared asset
for all developers — not to restrict use, but to ensure no single entity can
capture the platform before the community grows around it. Converts to MIT
on 2028-02-14. cpersona is MIT today.

GitHub: https://github.com/Cloto-dev/ClotoCore
MCP servers: https://github.com/Cloto-dev/cloto-mcp-servers

Happy to answer questions about the architecture, security model, or what
building a 50K LOC system with AI coding assistance actually looks like
in practice.
```

---

## Self-comment (posted by the author immediately after submission)

```
A few notes on the development process:

1. The $230 figure: $230 in Claude subscriptions (Pro → Max) for Claude Code,
   plus $2 in DeepSeek API for runtime testing. Cerebras free tier for
   additional testing. The coding itself was done by Claude Code under the
   subscription — not API calls. This isn't about being frugal — it's evidence
   that the barrier to building non-trivial systems has shifted. Architecture
   decisions, code review, and direction still require a human. But the
   implementation bottleneck is largely gone.

2. Why Rust for the kernel: Memory safety matters when you're running
   arbitrary plugin code. The event bus, capability injection, and sandbox
   model are much easier to reason about when you don't have to worry about
   use-after-free or data races. Compilation catches a lot of design mistakes
   early.

3. Why MCP as the plugin protocol: MCP (Model Context Protocol) is becoming
   the de facto standard for AI tool integration. By building on MCP, any
   server written for Claude Desktop or Claude Code works in ClotoCore with
   zero modification. The reverse is also true — cpersona (our memory server)
   works standalone in Claude Desktop.

4. MGP (Multi-Agent Gateway Protocol): This extends MCP with event-driven
   communication — plugins can emit events, react to other plugins' events,
   and participate in agent-to-agent messaging. The Discord bridge uses MGP
   to inject external messages into the agent loop without the agent knowing
   it's talking to Discord.

5. cpersona works standalone: You don't need ClotoCore to use the memory
   server. Point Claude Desktop or Claude Code at it, done. MIT license,
   single SQLite file, 16 tools, zero LLM dependency. That's the fastest
   way to try the most useful piece.

6. Benchmarks: We tested cpersona against a vector-only baseline on LMEB
   (22 memory retrieval tasks). The hybrid approach (RRF fusion of vector +
   FTS5 + keyword) matches or beats vector-only on 16/22 tasks — with
   QASPER showing +25 NDCG@10 improvement, where FTS5 catches exact names
   and IDs that vector search misses. All without any LLM calls. The delta
   is architecture, not model quality.

Architecture doc: https://github.com/Cloto-dev/ClotoCore/blob/main/docs/ARCHITECTURE.md
```

---

## Submission timing

| Candidate | JST | US Pacific | Day |
|------|---------|-------------|------|
| **Primary** | 4/8 (Wed) 01:00-03:00 | 4/7 (Tue) 09:00-11:00 | Tue |
| Backup 1 | 4/9 (Thu) 01:00-03:00 | 4/8 (Wed) 09:00-11:00 | Wed |
| Backup 2 | 4/10 (Fri) 01:00-03:00 | 4/9 (Thu) 09:00-11:00 | Thu |

---

## Tone check

- [x] "Restrained confidence" — list facts plainly
- [x] No mention of financial hardship
- [x] $230 presented neutrally as a "data point"
- [x] "Built with Claude Code" mentioned in passing
- [x] No hyperbole (avoid "revolutionary", "game-changing", etc.)
- [x] Neuro-Sama reference gives HN readers context (interest from the VTuber crossover audience)
- [x] No direct GitHub Sponsors call-to-action (let the repo side handle that organically)
- [x] License transparency (BSL 1.1 terms made explicit)
