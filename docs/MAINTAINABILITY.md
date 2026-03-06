# ClotoCore Maintainability Report

保守性の包括的スキャン結果と改善アクションリストを記録したドキュメントです。
開発の意思決定や技術的負債の管理に役立てるため、継続的に更新してください。

**最終更新**: 2026-03-04
**調査対象バージョン**: 0.5.3

---

## 1. プロジェクト構造概要

```
ClotoCore/
├── crates/
│   ├── core/        # カーネル・ハンドラー・DB・managers/・ミドルウェア
│   └── shared/      # 共有トレイト定義
├── mcp-servers/     # 5 MCP servers (cerebras, deepseek, embedding, ks22, terminal)
├── dashboard/       # React + TypeScript + Tauri 2.x
├── scripts/         # ユーティリティスクリプト
├── qa/              # issue-registry.json（バグ検証の真実の源泉）
└── .dev-notes/      # 保守ノート（gitignore対象・補足資料）
```

**技術スタック**: Rust（ワークスペース 2 クレート）/ TypeScript + React / Python

---

## 2. コード規模メトリクス

### 主要ファイルのサイズ（行数）

| ファイル | 行数 | 状態 |
|---|---|---|
| `crates/core/src/handlers.rs` | 1668 | ⚠️ 監視中 |
| `crates/core/src/db.rs` | 1658 | ⚠️ 監視中 |
| `crates/shared/src/lib.rs` | 654 | 許容範囲 |
| `crates/core/src/capabilities.rs` | 464 | 良好 |
| `crates/core/src/managers/` | (ディレクトリ) | 分割済み |

### テスト規模

| 項目 | 数値 |
|---|---|
| テスト関数総数（`#[test]` + `#[tokio::test]`） | ~90 |
| `#[cfg(test)]` ブロックを持つファイル | 15 |
| 統合テストファイル（`crates/core/tests/`） | 16 |
| 推定カバレッジ | ~35% |

---

## 3. 問題・懸念事項

### 🔴 要即時対応

#### A. `qa/issue-registry.json` の bug-017 が誤検知

`bug-017`（"CI/CD: cargo audit security check missing"）は `"status": "open"` だが、
`ci.yml:63-64` に `cargo audit` が既に存在する。**レジストリが現実と不一致**。

---

### ✅ 解決済み（参考）

- **`managers.rs` の肥大化**: `managers/` ディレクトリに分割完了
  (mod.rs, agents.rs, plugin.rs, registry.rs, mcp.rs, mcp_protocol.rs, mcp_transport.rs)
- **`evolution.rs` の肥大化**: `archive/evolution/` にアーカイブ完了、ファイル削除済み

---

### 🟠 高優先度（1ヶ月以内）

#### B. テストカバレッジ不足（~35%）

以下の重要パスが未テスト:

| 未テストシナリオ | 優先度 |
|---|---|
| DBマイグレーションロールバック | HIGH |
| プラグイン初期化失敗のハンドリング | HIGH |
| 同時並行イベントストーム（100+イベント/秒） | HIGH |
| メモリ枯渇（大量イベント履歴） | MEDIUM |
| レートリミッタークリーンアップの競合状態 | MEDIUM |
| パーミッション承認ワークフロー | MEDIUM |

モックプラグインが未実装のため、テストが実プラグインに依存している。
`MockPlugin` 実装（`crates/core/tests/mocks/mod.rs`）が必要。

---

### 🟡 中優先度（3ヶ月以内）

#### C. DB全件取得のページネーション欠如（M-03）

`db::get_all_json()` が LIMIT なしで全レコードを取得する。
プラグインが大量データを保存した場合にメモリ枯渇・タイムアウトのリスクあり。

```rust
// 現状（危険）
"SELECT key, value FROM plugin_data WHERE plugin_id = ?"
// → 無制限に全件取得

// 修正案（Option B: デフォルト上限）
const DEFAULT_MAX_RESULTS: usize = 1000;
"SELECT key, value FROM plugin_data WHERE plugin_id = ? LIMIT ?"
```

#### D. グレースフルシャットダウン未実装

バックグラウンドタスクの JoinHandle が保存されていないため、
シャットダウン時にクリーンアップができない。

```rust
// 現状（問題あり）
tokio::spawn(async move { processor_clone.process_loop(...).await });
// JoinHandle を破棄している

// 改善: JoinHandle を保存し、シャットダウン時に await
```

#### E. エラーメッセージの不明瞭さ（M-05）

DB操作エラー・設定エラーにコンテキスト情報が不足しており、
デバッグ効率が低下している。エラーに「どのエージェントの」「どの操作で」を付加すること。

#### F. イベント履歴の上限未設定

高頻度イベント時（1000件/分）に 60 分保持で最大 60,000 件 ≈ 60MB が蓄積される可能性がある。

```rust
// 追加推奨
const MAX_EVENT_HISTORY: usize = 10_000;
history.retain(|e| e.timestamp > cutoff);
if history.len() > MAX_EVENT_HISTORY {
    history.drain(..history.len() - MAX_EVENT_HISTORY);
}
```

---

### 🟢 低優先度

