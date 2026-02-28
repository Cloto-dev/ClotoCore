# MGP Adoption Guide — クライアント実装と採用戦略

**Companion to:** `MGP_SPEC.md`
**Status:** 構想（Implementation Concept）
**Date:** 2026-02-28

---

## 1. ライセンスと配布戦略

### 1.1 ライセンス分離

| コンポーネント | ライセンス | リポジトリ |
|--------------|-----------|-----------|
| MGP仕様書 (`MGP_SPEC.md`) | MIT | `mgp-spec` (独立) |
| MGP SDK (Python / TypeScript) | MIT | `mgp-sdk` (独立) |
| MGP バリデーションツール | MIT | `mgp-sdk` に同梱 |
| ClotoCore (リファレンス実装) | BSL 1.1 → MIT (2028) | `ClotoCore` (既存) |

MGP仕様とSDKはClotoCoreから完全に分離し、MIT ライセンスで公開する。
これにより、ClotoCoreの商用保護期間に関係なく、任意のプロジェクトがMGPを採用可能。

### 1.2 独立リポジトリ構成

```
mgp-spec/
├── spec/
│   ├── MGP_SPEC.md          ← プロトコル仕様（メイン）
│   ├── MGP_PATTERNS.md      ← アプリケーションパターン
│   └── MGP_ADOPTION.md      ← 本ドキュメント
├── sdk/
│   ├── python/              ← Python SDK
│   │   ├── mgp/
│   │   │   ├── __init__.py
│   │   │   ├── client.py    ← MGPClient
│   │   │   ├── server.py    ← enable_mgp()
│   │   │   ├── security.py  ← 権限・バリデーション
│   │   │   ├── discovery.py ← ツール検索
│   │   │   └── types.py     ← 型定義
│   │   └── pyproject.toml
│   └── typescript/          ← TypeScript SDK
│       ├── src/
│       │   ├── index.ts
│       │   ├── client.ts
│       │   ├── server.ts
│       │   ├── security.ts
│       │   ├── discovery.ts
│       │   └── types.ts
│       └── package.json
├── validator/               ← 準拠テストツール
│   └── mgp-validate         ← CLI
├── examples/                ← サンプル実装
│   ├── minimal-server/      ← Tier 1 最小MGPサーバー
│   ├── secure-server/       ← Tier 2 セキュリティ対応
│   └── full-server/         ← Tier 4 フル実装
├── LICENSE                  ← MIT
└── README.md
```

---

## 2. 段階的採用パス

### 2.1 Tier 概要

MGPは全ての拡張を一度に実装する必要がない。クライアントもサーバーも段階的に
採用できる。各Tierは前のTierを含む。

```
Tier 1 ──── Tier 2 ──── Tier 3 ──── Tier 4
 数時間      1週間      2-4週間     1-2ヶ月
 最小対応    基本安全    通信強化    フル機能
```

**Layer対応:** Tier 1-2 は主に Layer 1 (メタデータ) と Layer 2 (通知)。
Tier 3-4 は Layer 3 (プロトコルメソッド) と Layer 4 (Kernelツール) を含む。
Kernelツール (Layer 4) はサーバー側の実装不要 — カーネルが標準MCPツールとして提供する。

### 2.2 Tier 1 — 最小対応（数時間）

**目標:** MGPネゴシエーションに参加し、セキュリティメタデータを読み取る。

**クライアント側の変更:**
1. `initialize` リクエストの `capabilities` に `mgp` フィールドを追加
2. サーバーレスポンスの `mgp` フィールドを読み取り
3. `tools/list` の `security` フィールドからリスクレベルを表示

**サーバー側の変更:**
1. `initialize` レスポンスの `capabilities` に `mgp` フィールドを追加
2. `tools/list` の各ツールに `security` オブジェクトを追加

**変更量:** JSON フィールドの追加のみ。既存コードへの破壊的変更ゼロ。

**SDK 使用例（サーバー側 Python）:**

