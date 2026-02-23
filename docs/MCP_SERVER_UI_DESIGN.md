# MCP Server Management UI Design

> **Status:** Draft (2026-02-23)
> **Related:** `MCP_PLUGIN_ARCHITECTURE.md` Section 6, `ARCHITECTURE.md`, `SCHEMA.md`
> **Supersedes:** Plugin Manager UI (`ExivPluginManager.tsx`, `AgentPluginWorkspace.tsx`, `PluginConfigModal.tsx`)

---

## 1. Motivation

### 1.1 現状の課題

バックエンドは MCP-only アーキテクチャに移行済み (`MCP_PLUGIN_ARCHITECTURE.md`) だが、
Dashboard の Plugin UI は旧 Rust Plugin SDK 時代のまま残存している:

| 問題 | 該当コンポーネント | 影響 |
|------|---------------------|------|
| MCP Server 管理 UI が存在しない | - | サーバーの起動/停止/設定を API 直叩きでしか行えない |
| `magic_seal` チェックが残存 | `ExivPluginManager.tsx:205` | 旧 `0x56455253` 定数と比較、MCP HMAC 非対応 |
| `SYSTEM_ALWAYS_PLUGINS` ハードコード | `AgentPluginWorkspace.tsx:18` | 旧プラグインID をハードコード |
| God Component (設定モーダル) | `PluginConfigModal.tsx` | 全プラグイン種別を 1 コンポーネントで処理、保守困難 |
| Double-save パターン | `PluginConfigModal.tsx` | activate + config を別 API で保存、競合リスク |
| `sdk_version` / `magic_seal` 型定義 | `types.ts` | 廃止済みフィールドが型に残存 |

### 1.2 設計判断

**旧 Plugin UI をパッチするのではなく、MCP Server Management UI をゼロから新設する。**

- 旧 Plugin UI のアーキテクチャ自体が MCP の概念と合わない
- パッチでは God Component / Double-save の根本的な問題が残る
- MCP のサーバーライフサイクル管理は旧プラグインの activate/deactivate と質的に異なる

---

## 2. Design Decisions

合意済みの設計判断:

| # | 論点 | 選択肢 | 採用 | 根拠 |
|---|------|--------|------|------|
| 1 | アクセス制御の粒度 | サーバー単位 / ツール単位 | **ツール単位** | ツール毎に危険度が異なる (e.g. `execute_command` vs `recall`) |
| 2 | デフォルトポリシー | opt-in / opt-out | **opt-in** (deny by default) | 安全側、サーバー単位で opt-out に変更可能 |
| 3 | レイアウト | Master-Detail / Single-pane | **Master-Detail** | サーバー一覧と詳細を同時に見渡せる |
| 4 | アクセス制御 UI | Matrix / Tree | **Directory 階層 Tree** | エントリの親子関係を直感的に表現 |
| 5 | データモデル | 別テーブル / 統合 | **統合** (`mcp_access_control`) | 旧 `permission_requests` + 新ツールアクセスを一元管理 |

---

## 3. Data Model

### 3.1 新テーブル: `mcp_access_control`

旧 `permission_requests` テーブルと新 MCP ツールアクセス制御を統合する。

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTOINCREMENT | Auto-incrementing ID |
| `entry_type` | TEXT | NOT NULL, CHECK IN ('capability', 'server_grant', 'tool_grant') | エントリ種別 |
| `agent_id` | TEXT | NOT NULL, FK → agents(id) | 対象エージェント |
| `server_id` | TEXT | NOT NULL | MCP Server ID (e.g. `tool.terminal`) |
| `tool_name` | TEXT | | ツール名 (`tool_grant` 時のみ必須) |
| `permission` | TEXT | NOT NULL DEFAULT 'allow' | `allow` / `deny` |
| `granted_by` | TEXT | | 許可者 (UI 操作者 or `system`) |
| `granted_at` | TEXT | NOT NULL | ISO-8601 タイムスタンプ |
| `expires_at` | TEXT | | 有効期限 (NULL = 無期限) |
| `justification` | TEXT | | 許可/拒否の理由 |
| `metadata` | TEXT | | JSON メタデータ |

**Indexes:**
- `(agent_id, server_id, tool_name)` — アクセス解決用
- `(server_id)` — サーバー別一覧
- `(entry_type)` — 種別フィルタ

