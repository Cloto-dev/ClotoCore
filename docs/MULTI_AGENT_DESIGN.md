# Multi-Agent Delegation — v0.5.x Design

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-03
**Target:** v0.5.x

---

## 1. Overview

ClotoCore のエージェントが他のエージェントに問い合わせ・委譲を行うための
マルチエージェント協調システム。Agent-as-Tool (ツール公開) と
Event-Driven Delegation (イベント委譲) のハイブリッドアーキテクチャで実装する。

### 1.1 Design Principles

- **Core Minimalism**: カーネルは委譲のルーティングと安全制御のみ担当
- **Event-First**: 内部実装はイベントバス経由で疎結合に実現
- **Permission Isolation**: 委譲先エージェントは自身の権限のみで動作（権限の継承なし）
- **Loop Prevention**: 委譲深度の上限と呼び出しチェーン追跡で無限ループを防止

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

### UC-1: Character Interaction (キャラクター間会話)

**Layer:** Casual / Entertainment
**Priority:** Medium

Neuro-Sama と Evil Neuro のように、複数の AI キャラクターが互いに会話する。
ユーザーが「サフィーとアシスタントで〇〇について議論して」と指示すると、
エージェント間で複数ターンの対話が発生する。

**Flow:**

```
User: "サフィーとリサーチャーで、Rustの将来性について議論して"
  │
  ├── Agent A (Sapphy): "私はRustの安全性が最大の強みだと思う。
  │   リサーチャーさんはどう思う？"
  │     │
  │     └── ask_agent("researcher", "Rustの将来性について意見を聞かせて。
  │           私は安全性が強みだと主張しています")
  │           │
  │           └── Agent B (Researcher): "データで見ると、Stack Overflow の
  │                 調査で8年連続 most loved language です。ただし..."
  │
  ├── Agent A: リサーチャーの意見を踏まえて反論/同意
  │     │
  │     └── ask_agent("researcher", "その点について...")
  │
  └── Agent A: 議論をまとめてユーザーに報告
```

**Requirements:**
- 複数ターンの委譲を許可 (ただし max_delegation_depth 以内)
- 各エージェントが自身の人格を維持した応答を生成
- ユーザーに両方のエージェントの発言が可視化される (SSE イベント)

**UI Considerations:**
- チャット画面に複数エージェントのアイコン/名前が表示される
- 委譲中は「Agent B に問い合わせ中...」のインジケーター表示
- 議論の各ターンがタイムラインとして閲覧可能

---

### UC-2: Specialist Consultation (専門家への相談)

**Layer:** Casual / Practical
**Priority:** High

メインのキャラクターエージェントが、自身の知識外の質問を受けた際に
専門特化エージェントに裏で問い合わせ、キャラクターの口調で回答する。

**Flow:**

```
User: "今日の夕食、何がいいかな？"
  │
  └── Agent A (Sapphy — general personality):
        │
        ├── "料理のことはあの子に聞いてみるね！"
        │
        ├── ask_agent("chef", "ユーザーが夕食のメニューを相談しています。
        │     季節は3月、簡単に作れるものを3つ提案してください")
        │     │
        │     └── Agent B (Chef — cooking specialist):
        │           "1. 菜の花のペペロンチーノ 2. 春キャベツの回鍋肉 3. ..."
        │
        └── Agent A: Chef の提案をサフィーの口調で再構成して回答
              "聞いてきたよ！3月だから春の食材がいいみたい！
               菜の花のペペロンチーノとか、春キャベツの回鍋肉とか..."
```

**Requirements:**
- 1 対 1 の単発委譲 (最もシンプルなパターン)
- 委譲先の応答を委譲元が自身の人格で再解釈
- 委譲先エージェントの存在をユーザーに明示するかは設定次第

**Design Notes:**
- Agent A のシステムプロンプトに「専門分野外は ask_agent で委譲せよ」と指示
- Agent B は raw な専門情報を返す (人格不要、精度重視)
- Agent A が人格フィルタとして機能する

---

### UC-3: Second Opinion (セカンドオピニオン)

**Layer:** Casual / Quality
**Priority:** Medium

ユーザーの質問に対して、メインエージェントが自身の回答を生成した後、
別エージェントの意見も取り入れて最終回答を補強する。

**Flow:**

```
User: "このコードのパフォーマンス、大丈夫かな？"
  │
  └── Agent A (Main):
        │
        ├── [自身で初期分析] "O(n²) のループがありますね..."
        │
        ├── ask_agent("reviewer", "以下のコードのパフォーマンスについて
        │     私は O(n²) の問題を指摘しました。他に見落としはありますか？
        │     コード: ...")
        │     │
        │     └── Agent B (Reviewer):
        │           "O(n²) の指摘は正しい。追加で、メモリアロケーションが
        │            ループ内で発生している点も問題です..."
        │
        └── Agent A: 両方の分析を統合して最終回答
              "O(n²) の問題に加えて、もう一つ見つかりました。
               ループ内のメモリアロケーションも..."
```

**Requirements:**
- Agent A が先に自身の回答を生成し、その後で委譲
- 委譲プロンプトに Agent A の分析結果を含める (コンテキスト共有)
- 最終回答は Agent A が統合責任を持つ

**Design Notes:**
- ConsensusOrchestrator (エンジン協調) の上位概念
- ConsensusOrchestrator = 同じプロンプトを複数エンジンに投げる
- UC-3 = Agent A の分析を踏まえて Agent B が補完する (非対称)

---

### UC-4: Task Decomposition (MCP権限ベースのタスク分割)

**Layer:** Technical / Productivity
**Priority:** High

