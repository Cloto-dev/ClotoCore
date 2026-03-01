# Discord Bridge — 案 C 設計

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-01

---

## 1. 概要

ClotoCore エージェントを Discord 上で運用するためのブリッジシステム。
1 bot = 1 agent のマッピングで、エージェントが Discord チャンネル内で
自律的に会話・タスク実行を行えるようにする。

### 1.1 設計方針

- **案 C（ハイブリッド）**: Bridge プロセス + MCP サーバーの2プロセス構成
- **受信→応答は自動配送**（Bridge が SSE / MGP イベント購読で ThoughtResponse を受信）
- **能動操作は MCP ツール**（エージェントが `discord.send_message` 等を呼び出す）
- **stdio と WebSocket の分離**（MCP の stdio 汚染を完全回避）

### 1.2 解決する問題

| 問題 | 解決策 |
|------|--------|
| MCP stdio と Discord WebSocket の同居 | 2プロセス分離（Bridge + MCP） |
| 応答の配送先制御 | metadata に `discord_channel_id` を付与して正確にルーティング |
| Bot トークンの衝突 | Bridge のみがトークンを所有 |
| エージェントの能動的な Discord 操作 | MCP ツール → Bridge 内部 HTTP API 経由 |
| Cron ジョブからの定期投稿 | MCP ツール経由で Bridge にリクエスト |

---

## 2. アーキテクチャ

### 2.1 コンポーネント構成

```
┌──────────────────────────────────────────────┐
│              discord_bridge.py                │
│              (常駐プロセス)                    │
│                                              │
│  ┌────────────┐  ┌───────────┐  ┌─────────┐ │
│  │ Discord    │  │ SSE/MGP   │  │ Internal│ │
│  │ Gateway    │  │ Listener  │  │ HTTP API│ │
│  │ (WebSocket)│  │           │  │ :18900  │ │
│  └─────┬──────┘  └─────┬─────┘  └────┬────┘ │
│        │               │              │      │
│        │  on_message    │ ThoughtResp  │      │
│        ▼               ▼              ▼      │
│  ┌──────────────────────────────────────────┐│
│  │           Event Router                   ││
│  │  Discord msg → POST /api/message         ││
│  │  ThoughtResp → Discord channel           ││
│  │  HTTP req    → Discord API call          ││
│  └──────────────────────────────────────────┘│
└──────────────────────────────────────────────┘
        ▲                              ▲
        │ POST /api/message            │ HTTP localhost:18900
        ▼                              │
┌────────────────┐              ┌──────┴──────────┐
│ ClotoCore      │   stdio      │ discord_mcp.py  │
│ Kernel         │◄────────────►│ (MCP サーバー)   │
│                │              │                 │
│                │              │ Tools:          │
│                │              │  send_message   │
│                │              │  add_reaction   │
│                │              │  list_channels  │
│                │              │  get_history    │
│                │              │  edit_message   │
│                │              │  delete_message │
└────────────────┘              └─────────────────┘
```

### 2.2 データフロー

#### 受信フロー（Discord → Agent → Discord）

```
1. User sends message in Discord channel
2. discord_bridge.py receives via WebSocket (on_message)
3. Bridge POSTs to ClotoCore API:
   POST /api/message
   {
     "content": "こんにちは",
     "source": {"User": {"id": "discord:123456", "name": "shiminiku"}},
     "metadata": {
       "target_agent_id": "agent.cloto_default",
       "discord_channel_id": "987654321",
       "discord_message_id": "111222333",
       "discord_guild_id": "444555666"
     }
   }
4. Kernel routes to agent → agentic loop → ThoughtResponse
5. Bridge receives ThoughtResponse via SSE (or MGP notifications/mgp.event)
6. Bridge extracts discord_channel_id from source_message metadata
7. Bridge sends response to Discord channel via Discord API
```

#### 能動フロー（Agent → Discord）

```
1. Agent (e.g., via Cron job) decides to post to Discord
2. Agent calls MCP tool: discord.send_message
   {
     "channel_id": "987654321",
     "content": "定期レポートです: ..."
   }
3. discord_mcp.py receives tool call via stdio
4. MCP server POSTs to Bridge internal HTTP API:
   POST http://localhost:18900/send
   { "channel_id": "987654321", "content": "..." }
5. Bridge sends message via Discord API
6. MCP server returns result to Kernel
```

