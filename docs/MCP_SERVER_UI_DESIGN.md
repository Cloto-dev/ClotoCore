# MCP Server Management UI Design

> **Status:** Implemented (v0.5.3)
> **Updated:** 2026-03-04
> **Related:** `MCP_PLUGIN_ARCHITECTURE.md` Section 6, `ARCHITECTURE.md`, `SCHEMA.md`

---

## 1. Motivation

### 1.1 背景

バックエンドは MCP-only アーキテクチャに移行済み (`MCP_PLUGIN_ARCHITECTURE.md`)。
旧 Plugin UI (ClotoPluginManager.tsx, AgentPluginWorkspace.tsx, PluginConfigModal.tsx) は
v0.5.3 で完全に削除され、MCP Server Management UI に置き換えられた。

### 1.2 設計判断

**旧 Plugin UI をパッチするのではなく、MCP Server Management UI をゼロから新設した。**

- 旧 Plugin UI のアーキテクチャ自体が MCP の概念と合わない
- God Component / Double-save の根本的な問題を解消
- MCP のサーバーライフサイクル管理は旧プラグインの activate/deactivate と質的に異なる

---

## 2. Design Decisions

| # | 論点 | 選択肢 | 採用 | 根拠 |
|---|------|--------|------|------|
| 1 | アクセス制御の粒度 | サーバー単位 / ツール単位 | **ツール単位** | ツール毎に危険度が異なる (e.g. `execute_command` vs `recall`) |
| 2 | デフォルトポリシー | opt-in / opt-out | **opt-in** (deny by default) | 安全側、サーバー単位で opt-out に変更可能 |
| 3 | レイアウト | Master-Detail / Card Grid + Modal | **Card Grid + Modal** | サイドバー常設レイアウトとの共存、一覧性とモーダル詳細のバランス |
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

### 3.4 `mcp_servers` テーブル拡張

既存の MCP Server 設定に `default_policy` カラムを追加:

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `default_policy` | TEXT | NOT NULL DEFAULT 'opt-in' | `opt-in` (deny by default) / `opt-out` (allow by default) |

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

#### Access Control

| Method | Route | Description |
|--------|-------|-------------|
| GET | `/api/mcp/servers/:id/access` | アクセス制御一覧 (tree 構造) |
| PUT | `/api/mcp/servers/:id/access` | アクセス制御の一括更新 |
| GET | `/api/mcp/access/by-agent/:agent_id` | エージェント視点のアクセス一覧 |

### 4.3 Server Lifecycle

| Method | Route | Description |
|--------|-------|-------------|
| POST | `/api/mcp/servers/:id/restart` | MCP Server 再起動 |
| POST | `/api/mcp/servers/:id/start` | MCP Server 起動 |
| POST | `/api/mcp/servers/:id/stop` | MCP Server 停止 (削除せず) |

---

## 5. UI Design

### 5.1 Card Grid + Modal Detail (v0.5.3)

v0.5.3 でサイドバー常設レイアウト (`AppSidebar`) が導入されたため、
MCP ページは Master-Detail ではなく **カードグリッド + モーダル詳細** パターンを採用。

```
┌──────────────────────────────────────────────────────────────┐
│  🔌 MCP Servers   5 servers · 4 running     [↻] [+ Add Server] │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐           │
│  │ 🔌 tool.    │ │ 🔌 mind.    │ │ 🔌 mind.    │           │
│  │ terminal    │ │ deepseek    │ │ cerebras    │           │
│  │ ● Running   │ │ ● Running   │ │ ○ Stopped   │           │
│  │ 2 tools     │ │ 2 tools SDK │ │ 2 tools SDK │           │
│  └─────────────┘ └─────────────┘ └─────────────┘           │
│  ┌─────────────┐ ┌─────────────┐                            │
│  │ 🔌 memory.  │ │ 🔌 tool.    │                            │
│  │ ks22        │ │ embedding   │                            │
│  │ ● Running   │ │ ● Running   │                            │
│  │ 3 tools SDK │ │ 2 tools     │                            │
│  └─────────────┘ └─────────────┘                            │
│                                                              │
└──────────────────────────────────────────────────────────────┘

クリック → Modal (16:9 large) で詳細表示
```