```python
# Before: 標準MCP
from mcp.server import Server
server = Server("my-tool")

# After: MGP Tier 1 対応（3行追加）
from mgp import enable_mgp
enable_mgp(server,
    permissions=["network.outbound"],
    trust_level="standard"
)
# → initialize レスポンスに mgp capabilities を自動追加
# → ツール定義に security メタデータを自動付与
```

**SDK 使用例（クライアント側 TypeScript）:**

```typescript
// Before: 標準MCPクライアント
const client = new MCPClient();
await client.initialize(server);

// After: MGP Tier 1 対応
import { MGPClient } from '@mgp/sdk';
const client = new MGPClient({ extensions: ['security'] });
const { mgpSupported, tools } = await client.initialize(server);
// tools[0].security?.risk_level → "dangerous"
// mgpSupported → true/false
```

### 2.3 Tier 2 — 基本セキュリティ（1週間）

**追加機能:**
- 権限承認フロー（§3）
- 監査イベント送信（§6）
- 構造化エラーハンドリング（§14）
- アクセス制御階層（§5）

**SDK 使用例:**

```python
from mgp import MGPClient, ApprovalPolicy

client = MGPClient(
    extensions=["security", "access_control", "audit"],
    approval_policy=ApprovalPolicy.INTERACTIVE  # or AUTO_APPROVE
)

# 権限チェックは SDK が自動処理
result = await client.initialize(server)
# → permissions_required を検出
# → ポリシーに基づき承認/拒否
# → 監査イベントを自動送信
```

### 2.4 Tier 3 — 通信強化（2-4週間）

**追加機能:**
- ライフサイクル管理（§11）— ヘルスチェック、シャットダウン
- ストリーミング（§12）— チャンク受信、進捗表示
- 双方向通信（§13）— イベント購読、コールバック

**SDK 使用例:**

```python
from mgp import MGPClient

client = MGPClient(extensions=["security", "lifecycle", "streaming"])

# ストリーミングツール呼び出し
async for chunk in client.call_tool_stream("think", {"message": "..."}):
    print(chunk.text, end="", flush=True)

# ヘルスチェック
health = await client.health_check(server_id="mind.cerebras")
# → { status: "healthy", uptime_secs: 3600 }
```

### 2.5 Tier 4 — フル機能（1-2ヶ月）

**追加機能:**
- 動的ツール検索（§16 Mode A）
- 能動的ツール要求（§16 Mode B）
- コンテキストバジェット管理
- セッションツールキャッシュ

**SDK 使用例:**

```python
from mgp import MGPClient, ContextBudget

client = MGPClient(
    extensions=["security", "lifecycle", "streaming", "tool_discovery"],
    context_budget=ContextBudget(
        max_tokens=8000,
        pinned_tools=["think", "store", "recall"]
    )
)

# Mode A: 意図ベース検索
tools = await client.discover_tools("read file contents")
# → [{ name: "read_file", relevance: 0.95, ... }]

# Mode B: 能動的要求
result = await client.request_tools(
    reason="capability_gap",
    requirements={"capabilities": ["statistics"]}
)
# → tools_loaded: [{ name: "analyze_csv", ... }]
```

**Tier 4 準拠要件の明確化:**

- Mode A（動的ツール検索）: キーワードまたはカテゴリ検索の実装 — **セマンティック検索は不要**
- Mode B（能動的ツール要求）: capability gap 検出と `mgp.tools.request` Kernelツールの呼出し
- セッションツールキャッシュ: ピン留め + キャッシュの管理
- コンテキストバジェット: トークン制限の適用

セマンティック検索（ベクトル類似度）は**オプション強化機能**であり、Tier 4 準拠の
必須要件ではない。キーワード + カテゴリ検索のみで十分。

---

## 3. 実装難易度マトリクス

### 3.1 クライアント実装

