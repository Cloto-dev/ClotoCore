# ClotoCore インストーラー & 配布戦略

**Version:** 1.0.0
**Status:** Approved Design
**Date:** 2026-03-04

---

## 1. 概要

ClotoCoreをカジュアルユーザーが手軽に導入できるようにするための、
インストーラー構築・配布・自動更新の包括的な設計ドキュメント。

### 1.1 目標

- **ゼロ前提知識**: Rust/Node.js/Python の開発環境不要
- **ダウンロード → ダブルクリック → 起動**: 3ステップで完了
- **インストールレベル選択**: 最小 / 通常 / カスタム
- **自動更新**: 起動時に新バージョンを検知・適用

### 1.2 対象プラットフォーム

| Platform | Format | 自動更新 | 優先度 |
|----------|--------|---------|--------|
| Windows x64 | NSIS (.exe) via Tauri — Desktop Installer | Ed25519署名 | Phase 1 |
| macOS x64 | CLI binary (.tar.gz) | — | Phase 1 |
| macOS arm64 | CLI binary (.tar.gz) | — | Phase 1 |
| Linux x64 | CLI binary (.tar.gz) | — | Phase 1 |
| Linux arm64 | CLI binary (.tar.gz) | — | Phase 1 |

---

## 2. アーキテクチャ

### 2.1 配布チャネル

```
開発者 (タグ push: v0.5.3)
  │
  ├── GitHub Actions (CI/CD)
  │     ├── cargo tauri build (Windows のみ)
  │     ├── cargo build (CLI: 全プラットフォーム)
  │     ├── Ed25519 署名 (Windows NSIS)
  │     ├── latest.json 生成 (Windows-only)
  │     └── GitHub Releases へアップロード
  │
  └── GitHub Releases (配布ポイント)
        ├── cloto-system_0.5.3_x64-setup.exe       (Windows NSIS)
        ├── cloto-system_0.5.3_x64-setup.nsis.zip  (Tauri updater用)
        ├── cloto-system_0.5.3_x64-setup.nsis.zip.sig (Ed25519署名)
        ├── cloto-0.5.3-linux-x64.tar.gz           (CLI: Linux x64)
        ├── cloto-0.5.3-macos-arm64.tar.gz         (CLI: macOS arm64)
        ├── latest.json                             (自動更新: Windows-only)
        ├── SHA256SUMS.txt                          (チェックサム)
        └── SHA256SUMS.txt.sig                      (cosign署名)
```

### 2.2 ユーザーフロー

```
カジュアルユーザー:
  1. GitHub Releases → cloto-setup-x.y.z.exe ダウンロード
  2. ダブルクリック → NSIS インストーラー起動
  3. インストールレベル選択 (Full / Core / Custom)
  4. インストール先・オプション選択
  5. インストール完了 → デスクトップアプリ起動
  6. 初回起動: セットアップウィザード (Phase 2)
  7. 以降: 起動時に自動更新チェック (Phase 3)
```

---

## 3. Phase 1: CI/CD リリースビルド

### 3.1 現状と課題

**既存の `release.yml`**:
- CLIバイナリ (`cloto_system`) のマルチプラットフォームビルド
- Inno Setup による Windows GUI インストーラー
- cosign によるチェックサム署名

**課題**:
- Tauri デスクトップアプリ (`app.exe`) をビルドしていない
- Inno Setup インストーラーは CLI バイナリのみ同梱
- MCP サーバー (Python) をバンドルしていない
- 自動更新のための Ed25519 署名がない

### 3.2 Tauri ビルドの追加

`release.yml` に Tauri ビルドジョブを追加する。

```yaml
build-tauri:
  name: Build Tauri Installer (Windows)
  needs: build-dashboard
  runs-on: windows-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-node@v4
      with:
        node-version: "20"
    - uses: dtolnay/rust-toolchain@stable
      with:
        targets: x86_64-pc-windows-msvc
    - name: Install frontend dependencies
      run: npm ci
      working-directory: dashboard
    - name: Build Tauri
      uses: tauri-apps/tauri-action@v0
      env:
        GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
        TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
      with:
        projectPath: dashboard
        tauriScript: npx tauri
```

