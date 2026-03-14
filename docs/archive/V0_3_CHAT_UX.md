# v0.3.x Chat UX — ストリーミング・Artifactパネル・スケルトン

## Overview

v0.3.x ではエージェントチャットUIに3つの機能を導入し、
コード生成体験を大幅に改善する。

| 機能 | 概要 | バージョン |
|------|------|-----------|
| **A. タイプライター表示** | 受信済みテキストを文字送りで表示 | v0.3.0 |
| **B. Artifact パネル** | コードブロックを右側パネルに分離表示 | v0.3.0 |
| **D. スケルトン + アニメーション** | 生成待ちをシマーアニメーションで可視化 | v0.3.0 |

## 現状 (v0.2.x)

```
User sends message
  → POST /api/chat/{agentId}/messages (persist)
  → POST /api/chat (event bus → MCP mind server)
  → MCP returns full response as single JSON
  → ThoughtResponse event via SSE (complete text)
  → Frontend renders all at once
  → "THINKING..." placeholder during wait
```

**課題:**
- 応答全文が一括表示され、生成感がない
- コードブロックにシンタックスハイライトなし
- コピー・ダウンロード機能なし
- 待機中のフィードバックが "THINKING..." テキストのみ

---

## A. タイプライター表示

### 方針

フロントエンド側の疑似ストリーミング。MCP/バックエンドの変更は不要。

> 本物のトークン単位ストリーミング (MCP mind → Rust → SSE → フロント) は
> MCP protocol の変更を伴うため v0.4.x 以降に延期する。

### 動作フロー

```
ThoughtResponse received (full text)
  → テキストをバッファに格納
  → スケルトン非表示 (D → A へ遷移)
  → requestAnimationFrame ループで文字送り開始
    - 速度: ~5ms/文字 (約200文字/秒)
    - Markdown パース: 表示済み部分のみ逐次パース
    - ブリンクカーソル: 末尾に 2px 縦棒 (CSS @keyframes)
  → 全文表示完了
    → カーソル消去
    → コードブロック検出 → Artifact パネルへ分離 (B)
```

### 実装詳細

#### useTypewriter hook

```typescript
interface TypewriterState {
  displayedText: string;    // 現在表示中のテキスト
  isAnimating: boolean;     // アニメーション中か
  isComplete: boolean;      // 全文表示完了か
  skip: () => void;         // 即時全文表示
}

function useTypewriter(fullText: string, speed?: number): TypewriterState;
```

- `speed`: デフォルト 5ms/文字
- `skip()`: ユーザーがクリックまたは Enter で即時全文表示
- Markdown のコードフェンス (```) 内はブロック単位で一括表示
  (文字送りだと構文が崩れるため)

#### ブリンクカーソル (CSS)

```css
.typewriter-cursor {
  display: inline-block;
  width: 2px;
  height: 1em;
  background: currentColor;
  margin-left: 1px;
  animation: blink 1s step-end infinite;
}

@keyframes blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}
```

#### Markdown ストリーミングレンダリング

テキスト表示中も Markdown を正しくレンダリングする必要がある。

- **通常テキスト**: 文字送りしながら逐次レンダリング
- **コードブロック**: フェンスの開始 (```) を検出したらブロック全体をバッファし、
  閉じフェンス到達時に一括レンダリング
- **インラインコード**: バッククォート内を一括表示
- **リスト・テーブル**: 行単位で表示

### 変更ファイル

| ファイル | 変更 |
|----------|------|
| `dashboard/src/hooks/useTypewriter.ts` | 新規: タイプライター hook |
| `dashboard/src/components/AgentConsole.tsx` | ThoughtResponse 受信後の描画ロジック変更 |
| `dashboard/src/components/MessageContent.tsx` | 新規: Markdown パース + ストリーミング対応レンダラー |

---

## B. Artifact パネル

### 方針

15行以上のコードブロックを自動的に右側パネルに分離表示する。
Claude の Artifact システムを参考にしつつ、Cloto のデザイン言語に統合。

### トリガー条件

```
コードブロック検出
  → 行数 ≥ 15 → Artifact パネルに表示
  → 行数 < 15 → チャット内にインライン表示
```