| 拡張 | Tier | コード量目安 | 難易度 | 依存関係 |
|------|------|-------------|--------|----------|
| §2 ネゴシエーション | 1 | ~50行 | 極低 | なし |
| §4 セキュリティメタデータ読取 | 1 | ~30行 | 極低 | §2 |
| §3 権限承認 | 2 | ~200行 | 低 | §2 |
| §14 エラー処理 | 2 | ~100行 | 低 | なし |
| §6 監査送信 | 2 | ~80行 | 低 | §2 |
| §5 アクセス制御 (Kernel Tool) | 2 | ~300行 (kernel) / ~0行 (server) | 中 | §2 |
| §11 ライフサイクル (Kernel Tool) | 3 | ~200行 (kernel) / ~80行 (server: health response) | 低〜中 | §2 |
| §12 ストリーミング | 3 | ~400行 | 中 | §2 |
| §13 双方向 | 3 | ~500行 | 中 | §2 |
| §15 ディスカバリ (Kernel Tool) | 3 | ~150行 (kernel) / ~0行 (server) | 低 | §2 |
| §16 ツール検索 (Kernel Tool) | 4 | ~800-1500行 (kernel) / ~0行 (server) | 中〜高 | §2, §15 |

**Tier 1 合計: ~80行の追加で MGP 対応。**

### 3.2 サーバー実装

| 拡張 | Tier | コード量目安 | 難易度 |
|------|------|-------------|--------|
| §2 ネゴシエーション応答 | 1 | ~40行 | 極低 |
| §4 セキュリティメタデータ宣言 | 1 | ~20行/ツール | 極低 |
| §3 権限宣言 | 1 | ~10行 | 極低 |
| §11 ヘルスチェック応答 | 3 | ~80行 | 低 |
| §12 ストリーミング送信 | 3 | ~200行 | 中 |
| §13 イベント発行 | 3 | ~150行 | 低〜中 |

**サーバー側は Tier 1 で ~70行の追加。** ツールの `security` フィールドを宣言するだけ。

---

## 4. SDK 設計方針

### 4.1 設計原則

1. **ゼロコンフィグ** — インポートして1関数呼ぶだけで Tier 1 対応
2. **段階的拡張** — 必要な拡張だけ有効化。不要な機能のコードは読み込まれない
3. **MCPライブラリ非侵入** — 既存の MCP SDK をラップ。フォークや改変は不要
4. **型安全** — TypeScript では完全な型定義。Python では dataclass + type hints

### 4.2 Python SDK 構成

```
mgp/
├── __init__.py          # enable_mgp(), MGPClient をエクスポート
├── types.py             # MGPCapabilities, SecurityMetadata, AuditEvent 等
├── negotiate.py         # ケーパビリティネゴシエーション処理
├── security.py          # 権限管理、バリデータ、アクセス制御
├── lifecycle.py         # ヘルスチェック、シャットダウン
├── streaming.py         # チャンク処理、進捗レポート
├── discovery.py         # ツール検索、セッションキャッシュ、コンテキストバジェット
├── audit.py             # 監査イベント構築・送信
├── errors.py            # MGPエラーコード、リカバリロジック
└── server.py            # enable_mgp() サーバーラッパー
```

### 4.3 TypeScript SDK 構成

```
@mgp/sdk/
├── src/
│   ├── index.ts         # メインエクスポート
│   ├── types.ts         # 全型定義
│   ├── client.ts        # MGPClient クラス
│   ├── server.ts        # enableMGP() ラッパー
│   ├── security.ts      # 権限・バリデーション
│   ├── lifecycle.ts     # ヘルスチェック
│   ├── streaming.ts     # ストリーム処理
│   ├── discovery.ts     # ツール検索
│   ├── audit.ts         # 監査
│   └── errors.ts        # エラーハンドリング
└── package.json         # @mgp/sdk として npm 公開
```

---

## 5. バリデーションツール

### 5.1 概要

`mgp-validate` は、MGPサーバーまたはクライアントの準拠度をテストするCLIツール。

**"5分でMGP対応サーバー" 体験:** `mgp-validate` と最小サンプルサーバー
（`examples/minimal-server/`）を使えば、5分以内にMGP対応サーバーを起動し、
準拠テストを通すことができる。これがMGPの採用障壁の低さを実証する最も効果的な方法であり、
仕様書単体よりも圧倒的に訴求力がある。