> **Note:** Tauri ビルドは Windows のみ。Linux/macOS は CLI バイナリ (既存 `build` ジョブ) のみ配布。

**`tauri-apps/tauri-action`** が行うこと:
- `cargo tauri build` を実行
- プラットフォーム別のインストーラーを生成 (NSIS / DMG / AppImage)
- `TAURI_SIGNING_PRIVATE_KEY` が設定されていれば Ed25519 署名を自動生成
- `.sig` ファイルを成果物として出力

### 3.3 Ed25519 鍵の生成と管理

```bash
# 鍵ペア生成
npx @tauri-apps/cli signer generate -w ~/.tauri/cloto.key
```

- **公開鍵**: `tauri.conf.json` → `plugins.updater.pubkey` にコミット
- **秘密鍵**: GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY`
- **パスワード**: GitHub Secrets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

### 3.4 自動更新マニフェスト (`latest.json`)

Tauri の updater エンドポイント用マニフェストを GitHub Release に含める。

```json
{
  "version": "0.5.3",
  "notes": "ClotoCore v0.5.3",
  "pub_date": "2026-03-04T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<Ed25519 signature>",
      "url": "https://github.com/Cloto-dev/ClotoCore/releases/download/v0.5.3/cloto-system_0.5.3_x64-setup.nsis.zip"
    }
  }
}
```

`release.yml` の `release` ジョブ内でインラインシェルスクリプトにより自動生成する
(`.sig` ファイルから署名を読み取り、Windows-only の `latest.json` を出力)。

### 3.5 既存 Inno Setup インストーラーとの関係

Tauri の NSIS インストーラーが Inno Setup を**置き換える**。

| 項目 | Inno Setup (現行) | Tauri NSIS (新) |
|------|-------------------|-----------------|
| 対象バイナリ | CLI (`cloto_system`) | デスクトップアプリ (`app.exe`) |
| ダッシュボード | ブラウザ経由 | Tauri WebView 内蔵 |
| 自動更新 | なし | Ed25519 ネイティブ |
| コンポーネント選択 | ISS で定義済み | NSIS カスタマイズ or アプリ内 |
| 多言語 | 英語 / 日本語 | NSIS の多言語対応 |

**移行方針**:
- Phase 1: Tauri NSIS でデスクトップアプリのインストーラーを生成 (Windows のみ)
- 既存 Inno Setup (`installer/`) は **非推奨** (v0.5.11 で最後に使用)。参照用にファイルは保持
- Linux/macOS は CLI バイナリのみ配布 (Tauri ビルドなし)

---

## 4. Phase 1: NSIS インストーラーカスタマイズ

### 4.1 Tauri NSIS のカスタマイズ

`tauri.conf.json` の `bundle.nsis` セクションで制御:

```json
{
  "bundle": {
    "targets": "all",
    "nsis": {
      "displayLanguageSelector": true,
      "languages": ["English", "Japanese"],
      "installMode": "both",
      "headerImage": "icons/nsis-header.bmp",
      "sidebarImage": "icons/nsis-sidebar.bmp"
    }
  }
}
```

### 4.2 同梱コンポーネント

Tauri NSIS はデスクトップアプリ本体を同梱する。追加コンポーネント
(MCP サーバー、Python runtime) は **Phase 2 のアプリ内セットアップウィザード** で
オンデマンドダウンロード・セットアップする方式とする。

**理由**:
- Python runtime (embedded) は ~30MB、全 MCP サーバーは ~50MB+ になる
- インストーラーサイズを最小限に保つ (コアアプリのみ ~15-20MB)
- MCP サーバーの組み合わせはユーザーにより異なる
- アプリ内で「必要なものだけダウンロード」がUX上最適

### 4.3 インストールオプション

Tauri NSIS のデフォルト:
- インストール先選択 (`{autopf}\ClotoCore`)
- デスクトップショートカット作成
- スタートメニュー登録
- 多言語選択 (英語 / 日本語)

---

## 5. Phase 2: アプリ内セットアップウィザード (将来)

### 5.1 概要

初回起動時に表示されるウィザード。インストールレベルに応じてコンポーネントを
セットアップする。

### 5.2 インストールレベル

| レベル | 内容 | 対象ユーザー |
|--------|------|-------------|
| **最小** | コアアプリのみ。MCP サーバーなし | 試用・評価目的 |
| **通常** | コアアプリ + 推奨 MCP サーバー (terminal, deepseek, embedding) + Python venv 自動構築 | 一般ユーザー |
| **カスタム** | 個別に MCP サーバーを選択 | 上級ユーザー |

### 5.3 ウィザードフロー

```
Step 1: Welcome
  │  「ClotoCore へようこそ」
  │  インストールレベル選択: [最小] [通常(推奨)] [カスタム]
  │