### レイアウト

```
┌──────────────────────────────────────────────────────┐
│  ViewHeader                                    ● ─□× │
├────────────────────────┬─────────────────────────────┤
│                        │  ┌─ language ─── Copy  DL ┐ │
│   Chat Messages        │  │                        │ │
│                        │  │  syntax-highlighted    │ │
│   [user bubble]        │  │  code content          │ │
│   [agent bubble]       │  │                        │ │
│   [agent bubble...]    │  │                        │ │
│                        │  │                        │ │
│   ┌ inline code ─────┐ │  │                        │ │
│   │ (< 15 lines)     │ │  │                        │ │
│   └──────────────────┘ │  │                        │ │
│                        │  └────────────────────────┘ │
│                        │  ┌─ Tab 1 │ Tab 2 ────────┐ │
│                        │  │ (複数コード時タブ切替)   │ │
│                        │  └────────────────────────┘ │
├────────────────────────┴─────────────────────────────┤
│  [Input Area]                                  Send  │
└──────────────────────────────────────────────────────┘
```

- パネルが空の場合: チャットが全幅を占有 (現行と同じ)
- パネルにコードがある場合: `grid grid-cols-[1fr_1fr]` で50:50分割
- パネルはリサイズ可能 (ドラッグハンドル)
- モバイル: パネルはオーバーレイモーダル

### パネル機能

| 機能 | 説明 |
|------|------|
| **言語ラベル** | コードブロックの言語をヘッダーに表示 (例: `python`, `typescript`) |
| **コピーボタン** | クリップボードにコード全文をコピー。"Copied!" フィードバック (1.5秒) |
| **ダウンロードボタン** | 言語に応じた拡張子でファイルダウンロード (`code.py`, `code.ts` 等) |
| **行番号** | 左側にグレーの行番号表示 |
| **タブ切替** | 同一会話内の複数コードブロックをタブで切替 |
| **閉じるボタン** | パネルを閉じてチャット全幅に戻る |

### シンタックスハイライト

**highlight.js** を採用する。

選定理由:
- バンドルサイズが小さい (コア + 必要言語のみロード)
- 動的ストリーミングとの相性が良い (DOM 更新に強い)
- 189言語対応、十分なテーマ数
- Shiki は高品質だが WASM 依存でバンドルが大きく、ストリーミング中の再ハイライトが重い

```typescript
import hljs from 'highlight.js/lib/core';
import javascript from 'highlight.js/lib/languages/javascript';
import python from 'highlight.js/lib/languages/python';
import rust from 'highlight.js/lib/languages/rust';
// ... 必要な言語を個別インポート

hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('python', python);
hljs.registerLanguage('rust', rust);
```

デフォルトロード言語 (初期バンドル):
`javascript`, `typescript`, `python`, `rust`, `json`, `bash`, `html`, `css`, `toml`, `sql`

テーマ: `github-dark` (ダークモード向け、コントラスト良好)

### チャット内インラインコードブロック (< 15行)

15行未満のコードブロックもハイライト + コピーボタンを付与する。

```
┌─ python ─────────────────── [Copy] ┐
│  def hello():                      │
│      print("Hello, world!")        │
└────────────────────────────────────┘
```

### 変更ファイル

| ファイル | 変更 |
|----------|------|
| `dashboard/src/components/ArtifactPanel.tsx` | 新規: Artifact パネルコンポーネント |
| `dashboard/src/components/CodeBlock.tsx` | 新規: ハイライト付きコードブロック |
| `dashboard/src/components/AgentConsole.tsx` | レイアウト分割、Artifact 状態管理 |
| `dashboard/src/hooks/useArtifacts.ts` | 新規: コードブロック抽出・管理 hook |
| `dashboard/package.json` | `highlight.js` 依存追加 |

---

## D. スケルトン + アニメーション

### 方針

2段階モデルで待機状態を可視化する。

### 段階1: Processing (応答待ち)

ユーザーがメッセージ送信後、エージェントの応答が届くまでの間。
現在の "THINKING..." を**シマースケルトン**に置換。