### 5.2 使用例

```bash
# サーバーの準拠テスト
mgp-validate server ./my-server.py
# ✓ Tier 1: Capability negotiation ... PASS
# ✓ Tier 1: Security metadata on tools ... PASS
# ✓ Tier 2: Permission declarations ... PASS
# ✗ Tier 3: Health check response ... MISSING
# ✗ Tier 3: Streaming support ... MISSING
# Result: Tier 2 compliant (6/11 extensions)

# クライアントの準拠テスト
mgp-validate client ./my-client
# ✓ Sends mgp capabilities in initialize ... PASS
# ✓ Reads security metadata ... PASS
# Result: Tier 1 compliant

# 相互接続テスト
mgp-validate pair ./my-client ./my-server
# ✓ Negotiation succeeds ... PASS
# ✓ Graceful degradation with MCP server ... PASS
# ✓ Security metadata flows through ... PASS
```

### 5.3 準拠バッジ

Tier 達成に応じてプロジェクトに表示可能なバッジを提供:

```
[MGP Tier 1] [MGP Tier 2] [MGP Tier 3] [MGP Tier 4]
```

---

## 6. 採用シナリオ

### 6.1 既存 MCP クライアントへの MGP 追加

```
Day 1:  mgp SDK をインストール
        initialize に mgp フィールドを追加 → Tier 1 達成

Week 1: 権限承認 UI を追加
        監査ログを表示 → Tier 2 達成

Month 1: ストリーミング対応
         ヘルスチェック表示 → Tier 3 達成

Month 2+: ツール検索 UI
          コンテキストバジェット → Tier 4 達成
```

### 6.2 新規プロジェクトでの MGP 採用

**最速パス:** 以下の10行コードで MGP 対応サーバーが完成する。
`mgp-validate server` で準拠確認まで5分以内。

```python
# 10行で MGP 対応サーバーが完成
from mcp.server import Server
from mcp.types import Tool, TextContent
from mgp import enable_mgp

server = Server("my-tool")
enable_mgp(server, permissions=["memory.read"], trust_level="standard")

@server.tool(security={"risk_level": "safe"})
async def hello(name: str) -> str:
    return f"Hello, {name}!"
```

### 6.3 段階的な組織導入

```
Phase 1: 社内ツールに Tier 1 を適用（全ツールにリスクレベルを付与）
Phase 2: 権限管理を導入（誰がどのツールを使えるか制御）
Phase 3: 監査体制を構築（全ツール呼び出しを記録）
Phase 4: ツール検索を導入（大規模ツールエコシステムの管理）
```

---

## 7. 他プロジェクトとの関係

| プロジェクト | MGP との関係 |
|-------------|-------------|
| MCP (Anthropic) | MGP のベースプロトコル。MGP は MCP の厳密なスーパーセット |
| Claude Code | 標準 MCP クライアント。MGP Tier 1 対応で即座にセキュリティメタデータ活用可能 |
| Cursor | 40ツール制限あり。MGP §16 で制限を実質的に解消可能 |
| LangChain / LlamaIndex | ツールフレームワーク。MGP SDK をアダプタとして組み込み可能 |
| OpenAI Function Calling | 別プロトコルだが、MGP のセキュリティ概念（リスクレベル等）は参考にできる |

---

## 8. ロードマップ（構想）

| フェーズ | 成果物 | 状態 |
|---------|--------|------|
| Phase 0 | MGP 仕様書 (MGP_SPEC.md) | ドラフト完了 |
| Phase 1 | Python SDK (Tier 1-2) | 構想 |
| Phase 2 | TypeScript SDK (Tier 1-2) | 構想 |
| Phase 3 | バリデーションツール | 構想 |
| Phase 4 | SDK Tier 3-4 拡張 | 構想 |
| Phase 5 | 独立リポジトリ公開 + npm/PyPI 公開 | 構想 |
| Phase 6 | ClotoCore を MGP リファレンス実装に移行 | 構想 |