### 3.2 entry_type の定義

| entry_type | 意味 | server_id | tool_name | ツリー階層 |
|------------|------|-----------|-----------|-----------|
| `capability` | エージェントの機能要求 (旧 `permission_requests` 相当) | 要求先サーバー | NULL | Level 0 (root) |
| `server_grant` | サーバー全体への一括許可/拒否 | 対象サーバー | NULL | Level 1 |
| `tool_grant` | 個別ツールへの許可/拒否 | 対象サーバー | 対象ツール名 | Level 2 |

### 3.3 アクセス解決ロジック (Priority Rule)

エージェントがツールを呼び出す際の許可判定:

```
1. tool_grant が存在する → その permission を使用
2. server_grant が存在する → その permission を使用
3. どちらも存在しない → サーバーの default_policy を使用
     - default_policy = "opt-in"  → deny (デフォルト)
     - default_policy = "opt-out" → allow
```

**優先度: tool_grant > server_grant > default_policy**

```rust
// 疑似コード
fn resolve_access(agent_id: &str, server_id: &str, tool_name: &str) -> Permission {
    // 1. ツール単位の明示的許可を確認
    if let Some(tool_grant) = find_tool_grant(agent_id, server_id, tool_name) {
        return tool_grant.permission;
    }
    // 2. サーバー単位の一括許可を確認
    if let Some(server_grant) = find_server_grant(agent_id, server_id) {
        return server_grant.permission;
    }
    // 3. サーバーのデフォルトポリシーを適用
    match server.default_policy {
        "opt-out" => Permission::Allow,
        _         => Permission::Deny,  // opt-in (default)
    }
}
```

### 3.4 `mcp_servers` テーブル拡張

既存の MCP Server 設定に `default_policy` カラムを追加:

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `default_policy` | TEXT | NOT NULL DEFAULT 'opt-in' | `opt-in` (deny by default) / `opt-out` (allow by default) |

### 3.5 旧テーブルの扱い

| テーブル | 移行 |
|---------|------|
| `permission_requests` | `mcp_access_control` に `entry_type = 'capability'` として移行後、削除 |
| `plugin_settings.allowed_permissions` | `mcp_access_control` に `server_grant` として展開後、カラム削除を検討 |

---

## 4. API Design

### 4.1 既存 API (維持)

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers` | MCP Server 一覧 (status, tools 含む) |
| POST | `/api/mcp/servers` | MCP Server 登録 |
| DELETE | `/api/mcp/servers/:id` | MCP Server 停止・削除 |

### 4.2 新規 API

#### Server Settings

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers/:id/settings` | サーバー設定取得 (config, default_policy) |
| PUT | `/api/mcp/servers/:id/settings` | サーバー設定更新 |

**GET Response:**

```json
{
  "server_id": "tool.terminal",
  "default_policy": "opt-in",
  "config": {
    "SANDBOX_DIR": "/tmp/exiv-sandbox",
    "COMMAND_TIMEOUT": "120"
  },
  "auto_restart": true
}
```