**カードの情報:**
- サーバーID (font-mono, bold)
- ステータスドット (● Running / ○ Stopped / ● Error)
- ツール数
- バッジ: SDK (ClotoSDK対応), CONFIG (設定ファイルからの読み込み)

**モーダル詳細:**
- Modal (size="lg") に `McpServerDetail` を表示
- 3 タブ構成: Settings / Access / Logs
- ライフサイクル操作ボタン (Start / Stop / Restart / Delete)

### 5.2 Add Server Modal

`Modal` (size="sm") にフォームを表示:
- Server Name (バリデーション: `^[a-z][a-z0-9._-]{0,62}[a-z0-9]$`)
- Command (`python3` デフォルト)
- Arguments (スペース区切り)

### 5.3 Access タブ: Directory 階層 Tree

```
┌─────────────────────────────────────────────────────────────┐
│  Access Control — tool.terminal                              │
│  Default Policy: [opt-in ▼]                                  │
│                                                              │
│  ▼ agent.cloto_default                                        │
│    ├─ 📁 Server Grant: tool.terminal        [Allow ▼]        │
│    │   ├─ 🔧 execute_command                [Deny  ▼]        │
│    │   └─ 🔧 list_processes                 [Allow ▼]  (inherited)
│    └─ ...                                                    │
└─────────────────────────────────────────────────────────────┘
```

### 5.4 Settings タブ

サーバー設定 (command, transport, auto-restart)、環境変数、マニフェスト情報を表示・編集。

---

## 6. Component Architecture (v0.5.3)

### 6.1 現行コンポーネント構成

```
pages/
  McpServersPage.tsx              ← ルートページ (カードグリッド + モーダル管理)

components/
  Modal.tsx                       ← 共有モーダル (size: sm / lg)

components/mcp/
  McpServerDetail.tsx             ← モーダル内: 詳細コンテナ (タブ切替)
  McpServerSettingsTab.tsx        ← Settings タブ
  McpAccessControlTab.tsx         ← Access タブ (Tree + Summary Bar)
  McpAccessTree.tsx               ← ディレクトリ階層ツリー
  McpAccessSummaryBar.tsx         ← ツール別サマリー
  McpServerLogsTab.tsx            ← Logs タブ
  McpServerList.tsx               ← サーバーリスト (レガシー、McpServersPage に統合済み)
```

### 6.2 旧コンポーネント (v0.5.3 で削除済み)

| 旧コンポーネント | 代替 |
|-----------------|------|
| `ClotoPluginManager.tsx` | `McpServersPage.tsx` (カードグリッド) |
| `AgentPluginWorkspace.tsx` | `McpAccessControlTab.tsx` |
| `PluginConfigModal.tsx` | `McpServerSettingsTab.tsx` |
| `McpAddServerModal.tsx` | `McpServersPage.tsx` 内の `Modal` + インラインフォーム |

---

## 7. Implementation Status

### Phase A: バックエンド — 部分完了

- [x] MCP Server CRUD API (`/api/mcp/servers`)
- [x] Server Lifecycle API (start / stop / restart)
- [ ] `mcp_access_control` テーブル作成 (SQLite migration)
- [ ] `mcp_servers` に `default_policy` カラム追加
- [ ] Access control API (settings, access)
- [ ] アクセス解決ロジック (`resolve_access()`)

### Phase B: フロントエンド — 部分完了

- [x] `McpServersPage.tsx` — カードグリッド + Modal レイアウト
- [x] `McpServerDetail.tsx` — タブ構成の詳細ビュー
- [x] `McpServerSettingsTab.tsx` — 設定の CRUD
- [x] `McpServerLogsTab.tsx` — ログ表示
- [ ] `McpAccessControlTab.tsx` — Tree UI + Summary Bar (バックエンド API 待ち)

### Phase C: クリーンアップ — 完了

- [x] 旧 Plugin UI コンポーネント削除
- [x] `types.ts` から旧フィールド削除
