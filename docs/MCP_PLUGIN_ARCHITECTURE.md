# MCP Plugin Architecture (v2)

> **Status:** Approved (2026-02-23)
> **Supersedes:** Three-Tier Plugin Model (Rust/Python Bridge/WASM) → Two-Layer Model (Rust Core + MCP)
> **Related:** `ARCHITECTURE.md` Section 3, `WASM_PLUGIN_DESIGN.md` (historical)

---

## 1. Motivation

### 1.1 現状の課題

Exiv のプラグインシステムは当初 Three-Tier Model として設計された:

| Tier | 状態 | 保守コスト |
|------|------|-----------|
| Tier 1: Rust Native | 稼働中 (6 plugins) | SDK, macros, inventory, registry, factory, cast |
| Tier 2: Python Bridge | 削除済み | - |
| Tier 3: WASM | 未実装 (設計文書のみ) | - |

個人開発において、Rust Plugin SDK の保守負荷が大きい:

- `crates/shared/` — 5 つのプラグイントレイト定義
- `crates/macros/` — `#[exiv_plugin]` 手続きマクロ
- `plugins/` — 6 つの Rust プラグイン実装
- `managers/plugin.rs` — PluginManager (ファクトリ, ブートストラップ, 権限注入)
- `managers/registry.rs` — PluginRegistry (ディスパッチ, タイムアウト, セマフォ)
- `inventory` クレート — コンパイル時自動登録
- Magic Seal 検証 (`0x56455253`)
- Capability Injection (SafeHttpClient, FileCapability, ProcessCapability)

### 1.2 設計判断

**Rust Plugin SDK を全廃し、MCP (Model Context Protocol) を唯一のプラグイン規格とする。**

- **コアは Rust で「信頼」を売り、プラグインは MCP で「門戸」を開く**
- Kernel は MCP クライアント・オーケストレーターに特化する
- 設計原則 1.1 (Core Minimalism) の究極的な実現

### 1.3 選定根拠

代替案の検討結果:

| 案 | 構成 | 判定 | 理由 |
|----|------|------|------|
| A | Rust + MCP | **採用** | 保守性, 動的生成, エコシステム, 実装コスト |
| B | Rust + WASM (MCP interface) | 不採用 | 初期実装コスト高, 動的生成困難 |
| C | Rust + WASM + MCP (hybrid) | 不採用 | 3 層に戻り保守性悪化 |
| D | Go + MCP | 不採用 | Tauri 喪失, リライトコスト |
| E | Rust + Go + MCP | 不採用 | 2 言語保守, ~350 行のために過剰 |

---

## 2. Architecture

### 2.1 Two-Layer Model

```
┌──────────────────────────────────────────────────────┐
│  Layer 1: Rust Core (Kernel)                          │
│                                                        │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────┐ │
│  │ Axum     │ │ SQLite   │ │ Event    │ │ Tauri   │ │
│  │ HTTP     │ │ Database │ │ Bus      │ │ Desktop │ │
│  └──────────┘ └──────────┘ └──────────┘ └─────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ MCP Client Manager                               │ │
│  │  - Server lifecycle (spawn / stop / restart)      │ │
│  │  - Tool routing & dispatch                        │ │
│  │  - Manifest management                            │ │
│  │  - Magic Seal verification (HMAC)                 │ │
│  │  - Event → MCP Notification forwarding            │ │
│  └──────────────────────────────────────────────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ Chat Pipeline                                     │ │
│  │  - MCP Tool "think" 呼び出し (従来の ReasoningEngine) │
│  │  - MCP Tool "store" / "recall" (従来の MemoryProvider)│
│  └──────────────────────────────────────────────────┘ │
│  ┌──────────────────────────────────────────────────┐ │
│  │ Evolution Engine (変更なし)                        │ │
│  └──────────────────────────────────────────────────┘ │
└───────────────────────┬──────────────────────────────┘
                        │
                        │ MCP (JSON-RPC 2.0 over stdio)
                        │
┌───────────────────────▼──────────────────────────────┐
│  Layer 2: MCP Servers (任意言語)                       │
│                                                        │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────────┐ │
│  │ mind.*      │ │ core.*      │ │ tool.*          │ │
│  │ (Reasoning) │ │ (Memory)    │ │ (Execution)     │ │
│  │ deepseek    │ │ ks22        │ │ terminal        │ │
│  │ cerebras    │ │ moderator   │ │ (user plugins)  │ │
│  └─────────────┘ └─────────────┘ └─────────────────┘ │
└──────────────────────────────────────────────────────┘
```

### 2.2 Request Flow

