# ClotoCore コーディング品質評価レポート

**評価日**: 2026-03-16
**対象**: ClotoCore リポジトリ全体 (Rust バックエンド + TypeScript/React フロントエンド + CI/CD)

## 総合スコア: B+ (良好〜優秀)

---

## 1. Rust バックエンド: A-

### スコアカード

| 項目 | 評価 | 詳細 |
|------|------|------|
| エラーハンドリング | S | `.unwrap()` / `.expect()` **ゼロ**。カスタムエラー型 `AppError` + `thiserror` で統一 |
| 型安全性 | S | `ClotoId` ニュータイプ、7変種の `CapabilityType`、9変種の `Permission` enum |
| unsafe コード | A | 全ゼロ。安全な抽象化 (`Arc<RwLock>`, `DashMap`) のみ使用 |
| コード構成 | A | `db/` `handlers/` `managers/` の明確な責任分離、89ファイル |
| async パターン | A | `tokio::sync::broadcast` / `mpsc` 正しく使用、デッドロックなし |
| ドキュメント | S | 公開APIに1,340行のdocコメント |
| 依存関係 | A | Tokio, Axum 0.7, SQLx 0.8 — 標準的で保守的な選択 |
| 命名規約 | A | Rust 慣例に完全準拠 (snake_case / PascalCase / UPPER_SNAKE_CASE) |
| コード重複 | A | `db_timeout()`, `check_auth()`, `spawn_admin_audit()` 等の共通抽出済み |

### 詳細分析

#### エラーハンドリング (模範的)

カスタムエラー型が階層的に設計されている:

```
AppError (crates/core/src/lib.rs)
├── Cloto(ClotoError)     ← ドメインエラー (thiserror)
├── Internal(anyhow::Error) ← 内部エラー (コンテキスト付き)
├── NotFound(String)       ← 404
├── Validation(String)     ← 入力検証
└── Mgp(Box<MgpError>)    ← MGPプロトコルエラー
```