```
┌────────────────────────────────────┐
│ ██████████████████████  ← shimmer  │
│ ████████████████                   │
│ ██████████████████████████         │
│ ████████████                       │
└────────────────────────────────────┘
```

- 3〜4行のグレーバー (幅ランダム: 60-90%)
- 左→右のシマーグラデーション (1.8秒ループ)
- エージェントのアイコン + 色を使用 (誰が考え中か明確)

```css
.skeleton-shimmer {
  background: linear-gradient(
    90deg,
    var(--skeleton-base) 25%,
    var(--skeleton-highlight) 50%,
    var(--skeleton-base) 75%
  );
  background-size: 200% 100%;
  animation: shimmer 1.8s ease-in-out infinite;
}

@keyframes shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
```

### 段階2: Generation (テキスト表示)

ThoughtResponse 受信後、スケルトンからタイプライターへ遷移。

```
Processing (スケルトン)
  → ThoughtResponse 受信
  → スケルトン fade-out (200ms)
  → タイプライター fade-in (200ms)
  → 文字送り開始 (A)
```

### メッセージ出現アニメーション

全メッセージ (ユーザー・エージェント) に共通の出現エフェクト:

```css
.message-enter {
  animation: messageIn 300ms ease-out;
}

@keyframes messageIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
```

### コードブロック展開アニメーション

コードブロックの出現時に高さ拡張アニメーション:

```css
.code-block-enter {
  animation: expandIn 400ms ease-out;
  overflow: hidden;
}

@keyframes expandIn {
  from {
    max-height: 0;
    opacity: 0;
  }
  to {
    max-height: 1000px;
    opacity: 1;
  }
}
```

### 変更ファイル

| ファイル | 変更 |
|----------|------|
| `dashboard/src/components/SkeletonMessage.tsx` | 新規: シマースケルトンコンポーネント |
| `dashboard/src/components/AgentConsole.tsx` | "THINKING..." → SkeletonMessage 置換 |
| `dashboard/src/index.css` | アニメーション CSS 追加 |

---

## 実装順序

```
v0.3.0
  ├─ Phase 1: 基盤
  │   ├─ highlight.js セットアップ + CodeBlock コンポーネント
  │   ├─ MessageContent (Markdown レンダラー) 新規作成
  │   └─ インラインコードブロック改善 (ハイライト + コピー + 言語ラベル)
  │
  ├─ Phase 2: スケルトン (D)
  │   ├─ SkeletonMessage コンポーネント
  │   ├─ AgentConsole の isTyping → スケルトン置換
  │   └─ メッセージ出現アニメーション
  │
  ├─ Phase 3: タイプライター (A)
  │   ├─ useTypewriter hook
  │   ├─ AgentConsole 統合 (スケルトン → タイプライター遷移)
  │   └─ ブリンクカーソル + skip 機能
  │
  └─ Phase 4: Artifact パネル (B)
      ├─ ArtifactPanel コンポーネント
      ├─ useArtifacts hook (15行以上の抽出)
      ├─ AgentConsole レイアウト分割
      ├─ タブ切替・コピー・ダウンロード
      └─ リサイズハンドル
```

## 新規依存パッケージ

| パッケージ | 用途 | サイズ目安 |
|-----------|------|-----------|
| `highlight.js` | シンタックスハイライト | ~30KB (コア + 10言語) |
| `marked` (検討) | Markdown パース | ~40KB |

> `marked` は既存の Markdown レンダリングが無いため新規導入が必要。
> 軽量で拡張性が高く、ストリーミング対応のトークナイザを持つ。
> 代替: `markdown-it` (~60KB) も候補だが `marked` の方が軽量。

## 将来拡張 (v0.4.x 以降)

| 機能 | 概要 |
|------|------|
| **本物のトークンストリーミング** | MCP mind server → Rust → SSE のチャンク配信 |
| **ライブプレビュー** | HTML/React Artifact のサンドボックス実行 |
| **Diff 表示** | コード修正時の赤/緑差分ハイライト |
| **バージョン履歴** | Artifact ごとの版管理 |
| **インライン編集** | Artifact パネル内でコードを直接編集 |