```
User Message
  │
  ├─ POST /api/chat
  │    │
  │    ├─ Kernel: Chat Pipeline
  │    │    │
  │    │    ├─ MCP Client Manager → mind.deepseek の "think" Tool 呼び出し
  │    │    │    └─ JSON-RPC: {"method": "tools/call", "params": {"name": "think", ...}}
  │    │    │    └─ Response: {"result": {"content": [{"type": "text", "text": "..."}]}}
  │    │    │
  │    │    ├─ MCP Client Manager → core.ks22 の "store" Tool 呼び出し
  │    │    │
  │    │    └─ Event Bus → SSE broadcast
  │    │
  │    └─ JSON Response → Client
```

### 2.3 Event Flow

```
Kernel Event (e.g., ConfigUpdated)
  │
  ├─ Event Bus → SSE subscribers (Dashboard)
  │
  └─ MCP Client Manager → 全 MCP Server に Notification 送信
       │
       ├─ mind.deepseek:  notifications/exiv.event { type: "ConfigUpdated", ... }
       ├─ core.ks22:      notifications/exiv.event { type: "ConfigUpdated", ... }
       └─ tool.terminal:  notifications/exiv.event { type: "ConfigUpdated", ... }
```

---

## 3. MCP Protocol Usage

### 3.1 標準 MCP 機能の活用

| MCP Primitive | Exiv での用途 |
|---------------|--------------|
| **Tools** | プラグイン機能の主要表現 (think, store, recall, execute_command) |
| **Resources** | 読み取り専用データ公開 (metrics, status) |
| **Prompts** | テンプレートプロンプト (将来拡張) |
| **Notifications** | Kernel → Server イベント転送 |

### 3.2 Exiv 固有拡張 (Custom Methods)

MCP 標準を最大限活用しつつ、以下の Exiv 固有メソッドを定義する:

| Method | Direction | Purpose |
|--------|-----------|---------|
| `exiv/handshake` | Client → Server | マニフェスト交換 + Magic Seal 検証 |
| `exiv/shutdown` | Client → Server | Graceful shutdown 要求 |

**Notification (Server → Client):**

| Notification | Purpose |
|-------------|---------|
| `notifications/exiv.event` | Kernel イベントの転送 |
| `notifications/exiv.config_updated` | プラグイン設定変更の通知 |

### 3.3 従来トレイトの MCP Tool マッピング

#### ReasoningEngine → MCP Tools

```json
{
  "name": "think",
  "description": "Process a message and generate a response",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "string" },
      "context": {
        "type": "array",
        "items": { "type": "object" }
      }
    },
    "required": ["agent_id", "message"]
  }
}
```

```json
{
  "name": "think_with_tools",
  "description": "Process a message with available tool schemas",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "string" },
      "context": { "type": "array" },
      "tools": { "type": "array" },
      "tool_history": { "type": "array" }
    },
    "required": ["agent_id", "message"]
  }
}
```

#### MemoryProvider → MCP Tools

```json
{
  "name": "store",
  "description": "Store a message in agent memory",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "message": { "type": "object" }
    },
    "required": ["agent_id", "message"]
  }
}
```

```json
{
  "name": "recall",
  "description": "Recall relevant memories for a query",
  "inputSchema": {
    "type": "object",
    "properties": {
      "agent_id": { "type": "string" },
      "query": { "type": "string" },
      "limit": { "type": "integer", "default": 10 }
    },
    "required": ["agent_id", "query"]
  }
}
```

#### Tool → MCP Tools

```json
{
  "name": "execute_command",
  "description": "Execute a shell command in sandboxed directory",
  "inputSchema": {
    "type": "object",
    "properties": {
      "command": { "type": "string" },
      "args": { "type": "array", "items": { "type": "string" } },
      "timeout_secs": { "type": "integer", "default": 120 }
    },
    "required": ["command"]
  }
}
```

---

## 4. MCP Server Manifest

### 4.1 Manifest Structure

各 MCP Server は `exiv/handshake` で以下のマニフェストを返す:

```json
{
  "id": "mind.deepseek",
  "name": "DeepSeek Reasoning Engine",
  "description": "DeepSeek API reasoning engine with R1 support",
  "version": "0.1.0",
  "sdk_version": "0.1.0",
  "category": "Agent",
  "service_type": "Reasoning",
  "tags": ["#MIND", "#LLM"],
  "required_permissions": ["NetworkAccess"],
  "provided_capabilities": ["Reasoning"],
  "provided_tools": ["think", "think_with_tools"],
  "seal": "<HMAC-SHA256 signature>"
}
```

### 4.2 Naming Convention (維持)