複雑なタスクを、MCP サーバーのアクセス権限に基づいて複数の
専門エージェントに分割・委譲する。各エージェントは自身に
grant されたツールのみを使用する。

**Flow:**

```
User: "最新のRustセキュリティ情報を調べて、うちのCargo.tomlに影響があるか確認して"
  │
  └── Agent A (Coordinator — no MCP tools):
        │
        ├── ask_agent("researcher", "最新のRustセキュリティアドバイザリを
        │     調査してください。CVE番号、影響クレート、パッチバージョンを
        │     リストアップしてください")
        │     │
        │     └── Agent B (Researcher — websearch MCP granted):
        │           [web_search tool] → "RustSec Advisory: CVE-2026-XXXX..."
        │
        ├── ask_agent("developer", "以下のセキュリティアドバイザリが
        │     Cargo.toml に影響するか確認してください。
        │     アドバイザリ: ... Cargo.toml を読んで確認してください")
        │     │
        │     └── Agent C (Developer — terminal MCP granted):
        │           [terminal: cat Cargo.toml] → [terminal: cargo audit]
        │           → "影響あり: クレート X のバージョン Y.Z"
        │
        └── Agent A: 調査結果と影響分析を統合してレポート
```

**Requirements:**
- Coordinator は自身ではツールを持たない (純粋な指揮役)
- 各 worker エージェントは自身の granted MCP サーバーのみ使用
- 委譲は並列実行可能 (独立したサブタスクの場合)
- Coordinator が結果を統合して最終レポートを生成

**Design Notes:**
- 最小権限原則の自然な実現: 各エージェントに必要最小限の権限のみ付与
- 新しいタスク種別が増えても、MCP サーバーを grant するだけで拡張可能
- `mcp_access_control` テーブルの既存インフラがそのまま活用できる

**Parallel Delegation:**
```
Agent A ─┬── ask_agent("researcher", ...) ──► Agent B ──┐
         │                                               ├── Agent A: synthesize
         └── ask_agent("developer", ...)  ──► Agent C ──┘
```

---

### UC-5: Review / Verification (レビュー・検証)

**Layer:** Technical / Quality
**Priority:** Medium

あるエージェントの出力を、別のレビュー特化エージェントが検証する。
コード生成、翻訳、分析など、品質保証が重要なタスクに適用する。

**Flow:**

```
User: "ユーザー認証のミドルウェアを書いて"
  │
  └── Agent A (Developer — terminal MCP granted):
        │
        ├── [コード生成] auth_middleware.rs を作成
        │
        ├── ask_agent("reviewer", "以下の認証ミドルウェアのコードを
        │     レビューしてください。セキュリティ脆弱性、エッジケースの
        │     見落とし、パフォーマンス問題を指摘してください。
        │     コード: ```rust ... ```")
        │     │
        │     └── Agent B (Reviewer — no MCP tools, reasoning-focused):
        │           "問題点: 1. タイミング攻撃に対して脆弱 (constant-time
        │            比較を使用すべき) 2. トークンの有効期限チェックが..."
        │
        ├── [Agent A: レビュー指摘を反映してコードを修正]
        │
        └── Agent A: "レビューを反映して修正しました。変更点: ..."
```

**Requirements:**
- Reviewer は読み取り専用 (コード変更権限なし)
- レビュー結果に基づく修正は元のエージェントが実施
- 複数ラウンドのレビューサイクルが可能

**Design Notes:**
- UC-2 (専門家相談) の特殊形: 委譲先が「検証」に特化
- Reviewer エージェントには reasoning-heavy なエンジン (DeepSeek) を割り当て
- Developer エージェントには tool-capable なエンジン (Cerebras) を割り当て
- エンジンの得意分野とエージェントの役割を一致させる設計

---

### UC-6: Cross-Engine Collaboration (エンジン横断協調)

**Layer:** Technical / Advanced
**Priority:** Low

異なる LLM エンジンを持つエージェント間で協調し、各エンジンの
得意分野を活かしたタスク処理を行う。ConsensusOrchestrator の
エージェントレベル拡張。

**Flow:**

```
User: "このアルゴリズムの計算量を分析して、改善案を実装して"
  │
  └── Agent A (Analyst — DeepSeek engine, reasoning-focused):
        │
        ├── [深い推論] "現在の計算量は O(n³) です。
        │     動的計画法で O(n²) に改善可能です。
        │     状態遷移: dp[i][j] = ..."
        │
        ├── ask_agent("implementer", "以下のアルゴリズム改善案を
        │     実装してください。現在: O(n³), 改善: O(n²) DP。
        │     状態遷移: dp[i][j] = ... テストケースも作成してください")
        │     │
        │     └── Agent B (Implementer — Cerebras engine, tool-capable):
        │           [terminal: create file] → [terminal: run tests]
        │           → "実装完了。テスト5/5通過。ベンチマーク: 340ms → 12ms"
        │
        └── Agent A: 分析と実装結果を統合して報告
              "O(n³) → O(n²) への改善を完了しました。
               実測で 28x の高速化を確認..."
```

**Requirements:**
- 各エージェントが異なる LLM エンジンを使用
- 推論品質とツール実行能力の分離
- エンジン特性に基づくタスクの最適割り当て

**Design Notes:**
- ConsensusOrchestrator との違い:
  - Consensus: 同じプロンプトを複数エンジンに投げて統合
  - UC-6: 異なるプロンプト/タスクを各エンジンの得意分野に割り当て
- エンジンルーティング (v0.4.x) との統合:
  - 既存のルーティングルールはメッセージ単位
  - UC-6 はタスク単位でエージェント (= エンジン) を選択

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