---

## 3. Bridge プロセス (`discord_bridge.py`)

### 3.1 責務

1. **Discord Gateway 接続** — WebSocket で常時接続、メッセージ受信
2. **ClotoCore API 呼出し** — 受信メッセージを `POST /api/message` で転送
3. **応答リスナー** — SSE `/api/events` を監視し、ThoughtResponse を Discord に配送
4. **内部 HTTP API** — MCP サーバーからの操作リクエストを受付

### 3.2 内部 HTTP API

Bridge が `localhost:18900` で公開する API。MCP サーバーのみが使用する。

| Method | Path | 説明 |
|--------|------|------|
| POST | `/send` | メッセージ送信 |
| POST | `/react` | リアクション追加 |
| POST | `/edit` | メッセージ編集 |
| POST | `/delete` | メッセージ削除 |
| GET | `/channels/{guild_id}` | チャンネル一覧 |
| GET | `/history/{channel_id}` | メッセージ履歴 |
| GET | `/health` | ヘルスチェック |

### 3.3 設定

```env
# discord_bridge.py の環境変数
DISCORD_BOT_TOKEN=          # Discord Bot トークン（必須）
CLOTO_API_URL=http://localhost:8081/api  # ClotoCore API エンドポイント
CLOTO_API_KEY=              # ClotoCore 管理 API キー
TARGET_AGENT_ID=agent.cloto_default      # 対象エージェント ID
BRIDGE_PORT=18900           # 内部 HTTP API ポート
ALLOWED_CHANNEL_IDS=        # 応答するチャンネル（空=全チャンネル）
BOT_PREFIX=!                # コマンドプレフィックス（空=全メッセージに応答）
```

### 3.4 応答ルーティング

Bridge は ThoughtResponse イベントの `source_message_id` を追跡し、
元の Discord メッセージのチャンネルに応答をルーティングする。

```python
# 送信メッセージの追跡マップ
pending_responses: dict[str, str] = {}
# key: ClotoCore message ID → value: Discord channel ID

async def on_discord_message(message):
    cloto_msg_id = generate_id()
    pending_responses[cloto_msg_id] = str(message.channel.id)
    await post_to_cloto(cloto_msg_id, message)

async def on_thought_response(event):
    source_id = event["source_message_id"]
    channel_id = pending_responses.pop(source_id, None)
    if channel_id:
        channel = bot.get_channel(int(channel_id))
        await channel.send(event["content"])
```

### 3.5 メッセージフォーマット

Discord → ClotoCore の変換:

| Discord | ClotoCore |
|---------|-----------|
| `message.content` | `content` |
| `message.author.id` | `source.User.id` = `"discord:{id}"` |
| `message.author.display_name` | `source.User.name` |
| `message.attachments` | `metadata.attachments` (URL リスト) |
| `message.channel.id` | `metadata.discord_channel_id` |
| `message.guild.id` | `metadata.discord_guild_id` |

ClotoCore → Discord の変換:

| ClotoCore | Discord |
|-----------|---------|
| ThoughtResponse.content | 2000文字以下: 単一メッセージ / 超過: 分割送信 |
| コードブロック (```) | Discord のコードブロックとしてそのまま送信 |
| 長文 (4000文字超) | ファイル添付として送信 |

---

## 4. MCP サーバー (`discord_mcp.py`)

### 4.1 ツール一覧

| ツール名 | 説明 | 入力 |
|----------|------|------|
| `send_message` | チャンネルにメッセージ送信 | `channel_id`, `content`, `embed?` |
| `add_reaction` | メッセージにリアクション追加 | `channel_id`, `message_id`, `emoji` |
| `list_channels` | ギルドのチャンネル一覧取得 | `guild_id` |
| `get_history` | チャンネルの直近メッセージ取得 | `channel_id`, `limit?` |
| `edit_message` | 送信済みメッセージの編集 | `channel_id`, `message_id`, `content` |
| `delete_message` | メッセージ削除 | `channel_id`, `message_id` |

### 4.2 実装パターン

MCP サーバーは Bridge の内部 HTTP API を呼ぶ薄いラッパー:

```python
BRIDGE_URL = os.environ.get("DISCORD_BRIDGE_URL", "http://localhost:18900")

async def do_send_message(channel_id: str, content: str) -> dict:
    async with httpx.AsyncClient() as client:
        resp = await client.post(
            f"{BRIDGE_URL}/send",
            json={"channel_id": channel_id, "content": content},
        )
        resp.raise_for_status()
        return resp.json()
```

### 4.3 MCP サーバー設定

```json
// mcp-servers.json に追加
{
  "discord": {
    "command": "python",
    "args": ["-m", "cloto_mcp_discord"],
    "env": {
      "DISCORD_BRIDGE_URL": "http://localhost:18900"
    }
  }
}
```

---

## 5. MGP 連携（将来）

MGP §13 が実装された場合、応答リスナーを SSE から MGP イベント購読に移行できる。

### 5.1 SSE 方式（現在）

```python
# Bridge が ClotoCore の SSE を直接リスン
async for event in sse_stream(f"{CLOTO_API_URL}/events"):
    if event.type == "ThoughtResponse":
        await route_to_discord(event)
```

### 5.2 MGP 方式（将来）

```
Bridge が MCP サーバーとしても動作する場合:
  → mgp.events.subscribe(["ThoughtResponse"])
  → notifications/mgp.event で応答受信
  → SSE リスナー不要
```

ただし、Bridge を MCP サーバー化すると stdio と WebSocket の同居問題が再発するため、
MGP にトランスポート層拡張（§14 等）がない限り、2プロセス構成は維持する。

---

## 6. セキュリティ

### 6.1 アクセス制御

| 層 | 制御 |
|----|------|
| Discord | `ALLOWED_CHANNEL_IDS` でチャンネル制限 |
| Bridge HTTP | localhost のみバインド（外部アクセス不可） |
| ClotoCore API | `CLOTO_API_KEY` で認証 |
| MCP ツール | `resolve_tool_access()` でエージェント単位の制御 |

### 6.2 Discord 固有の考慮事項

- **Bot トークン**: Bridge プロセスのみが保持。MCP サーバーには渡さない
- **メンション**: `@everyone`, `@here` の送信を Bridge 側でブロック
- **レート制限**: Discord API のレート制限を Bridge が一元管理
- **メッセージ長**: 2000 文字制限を Bridge が自動分割
- **DM**: デフォルト無効。`ALLOW_DM=true` で有効化

### 6.3 ガードレール準拠

| ガードレール | 対応 |
|-------------|------|
| #1 セキュリティ | API キー認証、localhost バインド |
| #6 物理安全 | N/A（物理デバイス操作なし） |
| #7 外部プロセス | Bridge を systemd/pm2 で管理、ヘルスチェック |
| #8 プライバシー | Discord ユーザー ID はハッシュ化可能（設定） |

---

## 7. デプロイメント

### 7.1 プロセス管理

```
                    ┌─ cloto_core (Kernel)
                    │
systemd / pm2 ──────┼─ discord_bridge.py (常駐)
                    │
                    └─ MCP サーバー群 (Kernel が起動)
                        └─ discord_mcp.py
```

- `discord_bridge.py` は Kernel とは独立したプロセスとして起動
- Kernel の再起動時も Discord 接続を維持可能
- Bridge のクラッシュ時は自動再起動（systemd Restart=always）

### 7.2 起動順序

```
1. ClotoCore Kernel 起動
2. discord_bridge.py 起動（Kernel API への接続を確認）
3. Kernel が discord_mcp.py を MCP サーバーとして起動
4. MCP サーバーが Bridge ヘルスチェック（GET /health）
5. 準備完了
```

---

## 8. ファイル構成

```
mcp-servers/discord/
├── bridge.py           # Discord Bridge（常駐プロセス）
├── server.py           # MCP サーバー（ツール提供）
├── pyproject.toml
└── README.md
```

---

## 9. 制約と未解決事項

| 項目 | 状態 | 備考 |
|------|------|------|
| 音声チャンネル対応 | スコープ外 | テキストチャンネルのみ |
| スレッド対応 | Phase 2 | `thread_id` の追跡が必要 |
| Embed / ボタン | Phase 2 | リッチメッセージ |
| 複数ギルド | Phase 1 で対応 | `ALLOWED_CHANNEL_IDS` で制御 |
| MGP トランスポート拡張 | 未定 | SSE → MGP 移行の前提条件 |