| Namespace | 用途 | 例 |
|-----------|------|-----|
| `mind.*` | 推論エンジン (LLM) | `mind.deepseek`, `mind.cerebras` |
| `core.*` | コアシステム (記憶, 制御) | `core.ks22`, `core.moderator` |
| `tool.*` | ツール実行 | `tool.terminal`, `tool.web-search` |
| `adapter.*` | 外部プロトコルブリッジ | `adapter.discord`, `adapter.slack` |
| `vision.*` | 視覚/知覚 | `vision.screen`, `vision.gaze` |
| `hal.*` | ハードウェア抽象化 | `hal.audio`, `hal.gpio` |

---

## 5. Magic Seal (MCP)

### 5.1 従来方式 (廃止)

```rust
// Rust コンパイル時定数 — 廃止予定
magic_seal: 0x56455253  // ASCII: "VERS"
```

### 5.2 新方式: HMAC 署名マニフェスト

```
MCP Server 起動 → Kernel が exiv/handshake を呼び出し
                → Server がマニフェスト + HMAC 署名を返却
                → Kernel が HMAC を検証
                → 検証成功 → 接続確立
                → 検証失敗 → 接続拒否
```

**署名生成:**

```
seal = HMAC-SHA256(
  key  = EXIV_SDK_SECRET,
  data = canonical_json(manifest without "seal" field)
)
```

**EXIV_SDK_SECRET:**

- Exiv MCP SDK パッケージに埋め込み
- 公式 SDK を使用したことの軽量証明
- 暗号学的な改竄防止ではなく「信頼の表明」(従来の Magic Seal と同程度)

### 5.3 Unsigned Mode

開発時は `EXIV_ALLOW_UNSIGNED=true` で署名検証をスキップ可能。
本番環境ではデフォルトで署名必須。

---

## 6. Dynamic Plugin Creation (L5)

### 6.1 エージェントによる自律生成

```
Agent (L5 Autonomy)
  │
  ├─ 1. MCP Server コード生成
  │      Python + exiv-mcp-sdk を使用
  │      Tool 定義 + ビジネスロジック
  │
  ├─ 2. Kernel がコードを検証
  │      - AST セキュリティ検査
  │      - マニフェスト妥当性チェック
  │      - Permission 要求の妥当性確認
  │
  ├─ 3. Kernel が MCP Server をサブプロセス起動
  │      - stdio transport (プロセス隔離)
  │      - MCP handshake + Magic Seal 検証
  │
  ├─ 4. Tool が Kernel に登録
  │      - Chat Pipeline で利用可能に
  │      - Dashboard に表示
  │
  └─ 5. 不要時に破棄
        - プロセス kill + 登録解除
```

### 6.2 MCP Server 管理 API

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers` | 登録済み MCP Server 一覧 |
| POST | `/api/mcp/servers` | MCP Server 登録 (手動 / 動的) |
| DELETE | `/api/mcp/servers/:id` | MCP Server 停止・登録解除 |
| POST | `/api/mcp/servers/:id/restart` | MCP Server 再起動 |

---

## 7. MCP Client Manager

### 7.1 概要

現在の `adapter.mcp` プラグインを Kernel コア機能に昇格させる。

**責務:**

1. MCP Server のライフサイクル管理 (spawn / monitor / restart / stop)
2. MCP JSON-RPC クライアント (Tool 呼び出し, Notification 送信)
3. Tool ルーティング (Tool 名 → 適切な MCP Server へディスパッチ)
4. マニフェスト管理 + Magic Seal 検証
5. Kernel Event → MCP Notification 変換・転送
6. Server 死活監視 + 自動再起動

### 7.2 設定

```toml
# MCP Server 設定 (DB または設定ファイル)
[[mcp.servers]]
id = "mind.deepseek"
command = "python"
args = ["-m", "exiv_mcp_deepseek"]
env = { DEEPSEEK_API_KEY = "${DEEPSEEK_API_KEY}" }
transport = "stdio"
auto_restart = true

