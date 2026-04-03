# HN投稿ドラフト — ClotoCore

## タイトル候補

**A (推奨):**
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

## 本文ドラフト

```
Hi HN,

I've been building ClotoCore for the past 3 months — an open-source platform
for constructing AI agents with pluggable capabilities, written in Rust.

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
- $230 total development cost (API credits for LLMs used during coding)
- Built almost entirely with Claude Code

The original motivation was building something like Neuro-Sama — an AI
VTuber with real personality, memory, and agency. That's still the vision,
but the platform turned out to be useful for research assistants, automated
workflows, and anything that needs a persistent AI agent with real capabilities.

BSL 1.1 license (free for most use cases, converts to MIT in 2028).
Memory system (cpersona) is MIT.

GitHub: https://github.com/Cloto-dev/ClotoCore
MCP servers: https://github.com/Cloto-dev/cloto-mcp-servers

Solo developer from Japan. Happy to answer questions about the architecture,
security model, or the experience of building a 50K LOC project with AI
coding assistance.
```

---

## self-comment（投稿直後に自分で書くコメント）

```
A few notes on the development process:

1. The $230 figure: This is the total spend on API credits (Claude, DeepSeek,
   Cerebras) used during development. Claude Code did the vast majority of
   implementation work — I focus on architecture decisions and code review.
   This isn't a brag about being cheap; it's a data point on what AI-assisted
   development looks like in 2026.

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

## 投稿タイミング

| 候補 | 日本時間 | US西海岸時間 | 曜日 |
|------|---------|-------------|------|
| **第一候補** | 4/8 (水) 01:00-03:00 | 4/7 (火) 09:00-11:00 | 火曜 |
| 予備1 | 4/9 (木) 01:00-03:00 | 4/8 (水) 09:00-11:00 | 水曜 |
| 予備2 | 4/10 (金) 01:00-03:00 | 4/9 (木) 09:00-11:00 | 木曜 |

---

## トーンチェック

- [x] 「抑制された自信」— 事実を淡々と列挙
- [x] 経済的困窮への言及なし
- [x] $230は「data point」として中立的に提示
- [x] "Built with Claude Code" はさらっと明記
- [x] 過激表現なし（"revolutionary", "game-changing" 等の排除）
- [x] Neuro-Samaへの言及はHN読者に文脈を提供（VTuberクロスオーバー層の関心）
- [x] GitHub Sponsorsへの直接誘導なし（リポジトリ側で自然に導線）
- [x] ライセンスの透明性（BSL 1.1の条件を明示）