#### Access Control

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers/:id/access` | アクセス制御一覧 (tree 構造) |
| PUT | `/api/mcp/servers/:id/access` | アクセス制御の一括更新 |
| GET | `/api/mcp/access/by-agent/:agent_id` | エージェント視点のアクセス一覧 |

**GET `/api/mcp/servers/:id/access` Response:**

```json
{
  "server_id": "tool.terminal",
  "default_policy": "opt-in",
  "tools": ["execute_command", "list_processes"],
  "entries": [
    {
      "entry_type": "server_grant",
      "agent_id": "agent.exiv_default",
      "permission": "allow",
      "granted_by": "user",
      "granted_at": "2026-02-23T10:00:00Z"
    },
    {
      "entry_type": "tool_grant",
      "agent_id": "agent.exiv_default",
      "tool_name": "execute_command",
      "permission": "deny",
      "granted_by": "user",
      "granted_at": "2026-02-23T10:05:00Z"
    }
  ]
}
```

**PUT `/api/mcp/servers/:id/access` Request:**

```json
{
  "entries": [
    {
      "entry_type": "server_grant",
      "agent_id": "agent.exiv_default",
      "permission": "allow"
    },
    {
      "entry_type": "tool_grant",
      "agent_id": "agent.exiv_default",
      "tool_name": "execute_command",
      "permission": "deny"
    }
  ]
}
```

### 4.3 Server Lifecycle

| Method | Route | Description |
|--------|-------|-------------|
| POST | `/api/mcp/servers/:id/restart` | MCP Server 再起動 |
| POST | `/api/mcp/servers/:id/start` | MCP Server 起動 |
| POST | `/api/mcp/servers/:id/stop` | MCP Server 停止 (削除せず) |

---

## 5. UI Design

### 5.1 Master-Detail Layout

```
┌──────────────────────────────────────────────────────────────┐
│  MCP Server Management                              [+ Add]  │
├──────────────────┬───────────────────────────────────────────┤
│                  │                                           │
│  MCP Servers     │  tool.terminal                            │
│                  │  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  │
│  ● tool.terminal │                                           │
│  ● mind.deepseek │  Status: ● Running                        │
│  ● mind.cerebras │  Uptime: 2h 34m                           │
│  ○ core.ks22     │  Tools: 2 registered                      │
│  ● core.embedding│                                           │
│                  │  [Start] [Stop] [Restart]                  │
│                  │                                           │
│  ● = Running     │  ┌─ Settings ─┬─ Access ─┬─ Logs ─┐      │
│  ○ = Stopped     │  │            │          │        │      │
│  ◉ = Error       │  │  (tab content below)  │        │      │
│                  │  └────────────────────────────────┘      │
│                  │                                           │
├──────────────────┴───────────────────────────────────────────┤
│  Status Bar: 5 servers | 4 running | 1 stopped               │
└──────────────────────────────────────────────────────────────┘
```

**左ペイン (Master):**
- MCP Server 一覧 (ステータスインジケータ付き)
- `● Running` / `○ Stopped` / `◉ Error` の視覚的表示
- クリックで右ペインに詳細を表示
- `[+ Add]` ボタンで新規サーバー登録

**右ペイン (Detail):**
- 選択されたサーバーの詳細
- 3 タブ構成: Settings / Access / Logs
- ライフサイクル操作ボタン (Start / Stop / Restart)

### 5.2 Access タブ: Directory 階層 Tree

```
┌─────────────────────────────────────────────────────────────┐
│  Access Control — tool.terminal                              │
│  Default Policy: [opt-in ▼]                                  │
│                                                              │
│  ┌─ Summary Bar ──────────────────────────────────────────┐  │
│  │  execute_command: 1 agent allowed, 1 denied            │  │
│  │  list_processes:  2 agents allowed                     │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  Agent: [agent.exiv_default ▼]                               │
│                                                              │
│  ▼ agent.exiv_default                                        │
│    ├─ 🔑 Capability: NetworkAccess          [Approved]       │
│    ├─ 📁 Server Grant: tool.terminal        [Allow ▼]        │
│    │   ├─ 🔧 execute_command                [Deny  ▼]        │
│    │   └─ 🔧 list_processes                 [Allow ▼]  (inherited)
│    └─ 📁 Server Grant: mind.deepseek        [Allow ▼]        │
│        ├─ 🔧 think                          [Allow ▼]  (inherited)
│        └─ 🔧 think_with_tools              [Allow ▼]  (inherited)
│                                                              │
│  Legend:                                                     │
│  [Allow ▼] = 明示的許可  |  (inherited) = 親定義を継承       │
│  [Deny  ▼] = 明示的拒否  |  ドロップダウンで変更可能         │
└─────────────────────────────────────────────────────────────┘
```

**ツリー構造:**

| レベル | アイコン | 表示内容 | 操作 |
|--------|----------|----------|------|
| 0 (root) | 🔑 | `capability` — エージェントの機能要求 | Approve / Deny |
| 1 | 📁 | `server_grant` — サーバー一括許可 | Allow / Deny ドロップダウン |
| 2 | 🔧 | `tool_grant` — ツール個別許可 | Allow / Deny ドロップダウン |

**動作:**
- `server_grant` の 📁 をクリック/展開すると、配下のツール一覧が表示される
- ツールに明示的な `tool_grant` がない場合は `(inherited)` と表示
- ツールのドロップダウンを変更すると `tool_grant` が作成される
- `(inherited)` 状態に戻すと `tool_grant` が削除される (親に従う)

### 5.3 Summary Bar

Access タブ上部に表示。ツール横断でエージェントのアクセス状況を一覧する:

```
┌─ Summary Bar ──────────────────────────────────────────────┐
│  Tool              Allowed    Denied     Inherited          │
│  ──────────────    ───────    ──────     ─────────          │
│  execute_command   1 agent    1 agent    0 agents           │
│  list_processes    2 agents   0 agents   1 agent            │
└────────────────────────────────────────────────────────────┘
```

- ツール名をクリックすると、そのツールに関するエージェント一覧にフィルタリング
- Matrix UI の「横方向参照」を Tree UI で実現する手段

### 5.4 Settings タブ

```
┌─────────────────────────────────────────────────────────────┐
│  Settings — tool.terminal                                    │
│                                                              │
│  Server Configuration                                        │
│  ────────────────────                                        │
│  Command:    [python -m exiv_mcp_terminal    ]               │
│  Transport:  [stdio ▼]                                       │
│  Auto-restart: [✓]                                           │
│                                                              │
│  Environment Variables                                       │
│  ────────────────────                                        │
│  SANDBOX_DIR     [/tmp/exiv-sandbox          ]               │
│  COMMAND_TIMEOUT [120                        ]               │
│                                         [+ Add Variable]     │
│                                                              │
│  Manifest                                                    │
│  ────────                                                    │
│  ID:       tool.terminal                                     │
│  Version:  0.1.0                                             │
│  Category: Tool                                              │
│  Tags:     #TOOL, #EXECUTION                                 │
│  Tools:    execute_command, list_processes                    │
│                                                              │
│                              [Save Changes] [Reset]          │
└─────────────────────────────────────────────────────────────┘
```

---

## 6. Component Architecture

### 6.1 コンポーネント構成

```
pages/
  McpServersPage.tsx              ← ルートページ

