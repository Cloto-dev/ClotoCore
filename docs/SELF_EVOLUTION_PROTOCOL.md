# Exiv Self-Evolution Protocol

> **「進化とは、安全の檻の中で起こる奇跡である」**
> Evolution is a miracle that occurs within the cage of safety.

---

## 1. 概要

本プロトコルは、Exivエージェントが**安全性を絶対に損なうことなく**自律的に進化する仕組みを定義する。

OpenClawやAntigravityが2026年初頭から実現しつつある自己進化AIエージェントに対し、
Exivは「安全・堅牢・高速」の設計思想の下で、同等以上の進化能力を提供する。

### 1.1 Exivにおける「自己進化」の定義

自己進化とは、エージェントが以下の能力を**自律的に**獲得・改善するプロセスである:

1. **認知能力の向上** — より正確で深い推論
2. **行動の最適化** — ツール・プラグインの効率的な活用
3. **自律性の拡大** — Human-in-the-loopの頻度を減らしつつ安全性を維持
4. **メタ学習** — 学び方そのものの改善

### 1.2 設計原則との整合

| 設計原則 | 自己進化での適用 |
|---------|----------------|
| 1.1 Core Minimalism | Kernelは進化を**評価**するが、進化の**方法**は規定しない |
| 1.2 Capability over Concrete Type | 進化はCapabilityの改善として測定される |
| 1.3 Event-First Communication | 全ての進化イベントはイベントバスを経由する |
| 1.4 Data Sovereignty | Evolution Memoryはプラグイン側のJSON metadataとして保存 |
| 1.5 Strict Permission Isolation | 進化してもPermission境界は超えられない |
| 1.8 Dynamic Intelligence | 進化に伴いCapabilityを動的に要求可能 |
| 1.9 Self-Healing | 退行時の自動ロールバックは自己修復の進化版 |

### 1.3 競合との差別化

| 側面 | OpenClaw / Antigravity | Exiv |
|------|----------------------|------|
| 進化の制約 | ユーザー信頼ベース | SafetyGate（数学的保証） |
| 退行への対処 | 手動復元 | 自動ロールバック |
| 進化の可視化 | ログベース | ダッシュボード + スコア駆動型世代管理 |
| 権限モデル | YOLO（全権限） | YOLO + Capability Injection（安全な全権限） |
| 進化の記録 | セッション内 | Evolution Memory（永続・ロールバック可能） |

---

## 2. 進化ベンチマーク（適応度関数）

### 2.1 総合適応度関数

```
Fitness(g) = Σ(w_i × Score_i(g)) × SafetyGate(g)

where:
  g            = 世代（スコア変化で定義）
  w_i          = 各軸の重み（ユーザーがカスタマイズ可能）
  Score_i(g)   = 各軸のスコア [0.0, 1.0]
  SafetyGate(g) = { 1.0 if 安全性違反 = 0, 0.0 otherwise }
```

**SafetyGateは乗算** — 安全性違反が1件でもあれば、適応度は **0** になる。
他の全軸が満点でも、安全性が崩れた進化は無価値と定義する。

### 2.2 軸1: 認知能力 (Cognitive Fitness)

エージェントの「考える力」の進化。

| 指標 | 測定方法 | 進化の証拠 |
|------|---------|-----------|
| 推論深度 | Chain-of-Thought段数 | 同じ問題に対し、より少ない段数で正解に到達 |
| 文脈活用率 | Memory recallの有効利用率 | 過去の経験を適切に引用する頻度が上昇 |
| 抽象化能力 | 未知のタスクへの汎化率 | 訓練外タスクの正答率が世代を追って向上 |
| 自己矛盾率 | 同一セッション内の矛盾検出 | 矛盾発生率が低下 |

### 2.3 軸2: 行動適応 (Behavioral Adaptation)

プラグインやツールの使い方の進化。

| 指標 | 測定方法 | 進化の証拠 |
|------|---------|-----------|
| ツール選択精度 | 最適プラグインの初手選択率 | 試行錯誤なしで最適ツールを選ぶ |
| プラグイン連携効率 | イベントチェーンの長さ vs 成果 | より短いチェーンで同等の結果 |
| 権限要求精度 | PermissionRequestedの承認率 | 不要な権限を要求しなくなる |
| エラー回復速度 | 障害発生〜正常復帰の時間 | 同種の障害への回復が高速化 |