[[mcp.servers]]
id = "tool.terminal"
command = "exiv-mcp-terminal"
transport = "stdio"
auto_restart = true
```

---

## 8. Security Model

### 8.1 変更点

| 機能 | Rust Native (旧) | MCP (新) | 影響 |
|------|------------------|---------|------|
| SafeHttpClient | Kernel が注入 | MCP Server が自前管理 | セキュリティ低下 |
| FileCapability | サンドボックス化 | OS レベル制限 | 同等 (実装依存) |
| ProcessCapability | allowlist 強制 | MCP Server 内で制限 | セキュリティ低下 |
| メモリ隔離 | Rust 型安全 | プロセス隔離 | 同等 |
| Magic Seal | コンパイル時定数 | HMAC 署名 | 同等 |

### 8.2 緩和策

- **個人開発前提**: 全 MCP Server は自身で記述 → 信頼の前提が成立
- **サードパーティ対応時**: OS レベルサンドボックス (seccomp, AppArmor) の導入を検討
- **Magic Seal**: 公式 SDK を使用していない MCP Server の接続を拒否可能

### 8.3 維持される安全機構

- API Key 認証 (Dashboard ↔ Kernel)
- Event depth check (cascade 防止, max 5)
- Rate limiter (HTTP リクエスト制限)
- CORS origin 制限
- MCP Server プロセス隔離 (OS レベル)

---

## 9. Deprecated Components

本アーキテクチャ移行完了後に削除されるもの:

| Component | Path | Reason |
|-----------|------|--------|
| Plugin SDK | `crates/shared/src/lib.rs` (Plugin, ReasoningEngine, Tool, MemoryProvider, CommunicationAdapter traits) | MCP に置換 |
| Plugin Macros | `crates/macros/` | MCP マニフェストに置換 |
| Plugin Implementations | `plugins/deepseek/`, `plugins/cerebras/`, `plugins/ks22/`, `plugins/moderator/`, `plugins/terminal/`, `plugins/mcp/` | MCP Server として再実装 |
| PluginManager | `crates/core/src/managers/plugin.rs` | MCP Client Manager に置換 |
| PluginRegistry | `crates/core/src/managers/registry.rs` | MCP Client Manager に統合 |
| PluginFactory pattern | `crates/shared/` | 不要 |
| PluginCast | `crates/shared/` | 不要 |
| inventory crate | `Cargo.toml` | 不要 |
| Capability Injection | `crates/core/src/capabilities.rs` | MCP Server 自前管理 |
| Magic Seal 0x56455253 | `crates/shared/`, `crates/core/` | HMAC 署名に置換 |
| WASM Plugin Design | `docs/WASM_PLUGIN_DESIGN.md` | Historical reference として残存 |

---

## 10. Migration Plan

### Phase 1: tool.terminal → MCP Server

**目的**: 最も単純なプラグインで MCP 化の実証を行う。

1. `tool.terminal` を MCP Server として再実装 (Python or Rust)
2. MCP Client Manager の基礎実装 (adapter.mcp 拡張)
3. Kernel から MCP Tool `execute_command` を呼び出し可能にする
4. 既存の Rust `tool.terminal` と並行運用して動作確認

### Phase 2: mind.deepseek → MCP Server

**目的**: ReasoningEngine → MCP Tool の変換パターンを確立する。

1. `mind.deepseek` を MCP Server として再実装
2. Chat Pipeline を MCP Tool `think` 呼び出しに変更
3. `think_with_tools` の MCP Tool としての動作検証
4. Config 変更通知 (exiv/config_updated) の実装

### Phase 3: 残り全プラグイン移行

1. `mind.cerebras` → MCP Server
2. `core.ks22` → MCP Server (store/recall Tools)
3. `core.moderator` → Kernel 内ロジック吸収 or MCP Server

### Phase 4: Rust Plugin SDK 削除

1. `crates/shared/` からプラグイントレイト削除
2. `crates/macros/` 削除
3. `plugins/` ディレクトリ削除 (MCP Server は別リポジトリ or `mcp-servers/` に配置)
4. PluginManager, PluginRegistry 削除
5. `inventory` クレート依存削除
6. `ARCHITECTURE.md` 更新 (本ドキュメントを参照)

### Phase 5: 動的プラグイン生成

1. MCP Server 管理 API 実装
2. Magic Seal (HMAC) 実装
3. エージェント L5 による MCP Server 自律生成の実装
4. Dashboard の MCP Server 管理 UI

---

## 11. Trade-offs (許容済み)

| 損失 | 影響度 | 許容理由 |
|------|--------|---------|
| Capability Injection | 中 | 個人開発で全 Server を自身が記述 → 信頼前提が成立 |
| コンパイル時型安全 | 小 | JSON-RPC 境界での型検証は MCP SDK レベルで担保 |
| JSON-RPC オーバーヘッド | 無視可 | LLM API 呼出しが数百 ms、IPC の数 ms は誤差 |
| Zero-copy イベントディスパッチ | 小 | イベント頻度は低い (数回/秒以下) |

---

## 12. Future Considerations

- **Exiv MCP SDK**: Python / Node / Rust 向けの公式 SDK パッケージ提供
- **MCP Server マーケットプレイス**: コミュニティ製 MCP Server の配布基盤
- **MCP Sampling**: MCP 仕様の Sampling 機能成熟後、Kernel 側の推論呼び出しを標準化
- **サードパーティ対応**: OS レベルサンドボックスの導入 (seccomp / AppArmor)
