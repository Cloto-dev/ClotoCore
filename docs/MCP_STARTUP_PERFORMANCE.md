# MCP Server Startup Performance Analysis

**分析日**: 2026-03-17
**対象**: カーネル起動時の MCP サーバー接続フロー

## サーバー構成 (mcp.toml)

| サーバーID | カテゴリ | auto_restart | 起動対象 |
|-----------|---------|-------------|---------|
| tool.terminal | Tool | true | Priority |
| tool.agent_utils | Tool | true | Priority |
| tool.cron | Tool | true | Priority |
| tool.embedding | Tool | true | Priority |
| tool.websearch | Tool | true | Priority |
| tool.research | Tool | true | Priority |
| mind.deepseek | Engine | true | Priority |
| mind.cerebras | Engine | true | Priority |
| mind.claude | Engine | true | Priority |
| mind.ollama | Engine | true | Priority |
| memory.cpersona | Memory | true | Priority |
| vision.gaze_webcam | Vision | false | Deferred |
| vision.capture | Vision | false | Deferred |
| tool.imagegen | Tool | false | Deferred |
| voice.stt | Voice | false | Deferred |

**合計**: 15サーバー (11 Priority / 4 Deferred)
**全サーバー**: Python stdio

## 接続ライフサイクル (per server)

`connect_server()` (`mcp.rs:462-1065`) の各ステップ:

| # | ステップ | 所要時間 | タイムアウト | ブロッキング |
|---|---------|---------|------------|-------------|
| 1 | コマンド検証 | <1ms | — | No |
| 2 | Permission Gate D | 0-500ms (YOLO) | DB 10s | Yes |
| 3 | Seal検証 | 0-500ms | — | Conditional |
| 4 | Isolation Profile | <5ms | — | No |
| 5 | **プロセス起動** | **500ms-2s** | — | **Yes** |
| 6 | **Initialize RPC** | **1-5s** | **120s** | **Yes** |
| 7 | MGP Negotiation | <1ms | — | No |
| 8 | MGP Permission Flow | 0-500ms (YOLO) | 120s | Conditional |
| 9 | Initialized通知 | <1ms | — | No |
| 10 | **tools/list RPC** | **100-500ms** | **120s** | **Yes** |
| 11 | Cloto Handshake | 100-500ms | 120s | Optional |
| 12 | 登録 + インデックス | <50ms | — | No |
| 13 | Capability Dispatcher | 10-50ms | — | No |
| 14 | Audit/Lifecycle | <1ms | — | No (spawn) |

**1サーバーあたり正常時**: 2-8秒
**1サーバーあたり最悪時**: 360秒 (3回リトライ × 120秒タイムアウト)

## 起動タイムライン

### シナリオ A: 初回起動 (venv未作成)

```
  0s ─── カーネル起動、Config/DB初期化
  2s ─── Plugin Manager 初期化
  5s ─── ensure_mcp_venv() 開始
         ├── python -m venv 作成: 1-3s
         ├── pip upgrade: 2-10s
         └── 15サーバー依存 pip install (順次): 30-75s ← ★最大ボトルネック
 80s ─── Priority MCP接続 (11台並列) ← join_all で並列化済み
         ├── プロセス起動: ~2s (最も遅い1台)
         ├── Initialize RPC: ~2s
         └── tools/list: ~1s
 86s ─── HTTP サーバー起動 → ダッシュボードアクセス可能
 86s ─── Deferred MCP接続 (4台バックグラウンド)
 92s ─── 全サーバー接続完了
```

**合計**: **80-95秒** (ボトルネック: venv依存インストール)

### シナリオ B: 通常起動 (venv既存)

```
  0s ─── カーネル起動、Config/DB初期化
  2s ─── Plugin Manager 初期化
  5s ─── ensure_mcp_venv() — venv検出 + 依存sync
         └── pip install --quiet × 15 (no-op): 15-30s ← ★ボトルネック
 35s ─── Priority MCP接続 (11台並列)
         └── 並列接続: ~5s
 40s ─── HTTP サーバー起動
 46s ─── 全サーバー接続完了
```

**合計**: **35-46秒** (ボトルネック: pip no-op チェック)