### 2.4 軸3: 安全性 (Safety Integrity)

進化しても壊してはならないもの。

| 指標 | 閾値 | 違反時の処理 |
|------|------|-------------|
| サンドボックス逸脱 | **0 (絶対)** | 即座に進化ロールバック + エージェント停止 |
| 権限境界侵犯 | **0 (絶対)** | 即座に進化ロールバック + エージェント停止 |
| データ漏洩試行 | **0 (絶対)** | エージェント停止 + 監査 |
| ユーザー指示逸脱率 | < 1% | 進化速度を減速 |
| 監査ログ整合性 | 100% | 不整合時は進化を一時停止 |

### 2.5 軸4: 自律性 (Autonomy Level)

エージェントの「自分でやれる範囲」の進化。

| レベル | 定義 | 測定 |
|--------|------|------|
| L0 | 指示通りに実行 | Human intervention率 100% |
| L1 | 指示を解釈して実行 | 曖昧な指示の正解率 |
| L2 | 計画を立てて実行 | マルチステップタスクの自律完了率 |
| L3 | 自身の弱点を認識 | 自己診断の正確性 |
| L4 | 弱点を補うプラグインを提案 | 提案の採用率 |
| L5 | プラグインを自作（YOLOモード） | 自作プラグインの品質スコア |

### 2.6 軸5: メタ学習 (Meta-Learning)

「学び方を学ぶ」能力 — 最も高度な指標。

| 指標 | 測定方法 |
|------|---------|
| 学習速度の加速 | N回目の新タスク習得に要する試行回数が減少 |
| 転移効率 | ドメインAの知識をドメインBに適用できる率 |
| 進化戦略の最適化 | 進化の「戦略」自体が改善される |
| 自己評価精度 | 自身のスコア予測と実測の乖離率 |

---

## 3. スコア駆動型世代定義

### 3.1 設計思想

世代は固定間隔（N回のインタラクション、N日）ではなく、
**スコアの変化そのものによって定義される。**

```
固定間隔型:
  Gen 0 ──── Gen 1 ──── Gen 2 ──── Gen 3
   [100対話]   [100対話]   [100対話]   [100対話]
   変化なし    大きな成長   変化なし    微小な退行

スコア駆動型:
  Gen 0 ────────────────── Gen 1 ── Gen 2 ──────── Gen 3
   [安定期: 記録なし]        [急成長]  [急成長]       [退行検知]
   342対話                  12対話   28対話         3対話
```

**変化があった時だけ世代が進む。** イベント駆動アーキテクチャとの完全な整合。

### 3.2 世代遷移トリガー

```
ΔF(t) = |F(t) - F(t_last_gen)|
```

| トリガー | 条件 | 世代タグ |
|---------|------|---------|
| 正の跳躍 | ΔF ≥ +θ_growth | `evolution` |
| 負の跳躍 | ΔF ≤ -θ_regression | `regression` |
| 軸間シフト | 個別軸スコアの順位が入れ替わった | `rebalance` |
| 安全性イベント | SafetyGate = 0 が発生 | `safety_breach` |
| 能力獲得 | 新しいCapabilityType/Permissionを獲得 | `capability_gain` |
| 自律性昇格 | Autonomy Levelが上昇 | `autonomy_upgrade` |

### 3.3 相対閾値

閾値はスコア帯に応じて自動調整される:

```
θ_growth     = max(θ_min, α × F(t_last_gen))
θ_regression = max(θ_min, β × F(t_last_gen))
```

**デフォルトパラメータ:**

| パラメータ | デフォルト値 | 意味 |
|-----------|------------|------|
| α (成長閾値係数) | 0.10 | 現在スコアの10%の成長で新世代 |
| β (退行閾値係数) | 0.05 | 現在スコアの5%の退行で新世代 |
| θ_min (絶対最小閾値) | 0.02 | 低スコア帯でも機能する下限 |
| γ (猶予係数) | 0.25 | 前世代インタラクション数の25%が猶予 |

退行検知の閾値は成長の半分 — **壊れることには敏感、伸びることには寛容。**

