# ClotoCore Development Guide

開発者が遵守すべきガードレール（制約ルール）と、現在進行中のリファクタリングの状況を統合したドキュメントです。

---

## 1. Refactoring Guardrails (やってはいけないこと)

コードを変更する前に必ずこのリストを確認し、制約を遵守してください。

### 1.1 Security Hardening: Event Envelopes

**目標**: `ClotoEvent` を Kernel が管理する封筒（Envelope）で包み、送信元（Issuer）の改竄を防ぐ。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| `EventEnvelope` 構造体を作成する | `ClotoEvent` 自体に `issuer_id` を追加してはいけない | プラグインが ID を偽装できてしまうため |
| `EventProcessor` で `issuer` を検証する | `if plugin_id == "admin"` のようなハードコード特権判定を行ってはいけない | 原則 #2 (Capability over Concrete Type) に反する |
| プラグインの `on_event` 引数を変更する | プラグイン側で `issuer` を書き換え可能にしてはいけない | 封印後のデータ一貫性を損なうため |
| SSE 出力を調整する | 既存の JSON フォーマットを破壊してはいけない | Dashboard が壊れる「無限ループ」の典型例 |
| `dispatch_event` のシグネチャを変更する | プラグインの `on_event` 内で `dispatch` を直接呼ばせてはいけない | Kernel を経由しないイベント発行は「なりすまし」の温床 |

### 1.2 Cascading Protection: Event Depth Tracking

**目標**: イベントの無限ループや過剰連鎖によるリソース枯渇を防ぐ。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| `EnvelopedEvent` に `depth: u8` を追加する | `ClotoEvent` に `depth` を追加してはいけない | プラグイン側で深さを偽装できるため |
| `dispatch_event` で上限チェックする | 上限値をハードコードしてはいけない | `AppConfig.max_event_depth` で設定可能にするため |
| 再配信時に `parent.depth + 1` を設定する | 全イベントの `depth` を 0 で固定してはいけない | 連鎖を検知できなくなるため |
| 破棄時にエラーログを出力する | サイレントにイベントを捨ててはいけない | デバッグが不可能になるため |

### 1.3 State Management: Lock Aggregation

**目標**: プラグイン内部の状態管理を単純化し、設定更新時のアトミック性を保証する。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| 関連する設定を一つの `struct` にまとめる | 設定値ごとに個別の `RwLock` を使用してはいけない | 更新時の不整合状態を防ぐため |
| `Arc<RwLock<ConfigStruct>>` を使用する | `Arc<RwLock<Option<Arc<...>>>>` のような深いネストを作ってはいけない | 可読性低下とデッドロックリスク |
| `on_event` での設定更新をアトミックに行う | 一連の更新途中で `await` や他のロック取得を挟んではいけない | デッドロックと原子性の喪失 |

### 1.4 Storage & Memory: Chronological Consistency

**目標**: 記憶の想起（Recall）で常に最新の文脈が正確な順序で取得されることを保証する。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| ソータブルなタイムスタンプをキーに含める | キーの先頭から AgentID を外してはいけない | 範囲検索ができなくなり、他エージェントの記憶が混ざるため |
| タイムスタンプを固定長文字列にする | 生の時間数値をそのまま文字列にしてはいけない | 辞書順ソートが崩れるため（例: "100" < "9"）。ゼロパディング必須 |
| `recall` でメッセージを反転させる | Kernel 返却時に古い順のままにしてはいけない | LLM は「下に行くほど新しい」文脈を期待するため |

### 1.5 UI/UX: Clarity of Agency

**目標**: ユーザーが「Agent（対話相手）」と「Tool（機能）」を混同しないUI/UXを維持する。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| プラグインのカテゴリ分類を行う | `Tool` カテゴリのプラグインをエージェントリストに表示してはいけない | 認知的負荷の増大を防ぐため |
| エージェント定義をDBに保存する | 機能提供のみのプラグインを `agents` テーブルに登録してはいけない | エージェントは「人格」に限定すべき |

### 1.6 Physical Safety: HAL Rate Limiting

**目標**: HAL の物理操作でのAI暴走を防ぐ。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| マウス/キーボード操作を実装する | レートリミットなしに `InputControl` を実行してはいけない | OS全体が操作不能になる「物理的DoS」を防ぐため |
| 危険な操作を許可する | ユーザーの明示的承認なしに不可逆な操作を行ってはいけない | ハルシネーションによるデータ消失防止 |

### 1.7 External Process: MCP Resource Control

**目標**: MCP 経由の外部プロセス起動時のリソース枯渇やゾンビプロセスを防ぐ。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| 外部プロセスを起動する | PID管理と終了処理なしに起動してはいけない | ゾンビプロセスがメモリやポートを占有し続けるため |
| MCPツールを実行する | タイムアウト設定なしに外部ツールを呼んではいけない | ハングアップがKernel全体を停止させるため |

### 1.8 Privacy & Biometrics: Camera Usage

**目標**: Webカメラ利用時のプライバシー保護。

| ステップ | DO NOT | 理由 |
| :--- | :--- | :--- |
| カメラを起動する | ユーザーの同意なしにバックグラウンドで起動してはいけない | 盗撮・プライバシー侵害防止 |
| 顔画像を処理する | 顔の生映像をストレージに保存・外部送信してはいけない | 生体情報漏洩防止。座標データのみ配信 |
| 視線データを共有する | 許可ドメイン以外に視線データをストリーミングしてはいけない | 「何を見ているか」自体が機密情報 |

---

## 2. Current Refactoring Status