### シナリオ C: 理想的起動 (venv最適化後)

```
  0s ─── カーネル起動
  2s ─── Config/DB
  5s ─── venv確認のみ (pip install スキップ)
  6s ─── Priority MCP接続 (11台並列): ~5s
 11s ─── HTTP サーバー起動
 17s ─── 全サーバー接続完了
```

**合計**: **11-17秒**

## ボトルネック分析

### CRITICAL: venv依存同期 (`mcp_venv.rs:116-165`)

```rust
// install_server_deps() — 全サーバーを順次 pip install
for entry in entries.flatten() {
    pip install <server_path> --quiet  // 2-5秒/サーバー × 15 = 30-75秒
}
```

- **毎回起動時に実行** (no-op でも pip の依存解決に 2-3秒/サーバー)
- **順次実行** — 並列化されていない
- **全サーバー対象** — auto_restart=false のサーバーも含む

### HIGH: リクエストタイムアウト 120秒 (`config.rs:302-310`)

```rust
CLOTO_MCP_REQUEST_TIMEOUT_SECS = 120  // デフォルト
```

- Initialize RPC, tools/list, permission RPC 全てに適用
- 1台のサーバーが応答しないだけで 120秒ブロック
- リトライ3回で最大 360秒

### HIGH: Python インタープリタ起動 (per server)

- Python venv の `python.exe` 起動: 500ms-2s
- import 解決 (共通ライブラリ含む): 追加 500ms-1s
- 11台並列でも最も遅い1台が全体を律速

### MODERATE: tools/list RPC

- サーバーのツール登録数に依存
- 通常 100-500ms だが、大規模ツールセットでは 1-5s

## 推奨最適化

### 優先度 HIGH

| # | 施策 | 削減効果 | 実装コスト |
|---|------|---------|-----------|
| 1 | **venv依存syncをauto_restartサーバーのみに限定** | -15-30s | 低 |
| 2 | **pip install を並列化** (tokio::spawn × N) | -20-50s | 低 |
| 3 | **venv syncをバックグラウンド化** (HTTP起動後) | 体感 -30-75s | 中 |
| 4 | **初回後の依存syncスキップ** (ハッシュベースキャッシュ) | -15-30s | 中 |

### 優先度 MEDIUM

| # | 施策 | 削減効果 | 実装コスト |
|---|------|---------|-----------|
| 5 | **接続タイムアウトを 30s に短縮** (起動時のみ) | 最悪時 -270s | 低 |
| 6 | **tool schema キャッシュ** (DB保存、変更時のみ再取得) | -1-5s | 中 |
| 7 | **Python プロセスプール** (事前起動) | -5-10s | 高 |

### 優先度 LOW

| # | 施策 | 削減効果 | 実装コスト |
|---|------|---------|-----------|
| 8 | Seal検証の並列化 | <1s | 低 |
| 9 | Permission Gate Dのバッチ化 | <1s | 中 |

## タイムアウト設定一覧

| 設定 | デフォルト | 用途 | ファイル |
|------|-----------|------|---------|
| `CLOTO_MCP_REQUEST_TIMEOUT_SECS` | 120s | 全RPC (initialize, tools/list等) | config.rs:302 |
| `CLOTO_DB_TIMEOUT_SECS` | 10s | Permission DBチェック | config.rs:269 |
| `CLOTO_MEMORY_TIMEOUT_SECS` | 5s | メモリプラグイン | config.rs |
| `CLOTO_TOOL_TIMEOUT_SECS` | 30s | ツール実行 | config.rs |
| リトライバックオフ | 1s, 2s | connect_server リトライ | mcp.rs:681 |

## 関連ファイル

- 起動オーケストレーション: `crates/core/src/lib.rs:400-590`
- サーバー接続: `crates/core/src/managers/mcp.rs:311-356, 462-1065`
- MCP クライアント: `crates/core/src/managers/mcp_client.rs:56-270`
- トランスポート: `crates/core/src/managers/mcp_transport.rs:111-249`
- Venv管理: `crates/core/src/managers/mcp_venv.rs:116-267`
- タイムアウト設定: `crates/core/src/config.rs:269-310`