components/mcp/
  McpServerList.tsx               ← 左ペイン: サーバー一覧
  McpServerDetail.tsx             ← 右ペイン: 詳細コンテナ
  McpServerSettingsTab.tsx        ← Settings タブ
  McpAccessControlTab.tsx         ← Access タブ (Tree + Summary Bar)
  McpAccessTree.tsx               ← ディレクトリ階層ツリー
  McpAccessSummaryBar.tsx         ← ツール別サマリー
  McpServerLogsTab.tsx            ← Logs タブ
  McpAddServerModal.tsx           ← 新規登録モーダル
```

### 6.2 旧コンポーネントの廃止

| 旧コンポーネント | 状態 | 代替 |
|-----------------|------|------|
| `ExivPluginManager.tsx` | 削除 | `McpServerList.tsx` + `McpServerDetail.tsx` |
| `AgentPluginWorkspace.tsx` | 削除 | `McpAccessControlTab.tsx` |
| `PluginConfigModal.tsx` | 削除 | `McpServerSettingsTab.tsx` |

---

## 7. Migration Plan

### Phase A: バックエンド

1. `mcp_access_control` テーブル作成 (SQLite migration)
2. `mcp_servers` に `default_policy` カラム追加
3. 新 API エンドポイント実装 (settings, access, lifecycle)
4. アクセス解決ロジック (`resolve_access()`) を MCP Client Manager に統合
5. 旧 `permission_requests` データを `mcp_access_control` に移行

### Phase B: フロントエンド

1. `McpServersPage.tsx` + Master-Detail レイアウト実装
2. `McpServerList.tsx` — SSE でリアルタイムステータス更新
3. `McpServerSettingsTab.tsx` — 設定の CRUD
4. `McpAccessControlTab.tsx` — Tree UI + Summary Bar
5. `McpServerLogsTab.tsx` — サーバーログのストリーミング表示

### Phase C: クリーンアップ

1. 旧 Plugin UI コンポーネント削除 (`ExivPluginManager.tsx`, `AgentPluginWorkspace.tsx`, `PluginConfigModal.tsx`)
2. `types.ts` から `magic_seal`, `sdk_version` 等の旧フィールド削除
3. `permission_requests` テーブル削除 (migration)
4. ドキュメント更新 (`SCHEMA.md`, `CHANGELOG.md`)