| 項目 | 内容 | 工数 |
|---|---|---|
| `clone()` 最適化（L-01） | 160+ 呼び出し（大半は正当な Arc clone） | 1-2h |
| `clippy::pedantic` 有効化 | 追加の lint 指摘を得る | 1h |
| `rustfmt.toml` 設定 | プロジェクト固有のフォーマット設定 | 30分 |
| DB接続プール設定の環境変数化 | `SQLX_MAX_CONNECTIONS` を設定可能に | 10分 |
| レートリミッタークリーンアップ頻度 | 10分 → 2分（低影響） | 5分 |

---

## 4. コード品質サマリー

### `unwrap()` 使用（総計 245箇所）

実態を精査した結果、本番コードへの影響は限定的:

| カテゴリ | 件数 | 対処 |
|---|---|---|
| `#[cfg(test)]` ブロック内 | ~230 | 許容（テスト用途） |
| ベンチマーク（`benches/`） | ~10 | 許容 |
| `is_some()` チェック済みの安全な使用 | 2 | 許容 |
| 本番コードで要確認 | ~5 | 下記参照 |

**本番コード内の要確認箇所**:
- `dashboard/src-tauri/src/lib.rs:108` — Tauri アイコン取得（フレームワーク慣習として許容可）

### `#[allow(dead_code)]` （12箇所）

| 箇所 | 理由 | 判定 |
|---|---|---|
| `plugins/moderator/src/lib.rs` | 将来の UI 用フィールド | ✅ 正当 |
| `crates/core/src/handlers.rs:797` | 調査が必要 | ⚠️ 要確認 |

### TODO/FIXME/HACK コメント

**1件のみ**（ほぼゼロ）。優秀な状態。Issue Tracker による管理が徹底されている。

---

## 5. CI/CD 評価

| チェック | 状態 | 備考 |
|---|---|---|
| `cargo fmt --check` | ✅ | CI 必須 |
| `cargo clippy -D warnings` | ✅ | CI 必須 |
| `cargo test --workspace` | ✅ | CI 必須 |
| `cargo audit` | ✅ | CI 必須（bug-017 は誤検知、修正要） |
| Dashboard ビルド + テスト | ✅ | CI 必須 |
| リリース: チェックサム生成 | ✅ | SHA256 |
| リリース: cosign 署名 | ✅ | キーレス署名、優秀 |
| GitHub Actions ピン留め | ✅ | 全ステップがコミットハッシュで固定 |
| 並行 CI 制御 | ✅ | `concurrency` 設定済み |
| `cargo audit` ローカル | ⚠️ | 開発環境に未インストール |
| Windows インストーラー | ✅ | Inno Setup + Tauri |
| クロスコンパイル | ✅ | linux-x64/arm64, macOS-x64/arm64, win-x64 |

---

## 6. 総合評価

| 観点 | 評価 | コメント |
|---|---|---|
| CI/CD | **A** | 充実したパイプライン・セキュリティ重視 |
| セキュリティ | **B+** | 重大脆弱性は修正済み、継続監視が必要 |
| テスト | **C+** | 105 テスト関数だが推定 35% カバレッジ |
| コード品質 | **B** | `unwrap` 乱用なし・TODO ほぼゼロは優秀 |
| ファイル構造 | **B** | `managers.rs` 分割完了、`evolution.rs` アーカイブ済み |
| ドキュメント | **B+** | v0.5.3 で docs/ を監査・統合・最新化 |
| 依存関係管理 | **B+** | ワークスペース統一・ロックファイルあり |

---

## 7. アクションリスト

### 即時対応

- [ ] `qa/issue-registry.json` の bug-017 を `"status": "fixed"` に修正し `verify-issues.sh` を実行

### 1ヶ月以内

- [ ] `MockPlugin` 実装（`crates/core/tests/mocks/mod.rs`）
- [ ] `db::get_all_json()` に `LIMIT` 追加
- [ ] グレースフルシャットダウン実装（JoinHandle の保存）

### 3ヶ月以内

- [ ] テストカバレッジ 35% → 60%（DBマイグレーション・プラグイン初期化失敗・並行ストームを優先）
- [ ] エラーメッセージ改善（M-05）: DB/Config エラーにコンテキストを付加
- [ ] イベント履歴に上限設定（`MAX_EVENT_HISTORY = 10_000`）
- [ ] `clippy::pedantic` 有効化と修正

---

## 8. 既修正済みバグ（参考）

直近の開発サイクルで 17 件が修正済み（`qa/issue-registry.json` 参照）。
主要なもの:

| 重要度 | 内容 |
|---|---|
| CRITICAL | SafeHttpClient の `.expect()` によるパニックリスク |
| CRITICAL | TUI 端末の panic 時クリーンアップ欠如 |
| CRITICAL | Python サンドボックスの `socket` モジュール漏れ |
| HIGH | DeepSeek/Cerebras プラグインの空 API キーサイレント受け入れ |
| HIGH | CLI の SSE URL パスミスマッチ |
| HIGH | `logs.rs` でのマルチバイト文字 UTF-8 スライスパニック |
| HIGH | `config set` コマンドが API キーをディスクに書き込む問題 |
| HIGH | TUI スクロール位置の未クランプ・方向反転 |

---

*このドキュメントは保守性スキャンの結果を記録したものです。*
*次回スキャン推奨時期: **2026-06-04**（3ヶ月後）*