- 全操作が `Result<T>` + `?` 演算子で伝播
- データベース操作に `db_timeout()` ヘルパーでハング防止 (Bug #7 修正)
- 入力検証は詳細なエラーメッセージ付き (`handlers/agents.rs:109-116`)

#### 型安全性 (優秀)

- **ニュータイプ**: `ClotoId` が `Uuid` をラップし、ID混同を型レベルで防止
- **列挙型**: `CapabilityType` (7変種), `Permission` (9変種), `ClotoEventData` (25+変種)
- **結果型エイリアス**: `ClotoResult<T>`, `AppResult<T>` で一貫性確保
- **時刻型**: `DateTime<Utc>` (i64 ではなく)、UUID は `uuid::Uuid` (文字列ではなく)

#### 改善が必要な箇所

| 箇所 | 問題 | 推奨対応 |
|------|------|----------|
| `handlers/marketplace.rs` `run_install()` | 293行 | 関数分割 |
| `handlers/marketplace.rs` `run_batch_install()` | 303行 | 関数分割 |
| `handlers/setup.rs` `run_bootstrap_inner()` | 257行 | 関数分割 |
| `consensus.rs` `on_thought_response()` | 122行 | 関数分割 |
| `config.rs` `AppConfig` | 20+ bool フィールド | ビルダーパターン or サブ構造体 |
| Clippy 警告 | 38件 (軽微) | 段階的修正 |
| ベンチマーク | 2件コンパイルエラー | `registry.plugins` 廃止APIの更新 |

---

## 2. フロントエンド (TypeScript/React): B-

### スコアカード

| 項目 | 評価 | 詳細 |
|------|------|------|
| TypeScript 型付け | A | `strict: true`、`any` **ゼロ**、型定義充実 |
| カスタムフック | A- | `useRemoteData` (キャッシュ+重複排除)、`useAsyncAction` 等 |
| コンポーネント設計 | B- | 構造は良いが巨大コンポーネントが存在 |
| 状態管理 | B | Context API 適切だが一部 prop drilling 残存 |
| i18n | B+ | i18next + 9名前空間。一部ハードコード文字列 |
| CSS/Styling | B- | Tailwind + CSS変数 + ダークモード。クラス名爆発あり |
| API 層 | B- | 型安全だが `api.ts` が663行の一枚岩 |
| エラーハンドリング | C+ | ErrorBoundary あるが silent suppression 散在 |
| コード重複 | C+ | ステータス表示、ボタンスタイル、フォームstateが重複 |

### 詳細分析

#### TypeScript 使用 (優秀)

- `tsconfig.json` で `strict: true` 有効
- コンポーネントコードに `any` 型の使用なし
- 型定義が充実 (`types.ts`):
  - `ChatMessage` — 厳密なコンテンツ型付け
  - `McpServerInfo` — サーバー情報の完全な型定義
  - `AccessControlEntry` — オプショナルフィールド正確にマーク
- ジェネリック関数も適切に型付け (`useApi.ts`)

#### カスタムフック (良好〜優秀)

| フック | 品質 | 特徴 |
|--------|------|------|
| `useRemoteData` | S | モジュールレベルキャッシュ、リクエスト重複排除、TTL管理 |
| `useAsyncAction` | A | 非同期エラーハンドリングの統一パターン |
| `usePolling` | A | クリーンアップ付きポーリング |
| `useEventStream` | B+ | シングルトンSSE管理 (手動再実装) |

#### 改善が必要な箇所

| 箇所 | 問題 | 推奨対応 |
|------|------|----------|
| `AgentConsole.tsx` | 979行の巨大コンポーネント | SSE処理 / メッセージ描画 / アーティファクト表示 / コマンド承認 に分割 |
| `SetupWizard.tsx` | 826行 | ステップごとにサブコンポーネント化 |
| `AgentTerminal.tsx` | 637行 | ターミナル描画とロジックを分離 |
| `services/api.ts` | 663行の一枚岩 | `agents.ts`, `mcp.ts`, `cron.ts` 等にドメイン分割 |
| ステータス表示 | 3箇所で重複実装 | `lib/status.ts` に統合 |
| フォーム状態 | `useState` ×8パターンが複数箇所 | `useForm()` フック抽出 |
| ボタンスタイル | 同一Tailwindクラス列が15+箇所 | スタイルコンポーネント or `@apply` |
| ハードコード文字列 | `SecurityGuard.tsx`, `CommandApprovalCard.tsx` | i18n キーに移行 |
| エラー抑制 | `useEventStream.tsx`, `GeneralSection.tsx` で silent catch | ユーザー通知 or 再スロー |

---

## 3. テスト & CI/CD: B+

### スコアカード

| 項目 | 評価 | 詳細 |
|------|------|------|
| Rust ユニットテスト | S | 162テスト、ソースコード内に分散 |
| Rust 統合テスト | S | 90テスト、13ファイル、2,187行 |
| フロントエンドテスト | D | 14テスト (ユーティリティのみ)。コンポーネントテストなし |
| CI パイプライン | A | Lint + Clippy + audit + テストラチェット + Sentinel |
| リリースパイプライン | A | 8プラットフォーム並列ビルド、cosign署名 |
| セキュリティ監査 | A- | `cargo audit` 有効。GitHub Secret Scanning 未設定 |
| ミューテーションテスト | B- | 導入済みだが `continue-on-error: true` |

### Rust テスト詳細

#### 統合テストファイル一覧

| ファイル | 行数 | 対象領域 |
|----------|------|----------|
| `handlers_http_test.rs` | 285 | HTTP API、エージェント作成、プラグイン設定 |
| `e2e_workflows_test.rs` | 241 | メッセージフロー、権限付与、イベント連鎖 |
| `security_forging_test.rs` | 235 | Identity forgery 防止、悪意あるプラグイン隔離 |
| `event_memory_management_test.rs` | 219 | イベントサイクルのメモリリーク、循環参照 |
| `sse_streaming_test.rs` | 189 | SSE ストリーミング、トークン検証 |
| `permission_elevation_test.rs` | 187 | 動的権限昇格フロー |
| `event_cascading_test.rs` | 186 | カスケードイベント伝播制限 |
| `permission_workflow_test.rs` | 154 | 権限リクエストライフサイクル |
| `plugin_lifecycle_test.rs` | 142 | プラグインパニック隔離、Magic Seal 検証 |
| `concurrent_events_test.rs` | 121 | 競合条件、デッドロック防止、深度制限 |
| `migration_test.rs` | 92 | DB スキーマべき等性 |
| `kernel_integration_test.rs` | 75 | Capability injection、パニック隔離 |
| `system_loop_test.rs` | 61 | コアシステムイベントループ |

#### テストインフラ

- **テストユーティリティ** (`test_utils.rs`): インメモリSQLite + 全マネージャー付き `AppState` 生成
- **モックプラグイン** (`tests/common/`): 正常プラグイン + 意図的パニックプラグイン
- **ベースライン管理** (`qa/test-baseline.json`): Rust 90テスト / Dashboard 14テスト

### CI パイプライン構成

```
ci.yml (Push + PR)
├── Lint        → cargo fmt --check + clippy -D warnings
├── Audit       → cargo audit (脆弱性検出)
├── Test        → cargo test + テスト数ラチェット
├── Sentinel    → テスト削除検知 + アサーションなしテスト検出
├── Mutation    → cargo-mutants (master のみ, continue-on-error)
└── Dashboard   → tsc --noEmit + biome lint + npm build + vitest

release.yml (タグ v*)
├── 8プラットフォーム並列ビルド
├── SHA256 チェックサム + cosign 署名
└── Tauri 自動アップデータ JSON 生成
```

### フロントエンドテストの課題

| 指標 | 値 |
|------|-----|
| テストファイル数 | 3 |
| テスト数 | 14 |
| コード行数 | ~10,500 |
| カバレッジ密度 | **0.1%** |
| コンポーネントテスト | **0** |
| フックテスト | **0** |
| 統合テスト | **0** |

---

## 4. 総合評価

### 強み (維持すべき点)

1. **ゼロ unsafe + ゼロ unwrap** — Rust の安全性を最大限活用
2. **カスタムエラー型の一貫した使用** — anyhow + thiserror の模範的な組み合わせ
3. **セキュリティテストの充実** — forgery, elevation, isolation を網羅
4. **CI 品質ゲート** — Sentinel + テストラチェット + cargo audit の独自防衛線
5. **TypeScript strict + any ゼロ** — フロントエンドの型安全性が高い
6. **1,340行の doc コメント** — 公開 API の文書化が充実

### 弱み (改善推奨)

| 優先度 | 項目 | 影響 | 推奨対応 |
|--------|------|------|----------|
| **高** | フロントエンドテスト不足 | リグレッションリスク大 | コンポーネントテスト + フックテスト追加 (目標: 50+) |
| **高** | 巨大コンポーネント (AgentConsole 979行) | 保守性・可読性低下 | 責務別にサブコンポーネント分割 |
| **中** | api.ts 一枚岩 (663行) | ナビゲーション困難 | ドメイン別ファイル分割 |
| **中** | フォーム状態・ステータス表示の重複 | DRY 違反 | `useForm()` + `lib/status.ts` 抽出 |
| **中** | エラーの silent suppression | 問題の見逃し | 通知 or 再スロー |
| **低** | Clippy 警告 38件 | コード品質の微細な問題 | 段階的修正 |
| **低** | ベンチマークのコンパイルエラー | CI 不通過 (非致命的) | 廃止 API の更新 |

---

## 5. 他プロジェクトとの比較所感

- **エラーハンドリング**: unwrap ゼロは個人〜小規模チーム開発としては異例に高い水準
- **テスト**: Rust 側のセキュリティテスト (forgery, elevation) はエンタープライズ水準
- **CI**: Sentinel (テスト削除検知) は独自のアプローチで、テスト文化の後退を構造的に防止
- **フロントエンド**: TypeScript の型品質は高いが、テスト不足がボトルネック
- **依存関係**: 不必要な依存がなく、全依存が明確な目的を持つ — 保守コスト最小化
