# Discord Bridge — Plan C Design

**Version:** 0.1.0-draft
**Status:** Draft
**Date:** 2026-03-01

---

## 1. Overview

A bridge system for operating ClotoCore agents on Discord.
With a 1 bot = 1 agent mapping, agents can autonomously converse
and execute tasks within Discord channels.

### 1.1 Design Principles

- **Plan C (Hybrid)**: Two-process architecture consisting of a Bridge process + MCP server
- **Receive-to-respond is automatic delivery** (Bridge subscribes to SSE / MGP events and receives ThoughtResponses)
- **Active operations use MCP tools** (agent calls `discord.send_message`, etc.)
- **Separation of stdio and WebSocket** (completely avoids MCP stdio contamination)

### 1.2 Problems Solved

| Problem | Solution |
|---------|----------|
| Coexistence of MCP stdio and Discord WebSocket | Two-process separation (Bridge + MCP) |
| Response delivery destination control | Route accurately by attaching `discord_channel_id` in metadata |
| Bot token collision | Only Bridge holds the token |
| Agent's active Discord operations | MCP tool → Bridge internal HTTP API |
| Scheduled posts from cron jobs | Request to Bridge via MCP tool |

---

## 2. Architecture

### 2.1 Component Structure

```
┌──────────────────────────────────────────────┐
│              discord_bridge.py                │
│              (persistent process)             │
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
│ Kernel         │◄────────────►│ (MCP server)    │
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

### 2.2 Data Flow

#### Receive Flow (Discord → Agent → Discord)

```
1. User sends message in Discord channel
2. discord_bridge.py receives via WebSocket (on_message)
3. Bridge POSTs to ClotoCore API:
   POST /api/message
   {
     "content": "Hello",
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

#### Active Flow (Agent → Discord)

```
1. Agent (e.g., via Cron job) decides to post to Discord
2. Agent calls MCP tool: discord.send_message
   {
     "channel_id": "987654321",
     "content": "Here is the periodic report: ..."
   }
3. discord_mcp.py receives tool call via stdio
4. MCP server POSTs to Bridge internal HTTP API:
   POST http://localhost:18900/send
   { "channel_id": "987654321", "content": "..." }
5. Bridge sends message via Discord API
6. MCP server returns result to Kernel
```

---

## 3. Bridge Process (`discord_bridge.py`)

### 3.1 Responsibilities

1. **Discord Gateway connection** — Always-on WebSocket connection, message reception
2. **ClotoCore API calls** — Forward received messages via `POST /api/message`
3. **Response listener** — Monitor SSE `/api/events` and deliver ThoughtResponses to Discord
4. **Internal HTTP API** — Accept operation requests from MCP server

### 3.2 Internal HTTP API

API exposed by Bridge on `localhost:18900`. Used only by the MCP server.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/send` | Send message |
| POST | `/react` | Add reaction |
| POST | `/edit` | Edit message |
| POST | `/delete` | Delete message |
| GET | `/channels/{guild_id}` | Channel list |
| GET | `/history/{channel_id}` | Message history |
| GET | `/health` | Health check |

### 3.3 Configuration

```env
# Environment variables for discord_bridge.py
DISCORD_BOT_TOKEN=          # Discord Bot token (required)
CLOTO_API_URL=http://localhost:8081/api  # ClotoCore API endpoint
CLOTO_API_KEY=              # ClotoCore admin API key
TARGET_AGENT_ID=agent.cloto_default      # Target agent ID
BRIDGE_PORT=18900           # Internal HTTP API port
ALLOWED_CHANNEL_IDS=        # Channels to respond in (empty = all channels)
BOT_PREFIX=!                # Command prefix (empty = respond to all messages)
```

### 3.4 Response Routing

Bridge tracks the `source_message_id` of ThoughtResponse events and
routes responses to the Discord channel of the original message.

```python
# Tracking map for sent messages
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

### 3.5 Message Format

Discord → ClotoCore conversion:

| Discord | ClotoCore |
|---------|-----------|
| `message.content` | `content` |
| `message.author.id` | `source.User.id` = `"discord:{id}"` |
| `message.author.display_name` | `source.User.name` |
| `message.attachments` | `metadata.attachments` (URL list) |
| `message.channel.id` | `metadata.discord_channel_id` |
| `message.guild.id` | `metadata.discord_guild_id` |

ClotoCore → Discord conversion:

| ClotoCore | Discord |
|-----------|---------|
| ThoughtResponse.content | 2000 chars or less: single message / exceeds: split send |
| Code blocks (```) | Sent as-is as Discord code blocks |
| Long text (over 4000 chars) | Sent as file attachment |

---

## 4. MCP Server (`discord_mcp.py`)

### 4.1 Tool List

| Tool Name | Description | Input |
|-----------|-------------|-------|
| `send_message` | Send message to channel | `channel_id`, `content`, `embed?` |
| `add_reaction` | Add reaction to message | `channel_id`, `message_id`, `emoji` |
| `list_channels` | Get guild channel list | `guild_id` |
| `get_history` | Get recent messages from channel | `channel_id`, `limit?` |
| `edit_message` | Edit a sent message | `channel_id`, `message_id`, `content` |
| `delete_message` | Delete a message | `channel_id`, `message_id` |

### 4.2 Implementation Pattern

The MCP server is a thin wrapper that calls the Bridge's internal HTTP API:

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

### 4.3 MCP Server Configuration

```json
// Add to mcp-servers.json
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

## 5. MGP Integration (Future)

When MGP Section 13 is implemented, the response listener can migrate from SSE to MGP event subscription.

### 5.1 SSE Method (Current)

```python
# Bridge directly listens to ClotoCore's SSE
async for event in sse_stream(f"{CLOTO_API_URL}/events"):
    if event.type == "ThoughtResponse":
        await route_to_discord(event)
```

### 5.2 MGP Method (Future)

```
If Bridge also operates as an MCP server:
  → mgp.events.subscribe(["ThoughtResponse"])
  → Receive responses via notifications/mgp.event
  → SSE listener not needed
```

However, if Bridge is converted to an MCP server, the stdio and WebSocket coexistence
problem would resurface. Unless MGP has a transport layer extension (Section 14, etc.),
the two-process architecture will be maintained.

---

## 6. Security

### 6.1 Access Control

| Layer | Control |
|-------|---------|
| Discord | Channel restriction via `ALLOWED_CHANNEL_IDS` |
| Bridge HTTP | Bound to localhost only (no external access) |
| ClotoCore API | Authenticated via `CLOTO_API_KEY` |
| MCP Tools | Per-agent control via `resolve_tool_access()` |

### 6.2 Discord-Specific Considerations

- **Bot token**: Held only by Bridge process. Not passed to MCP server
- **Mentions**: Bridge blocks sending of `@everyone` and `@here`
- **Rate limiting**: Bridge centrally manages Discord API rate limits
- **Message length**: Bridge auto-splits at the 2000-character limit
- **DMs**: Disabled by default. Enable with `ALLOW_DM=true`

### 6.3 Guardrail Compliance

| Guardrail | Response |
|-----------|----------|
| #1 Security | API key authentication, localhost binding |
| #6 Physical Safety | N/A (no physical device operations) |
| #7 External Processes | Manage Bridge via systemd/pm2, health checks |
| #8 Privacy | Discord user IDs can be hashed (configurable) |

---

## 7. Deployment

### 7.1 Process Management

```
                    ┌─ cloto_core (Kernel)
                    │
systemd / pm2 ──────┼─ discord_bridge.py (persistent)
                    │
                    └─ MCP server group (launched by Kernel)
                        └─ discord_mcp.py
```

- `discord_bridge.py` runs as a process independent of Kernel
- Discord connection can be maintained during Kernel restarts
- Bridge auto-restarts on crash (systemd Restart=always)

### 7.2 Startup Order

```
1. ClotoCore Kernel starts
2. discord_bridge.py starts (confirms connection to Kernel API)
3. Kernel launches discord_mcp.py as MCP server
4. MCP server performs Bridge health check (GET /health)
5. Ready
```

---

## 8. File Structure

```
mcp-servers/discord/
├── bridge.py           # Discord Bridge (persistent process)
├── server.py           # MCP server (tool provider)
├── pyproject.toml
└── README.md
```

---

## 9. Constraints and Open Issues

| Item | Status | Notes |
|------|--------|-------|
| Voice channel support | Out of scope | Text channels only |
| Thread support | Phase 2 | Requires `thread_id` tracking |
| Embeds / Buttons | Phase 2 | Rich messages |
| Multiple guilds | Supported in Phase 1 | Controlled via `ALLOWED_CHANNEL_IDS` |
| MGP transport extension | TBD | Prerequisite for SSE → MGP migration |