### Phase 5: Post-Audit Security & Performance Hardening (2026-02-13)

**Trigger:** CODE_QUALITY_AUDIT.md (Score: 65/100)

| Category | Item | Status |
|----------|------|--------|
| Security | ダミーAPIキー削除、環境変数ベースに移行 (`db.rs`) | Done |
| Security | 認証バイパス修正、release buildで`CLOTO_API_KEY`必須化 (`handlers.rs`) | Done |
| Security | ~~Python Bridge メソッドホワイトリスト導入~~ (deleted with python_bridge) | Done |
| Security | ~~パストラバーサル対策~~ (deleted with python_bridge) | Done |
| Security | 未使用DISCORD_TOKEN削除 (`.env`) | Done |
| Performance | イベント履歴 `Vec` → `VecDeque` (O(1) pop_front) | Done |
| Performance | ホワイトリスト `Vec` → `HashSet` (O(1) lookup) | Done |
| Performance | Python Bridge バックグラウンドリーダー JoinHandle 追跡 | Done |
| Quality | managers.rs イベントディスパッチのネスト削減 | Done |
| Quality | StatusCore.tsx React import 統合 | Done |
| Verification | 全11テストパス、警告ゼロ | Done |

**Audit Score Impact:**
- Security (C): 55 → ~75
- Performance (D): 60 → ~80

### Phase 6: Feature Expansion & Hardening (2026-02-14)

**Trigger:** Post-Phase 5 stabilization

| Category | Item | Status |
|----------|------|--------|
| Security | Human-in-the-Loop 権限承認ワークフロー (`permission_requests` テーブル) | Done |
| Security | Rate Limiting: per-IP 10 req/s, burst 20 (`middleware.rs`) | Done |
| Security | Audit Logging: セキュリティイベント全記録 | Done |
| Security | .env ファイルパーミッション 0600 (Unix) | Done |
| Security | BIND_ADDRESS デフォルト 127.0.0.1 (loopback only) | Done |
| Security | CORS origin スキーム検証 (http/https のみ許可) | Done |
| Security | cosign keyless署名 (リリースアーティファクト) | Done |
| Quality | Unit Tests: handlers, db, capabilities, middleware, validation, config | Done |
| Quality | Input validation モジュール (エージェント作成・設定更新) | Done |
| Quality | Atomic file writes (.maintenance ファイル) | Done |
| Feature | Self-Healing Python Bridge (自動再起動、最大3回) | Done |
| Feature | Build Optimization (`CLOTO_SKIP_ICON_EMBED=1`) | Done |
| Feature | 全コメント英語化 (国際アクセシビリティ) | Done |
| Feature | Windows GUI インストーラー (Inno Setup) | Done |
| Feature | GitHub Pages ランディングページ (OS自動検出) | Done |
| Infra | GitHub Actions リリースワークフロー (5プラットフォーム + インストーラー) | Done |

**Test Count:** 133 tests
**Audit Score:** 90+/100

### Remaining Items (Next Phase)

- [ ] Event Envelope: Kernel 管理の封筒によるイベント改竄防止
- [ ] MCP server hot-reload: ランタイムMCPサーバー再接続

---

## 3. Versioning

ClotoCore uses a phase-based versioning scheme with three stages.

### Phases

| Phase | Display | Cargo (Semver) | Git Tag | Status |
|-------|---------|---------------|---------|--------|
| Alpha | A1, A2, ... | `0.0.1`, `0.0.2`, ... | `vA1` | Completed (A1–A7) |
| Beta | βX.Y | `0.X.Y` | `v0.X.Y` | **Current (β1 = 0.1.0)** |
| Stable | 1.X.Y | `1.X.Y` | `v1.X.Y` | Future |

- **Alpha (A)**: Rapid prototyping. Breaking changes expected on every release.
- **Beta (βX.Y)**: Feature complete, stabilization phase. Follows the same X.Y convention as Stable under the `0.` prefix. `X` = major update, `Y` = minor update / patch. Example: β1 → β1.1 → β1.2 → β2 → β2.1.
- **Stable (1.X.Y)**: Production ready. The leading `1` is fixed unless a major architectural overhaul occurs. `X` = major update, `Y` = minor update / patch.

### System vs Plugin Versions

| Component | Versioning | Source of Truth |
|-----------|-----------|----------------|
| System (kernel, SDK, macros) | Unified workspace version | `Cargo.toml` → `workspace.package.version` |
| Plugins | Independent per plugin | `#[cloto_plugin(version = "...")]` in each plugin's `lib.rs` |
| Dashboard | Matches system version | `dashboard/package.json` |

Plugins maintain their own version numbers because they can evolve independently of the kernel. When creating a new plugin, start at `0.1.0`.

### Release Process

1. Bump the version in `Cargo.toml` (workspace) and `dashboard/package.json`
2. Commit: `chore: bump version to β1.1 (0.1.1)` or `chore: bump version to β2 (0.2.0)`
3. Tag: `git tag v0.1.1` or `git tag v0.2.0`
4. Push: `git push origin master --tags`
5. The GitHub Actions release workflow builds and publishes automatically

---

*Document History:*
- 2026-02-08: Guardrails 初版作成 (Event Security, Cascading Protection, Lock Aggregation, Storage Consistency)
- 2026-02-10: UI/UX Clarity, Physical Safety, MCP Resource Control, Privacy & Biometrics 追加
- 2026-02-13: REFAC_STATUS.md と統合、Phase 5 完了ステータス追加
- 2026-02-15: Phase 6 完了ステータス追加、残タスク更新