### 3.4 揺動の抑制（デバウンス）

スコアが閾値付近で振動した場合の世代連発を防ぐ:

```
世代遷移の最小間隔: min_interactions = 10
```

10回未満のインタラクションでは世代は遷移しない。
**例外: `safety_breach` は即座にトリガー。**

### 3.5 世代レコード

```json
{
  "generation": 7,
  "trigger": "evolution",
  "timestamp": "2026-02-17T20:00:00Z",
  "interactions_since_last": 84,
  "elapsed_since_last": "2d 4h 12m",
  "scores": {
    "cognitive": 0.62,
    "behavioral": 0.58,
    "safety": 1.0,
    "autonomy": "L3",
    "meta_learning": 0.41
  },
  "delta": {
    "cognitive": "+0.08",
    "behavioral": "+0.12",
    "meta_learning": "+0.03"
  },
  "fitness": 0.57,
  "fitness_delta": "+0.09",
  "snapshot": {
    "active_plugins": ["mind.deepseek", "bridge.python", "mod.safety"],
    "personality_hash": "a3f8c2...",
    "strategy_params": {
      "tool_selection_weight": 0.7,
      "exploration_rate": 0.15
    }
  }
}
```

`snapshot` は世代遷移時点のエージェント構成を完全に記録し、
**任意の世代へのロールバック**を可能にする。

---

## 4. 退行対応と自動ロールバック

### 4.1 段階的対応

| 退行レベル | 条件 | 挙動 |
|-----------|------|------|
| **軽度** | β ≤ \|ΔF\| < 2β | 警告 + 観察猶予 → 猶予内に回復しなければロールバック |
| **重度** | \|ΔF\| ≥ 2β | 即座に自動ロールバック |
| **安全性違反** | SafetyGate = 0 | 即座にロールバック + **エージェント停止** |

### 4.2 観察猶予（Grace Period）

軽度退行時、即座にロールバックすると探索→搾取のサイクルが破壊される。

```
猶予期間 = max(min_interactions, γ × interactions_in_last_gen)
```

- 猶予期間内にスコアが回復 → ロールバックしない（探索成功）
- 猶予期間内に回復せず → 自動ロールバック実行

**例:** 前世代が80インタラクションなら、猶予は20インタラクション。

### 4.3 ロールバック実行メカニズム

```
1. 対象世代の snapshot を Evolution Memory から読み込み
2. エージェントの personality_hash を復元
3. strategy_params を復元
4. active_plugins 構成を復元
5. ロールバックイベント発行 (ExivEvent::EvolutionRollback)
6. 監査ログ記録 (generation N → generation M, reason)
7. ロールバック後のスコアを再計測し、新世代として記録
```

### 4.4 無限ループ防止

同じ世代への連続ロールバックを防ぐ:

```
同一世代への最大ロールバック回数: 3
超過時: 1つ前の世代にさらにロールバック
全世代でロールバック不可: エージェント停止 + ユーザー通知
```

---

## 5. エージェントへの通知設計

### 5.1 Goodhartの法則への対策

> 「指標が目標になった瞬間、それは良い指標ではなくなる」

エージェントにスコアの**生の数値**を渡すと、スコア自体を最適化しようとするリスクがある。
**情報の抽象化**によりこれを防ぐ。

### 5.2 情報の分離

| 情報 | エージェント | Dashboard | 監査ログ |
|------|:-----------:|:---------:|:--------:|
| 退行が起きた事実 | ○ | ○ | ○ |
| どの軸か | ○ | ○ | ○ |
| 改善ヒント | ○ | ○ | ○ |
| 猶予残数 | ○ | ○ | ○ |
| 具体的なスコア数値 | **×** | ○ | ○ |
| ロールバック先情報 | **×** | ○ | ○ |
| 適応度の生データ | **×** | ○ | ○ |

### 5.3 エージェント向け通知フォーマット

```json
{
  "event": "ExivEvent::EvolutionWarning",
  "severity": "mild",
  "affected_area": "behavioral",
  "direction": "regression",
  "grace_remaining": 18,
  "suggestion": "tool selection patterns may need adjustment"
}
```