Step 2: Components (カスタム選択時のみ)
  │  チェックリスト:
  │  ☑ terminal (コマンド実行)
  │  ☑ deepseek (推論エンジン)
  │  ☑ embedding (ベクトル検索)
  │  ☐ cerebras (高速推論)
  │  ☐ tts (音声合成)
  │  ☐ stt (音声認識)
  │  ☐ ...
  │
Step 3: API Keys (該当サーバーが選択された場合)
  │  DeepSeek API Key: [________________]
  │  Cerebras API Key: [________________]
  │  (スキップ可能 → 後でSettingsから設定)
  │
Step 4: Setup
  │  プログレスバー:
  │  [===========          ] Python venv 構築中...
  │  [==================   ] MCP サーバー初期化中...
  │  [=====================] 完了!
  │
Step 5: Complete
     「セットアップが完了しました」
     [ダッシュボードを開く]
```

### 5.4 技術的考慮事項

- **Python runtime**: Embedded Python (pystand / python-build-standalone) を使用
  - ユーザーの Python インストールに依存しない
  - ~30MB の組み込み Python を初回セットアップ時にダウンロード
- **venv 構築**: `python -m venv` + `pip install` を Tauri の shell plugin で実行
- **進捗通知**: Tauri → フロントエンド間で進捗イベントを送信
- **設定保存**: セットアップ結果を `mcp.toml` / `.env` に反映
- **スキップ可能**: ウィザードは後から Settings → Setup でやり直し可能

---

## 6. Phase 3: 自動更新 (将来)

Tauri v2 のネイティブ自動更新機能を有効化し、デスクトップアプリが自動的に
新バージョンを検知・ダウンロード・適用するシステム。

### 6.1 Option B (現行) との比較

| Feature | Option B (現行) | Tauri Native (Phase 3) |
|---------|----------------|----------------------|
| Update check | 手動ボタン | 起動時に自動チェック |
| Update apply | CLI バイナリ差し替え | Tauri ネイティブ (プラットフォーム別) |
| Signature verification | SHA256 チェックサム | Ed25519 暗号署名 |
| User experience | 手動再起動 | シームレスな再起動プロンプト |
| Platform support | CLI が PATH に必要 | Tauri ビルド全体で動作 |

### 6.2 `tauri.conf.json` updater 設定

```json
{
  "plugins": {
    "updater": {
      "pubkey": "<Ed25519 public key>",
      "endpoints": [
        "https://github.com/Cloto-dev/ClotoCore/releases/latest/download/latest.json"
      ],
      "dialog": true
    }
  }
}
```

### 6.3 Dashboard UI 統合

**起動時の自動チェック** (App.tsx またはルートレイアウト):

```typescript
import { check } from '@tauri-apps/plugin-updater';

useEffect(() => {
  if (!isTauri) return;
  check().then(update => {
    if (update?.available) {
      // 通知またはモーダルを表示
    }
  }).catch(console.error);
}, []);
```

**Settings → About での手動チェック**:

```typescript
import { check } from '@tauri-apps/plugin-updater';

