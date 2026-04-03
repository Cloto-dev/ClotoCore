# HN投稿 & リポジトリ整備計画（ClotoCore中心）
## 2026-04-03 〜 04-10

---

## 投稿対象
**ClotoCore** (https://github.com/Cloto-dev/ClotoCore)
cpersonaはClotoCoreの一部として言及。単体投稿ではない。

## キー数字
| 指標 | 値 |
|------|-----|
| Rust LOC | ~34,000 |
| TypeScript LOC | ~17,000 |
| Total LOC | ~51,000 |
| MCP/MGP servers | 17 |
| Tools | 100+ |
| Tests | 351 (Rust 234 + Python 117) |
| Dev cost | $230 |
| Dev period | 3ヶ月（2026年1月〜） |
| Developer | Solo |

## 訴求軸（2軸）
1. **AIエージェント基盤** — Rustカーネル、MCPプラグイン、サンドボックス、GUIダッシュボード
2. **$230ソロ開発** — AI-assisted developmentのdata point

---

## 今日完了した作業
- [x] cpersona専用README.md新規作成（HN経由の流入用）
- [x] cloto-mcp-servers/README.md → v2.4.4対応更新
- [x] ClotoCore/README.md → テスト数・embedding記述更新
- [x] 3ファイル間の整合性検証

## 残タスク

### 博士側（手動）
| タスク | 期限 |
|--------|------|
| README内容レビュー | 4/4 |
| git commit & push（cloto-mcp-servers + ClotoCore） | 4/4 |
| GitHub Sponsors設置確認 | 4/6 |
| テスト全通過確認 | 4/7 |
| HN投稿文の最終レビュー・暗記 | 4/7 |
| ダッシュボードスクリーンショット最新化（任意） | 4/7 |

### 次のセッション
| タスク | 説明 |
|--------|------|
| HN投稿文の最終磨き | 博士のフィードバック反映 |
| Q&Aの追加・修正 | 実際のHN投稿例を参考に調整 |
| self-commentの精査 | 4項目の優先順位・長さ調整 |

---

## タイムライン

| 日付 | タスク |
|------|--------|
| 4/3 (木) | ✅ README整備 + HN投稿文ドラフト + Q&A完成 |
| 4/4 (金) | 博士レビュー → commit & push |
| 4/5 (土) | Q&A暗記 + 最終調整 |
| 4/6 (日) | QRトラッキング案件締切対応 |
| 4/7 (月) | 最終確認（テスト・Sponsors・スクショ） |
| 4/8 (火) | **HN投稿** 🎯 JST 01:00-03:00 = US西海岸火曜午前 |
| 4/9 (水) | HNコメント対応 |
| 4/10 (木) | 予備日 |

---

## 投稿前チェックリスト

- [ ] ClotoCore/README.md が最新かつ正確
- [ ] cloto-mcp-servers/README.md がv2.4.4と整合
- [ ] cpersona/README.md が存在しHN経由の流入に対応
- [ ] GitHub Sponsors設置済み
- [ ] ライセンスファイル確認（ClotoCore=BSL 1.1, cpersona=MIT）
- [ ] テスト全通過確認（cargo test + pytest）
- [ ] ダッシュボードスクリーンショットが最新
- [ ] HN投稿文の最終レビュー完了
- [ ] self-commentの準備完了
- [ ] Q&A回答の暗記完了

---

## 成果物一覧

| ファイル | 内容 |
|----------|------|
| `hn-show-hn-clotocore.md` | HN投稿文ドラフト + self-comment + タイミング表 |
| `hn-qa-clotocore.md` | 想定Q&A 18問（技術8 + 開発プロセス4 + ビジネス3 + 批判4） |
| `hn-plan-clotocore.md` | この計画書 |
| `servers/cpersona/README.md` | cpersona専用README（新規、リポジトリに直接書き込み済み） |