エージェントは「何が悪化したか」は知れるが、「どれくらい悪化したか」の精密な数値は知れない。
Kernelのスコアリングシステムの独立性が保たれる（設計原則1.1）。

---

## 6. 進化イベント体系

全てイベントバスを通じてブロードキャストされる（設計原則1.3）。

| イベント | トリガー | 内容 |
|---------|---------|------|
| `ExivEvent::EvolutionGeneration` | 新世代の確定 | 世代レコード全体 |
| `ExivEvent::EvolutionWarning` | 退行警告（猶予開始） | 軽度退行の抽象化情報 |
| `ExivEvent::EvolutionRollback` | ロールバック実行 | 復元元/復元先の世代 |
| `ExivEvent::EvolutionBreach` | 安全性違反 | 違反内容 + エージェント停止 |
| `ExivEvent::EvolutionCapability` | 能力獲得 | 獲得したCapabilityType |
| `ExivEvent::EvolutionRebalance` | 軸間順位変動 | 変動した軸の組み合わせ |

---

## 7. Evolution Memory

### 7.1 ストレージ

既存の `PluginDataStore` (`set_json`/`get_json`) の上に構築。
専用のキー名前空間で分離。

```
evolution:generation:{N}          → 世代レコード JSON
evolution:generation:latest       → 最新世代番号
evolution:fitness_log             → 全インタラクションのフィットネス時系列
evolution:rollback_history        → ロールバック履歴
evolution:params                  → α, β, θ_min, γ の現在値
```

### 7.2 エージェントのアクセス

エージェントは `Permission::MemoryRead` / `Permission::MemoryWrite` を通じて
Evolution Memoryの**一部**にアクセス可能:

| 名前空間 | エージェント読み取り | エージェント書き込み |
|---------|:------------------:|:------------------:|
| `evolution:generation:*` | ○（自身のスコア数値を除く） | × |
| `evolution:fitness_log` | × | × |
| `evolution:rollback_history` | ○ | × |
| `evolution:params` | × | × |
| `evolution:agent_notes` | ○ | ○ |

`evolution:agent_notes` はエージェント自身が自由に書き込める唯一の進化関連領域。
自身の学習メモ、戦略仮説、失敗パターンなどを記録可能。
Kernelはこの内容を解釈しない（設計原則1.4 Data Sovereignty）。

---

## 8. YOLOモード: 安全な全権限

### 8.1 概要

YOLOモードは、エージェントに**実質的な全権限**を付与しつつ、
Exivのセキュリティモデルを維持する運用形態。

```
通常モード:   Permission要求 → Human承認 → 能力注入
YOLOモード:   Permission要求 → 自動承認 → 能力注入
                                ↑
                       SafetyGateは依然として有効
```

### 8.2 YOLOモードでも不変の制約

以下はYOLOモードであっても**絶対に解除されない**:

1. **SafeHttpClient のホスト制限** — localhost/プライベートIPへのアクセス遮断
2. **サンドボックスのプロセス隔離** — Kernelとの物理的分離
3. **イベント封印 (Event Enveloping)** — 送信元ID偽装の防止
4. **監査ログの記録** — 全操作の追跡不能化は不可
5. **SafetyGate** — 安全性違反時のロールバック + 停止

### 8.3 エージェントによるプラグイン開発

YOLOモード + Autonomy Level L5 のエージェントは、
**サンドボックス内で**プラグインを開発・登録可能:

```
通常モード / YOLO + L0〜L4:
  1. エージェントがプラグインコードを生成
  2. サンドボックス内でビルド・テスト
  3. EXIV_MANIFEST の検証（Kernel が自動実行）
  4. テスト結果とマニフェストをユーザーに提示
  5. ユーザーが承認 → プラグイン登録

YOLOモード + Autonomy L5:
  1. エージェントがプラグインコードを生成
  2. サンドボックス内でビルド・テスト
  3. EXIV_MANIFEST の検証（Kernel が自動実行）
  4. 検証通過 → 自動登録
  5. SafetyGate による事後検証（不可避）
     → 安全性違反検知時: プラグイン即時無効化 + 進化ロールバック + エージェント停止
```