const update = await check();
if (update?.available) {
  await update.downloadAndInstall();
  // ユーザーに再起動を促す
}
```

### 6.4 Option B からの移行手順

1. Ed25519 鍵ペアを生成 (§3.3 参照)
2. 公開鍵を `tauri.conf.json` に追加
3. 秘密鍵を GitHub Secrets に登録
4. `release.yml` に署名 + マニフェスト生成を追加
5. `AboutSection.tsx` の `checkForUpdates()` を Tauri ネイティブチェックに置換
6. CLI shell 実行パスを削除 (非 Tauri 環境用のフォールバックは維持)
7. フルサイクルテスト: ビルド → リリース → 自動更新通知 → 適用

### 6.5 前提条件

- Phase 1 の CI/CD で Ed25519 署名 + `latest.json` が生成されること
- `tauri.conf.json` に公開鍵が設定されていること
- GitHub Secrets に秘密鍵が設定されていること

---

## 7. バージョニング戦略

> バージョニングの詳細は `docs/DEVELOPMENT.md` § 3 を参照。

**リリース固有の補足**:
- `release.yml` は `-alpha` / `-beta` / `-rc` を含むバージョンを
  自動的に prerelease フラグ付きで公開する (実装済み)
- 3箇所 (`Cargo.toml`, `dashboard/package.json`, `tauri.conf.json`) を
  同時更新する必要がある。`release.yml` がタグとの一致を検証する

---

## 8. セキュリティ

### 8.1 署名チェーン

```
開発者
  │  git tag v0.5.3 && git push --tags
  │
GitHub Actions
  │  ├── バイナリビルド
  │  ├── Ed25519 署名 (Tauri updater用)
  │  ├── cosign 署名 (チェックサム用、keyless)
  │  └── SHA256 チェックサム生成
  │
GitHub Releases
  │  ├── .exe / .dmg / .AppImage (インストーラー)
  │  ├── .sig (Ed25519 署名)
  │  ├── SHA256SUMS.txt (チェックサム)
  │  └── SHA256SUMS.txt.sig (cosign 署名)
  │
エンドユーザー
     ├── インストーラー: OS が署名を検証
     └── 自動更新: Tauri が Ed25519 を検証
```

### 8.2 鍵管理

| 鍵 | 保管場所 | 用途 |
|----|---------|------|
| Ed25519 秘密鍵 | GitHub Secrets | Tauri 自動更新の署名 |
| Ed25519 公開鍵 | `tauri.conf.json` (リポジトリ) | クライアント側の署名検証 |
| cosign | keyless (Sigstore) | チェックサムの署名 |

---

## 9. 実装ロードマップ

### v0.5.x: UI/UX 最適化フェーズ (現在)

ダッシュボードの操作性・視認性を改善し、インストーラー配布前に
UIを安定させるフェーズ。

- 常時サイドバーレイアウト (AppLayout + AppSidebar)
- 統一モーダルシステム (Modal コンポーネント)
- MCP ページのカードグリッド化
- サイドバー折りたたみ機能
- ブラウザ履歴ナビゲーション (戻る/進む)
- ダークテーマ調整
- その他UI改善・バグ修正

### v0.6.0: Phase 1 — CI/CD + Tauri インストーラー配布

1. Ed25519 鍵ペア生成、GitHub Secrets 登録
2. `tauri.conf.json` に公開鍵追加、NSIS設定 (Windows-only targets)
3. `release.yml`: `build-installer` (Inno Setup) → `build-tauri` (NSIS) に置換
4. `latest.json` を `release` ジョブ内でインライン生成 (Windows-only)
5. Inno Setup ファイルを非推奨化 (参照用保持)
6. テストリリース (`v0.6.0-alpha.1`) で検証

### v0.7.0: Phase 2 — アプリ内セットアップウィザード

1. ウィザード UI コンポーネント設計
2. Embedded Python ダウンロード・展開ロジック
3. MCP サーバー選択・インストールフロー
4. API キー設定フロー
5. Settings からの再実行機能

### v0.7.x: Phase 3 — 自動更新有効化

1. `latest.json` の自動生成確認
2. Dashboard に更新チェック UI 追加
3. 更新ダウンロード・適用フローのテスト
4. Option B (CLI更新) からの移行

---

## 10. 関連ファイル

| ファイル | 説明 |
|---------|------|
| `.github/workflows/release.yml` | リリースビルド CI/CD (build-tauri + latest.json 生成) |
| `dashboard/src-tauri/tauri.conf.json` | Tauri 設定 (NSIS, updater, Ed25519 公開鍵) |
| `installer/cloto-setup.iss` | Inno Setup 設定 (**非推奨**: 参照用保持, v0.5.11 最終) |
| `installer/build-installer.ps1` | Inno Setup ビルドスクリプト (**非推奨**: 同上) |

---

*Document created: 2026-03-04*