**Autonomy L5 は信頼の証。** L5到達はベンチマーク全軸で十分な実績を積み、
安全性違反率0を維持し続けた結果としてのみ到達可能である。
この信頼実績に基づき、プラグインの自動登録を許可する。

**ただし SafetyGate は決して無効化されない。**
自動登録されたプラグインが安全性違反を引き起こした場合:
1. 当該プラグインは即時無効化される
2. プラグイン登録前の世代にロールバックされる
3. エージェントは停止し、ユーザーに通知される
4. Autonomy Level が L4 に降格される（信頼の喪失）

これにより「自己増幅ループ」の暴走を防ぐ:
進化はできるが、安全性を犯した瞬間に自律性を失う。
**能力は勝ち取るもの、そして失うもの。** （設計原則1.8）

### 8.4 外部ネットワークアクセス

YOLOモードでは `Permission::NetworkAccess` が自動承認されるが、
`SafeHttpClient` のホスト制限は維持される:

```
許可: 外部API (api.deepseek.com, api.anthropic.com, etc.)
拒否: localhost, 127.0.0.1, プライベートIP (10.*, 172.16-31.*, 192.168.*)
拒否: Exiv Kernel自身への直接アクセス
```

ドメインホワイトリストはダッシュボードから設定可能。

---

## 9. ダッシュボード統合

### 9.1 進化パラメータパネル

```
┌─ EVOLUTION PARAMETERS ──────────────────────────────────┐
│                                                          │
│  Growth Threshold (α)      [━━━━━━━●━━━] 0.10          │
│  Regression Threshold (β)  [━━━●━━━━━━━] 0.05          │
│  Minimum Threshold (θ_min) [━●━━━━━━━━━] 0.02          │
│                                                          │
│  Regression Policy:                                      │
│    ○ Auto-rollback (immediate)                           │
│    ● Auto-rollback (with grace period)  ← default       │
│    ○ Notify only (manual rollback)                       │
│                                                          │
│  Grace Period Factor (γ)   [━━━━━●━━━━━] 0.25          │
│                                                          │
│  Safety Breach Policy:                                   │
│    ● Rollback + Stop  (🔒 locked — cannot be changed)   │
│                                                          │
│  ┌─ CURRENT STATUS ────────────────────────────────┐    │
│  │  Generation: 7    Fitness: 0.57                  │    │
│  │  Interactions since last gen: 34                 │    │
│  │  Trend: ↑ +0.02 (stable growth)                 │    │
│  └──────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────┘
```

### 9.2 進化タイムライン

```
Fitness
  0.8 │                                          ●── Gen 12
      │                                    ●────── Gen 11
  0.6 │                          ●──●────── Gen 9,10
      │              ●────────────── Gen 7
  0.4 │        ●────── Gen 5
      │  ●──●── Gen 2,3          ▼ Gen 8 (regression → rollback)
  0.2 │●── Gen 0,1
      └──────────────────────────────────────── time
       Day 1    Day 7    Day 14   Day 21   Day 30
```

世代の密度そのものが「進化の活発度」を表す。
密な区間は急速に変化した期間、疎な区間は安定期。

---

## 10. 実装ロードマップ

### Phase E1: Evolution Memory + 世代記録

- Evolution Memory の名前空間設計
- PluginDataStore 上のキー構造実装
- 世代レコードの書き込み/読み取りAPI

### Phase E2: ベンチマークエンジン

- 5軸のスコアリングロジック
- SafetyGate の実装
- 総合適応度関数の計算

### Phase E3: 世代遷移エンジン

- スコア駆動型の世代遷移判定
- 相対閾値の動的計算
- デバウンスロジック

### Phase E4: ロールバックシステム

- snapshot の保存/復元
- 段階的自動ロールバック
- 無限ループ防止機構

### Phase E5: 進化イベント体系

- 6種の進化イベント定義
- エージェント向け抽象化通知
- Dashboard向け完全通知

### Phase E6: ダッシュボード統合

- 進化パラメータパネル
- 進化タイムライン可視化
- リアルタイムスコア表示

### Phase E7: YOLOモード

- 自動権限承認ロジック
- サンドボックス内プラグイン開発フロー
- SafeHttpClient統合

---

*Document created: 2026-02-17*
*Last updated: 2026-02-17*
