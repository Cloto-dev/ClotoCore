"""
Exiv MCP Server: KS2.2 Memory
Persistent memory with FTS5 full-text search and pluggable vector embedding.
Ported from plugins/ks22/src/lib.rs + ai_karin KS2.1 architecture.

Phase 1: store, recall (FTS5 + keyword), update_profile (stub), archive_episode (simple)
"""

import asyncio
import json
import os
import re
import hashlib
from datetime import datetime, timezone

import aiosqlite
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

# ============================================================
# Configuration
# ============================================================

DB_PATH = os.environ.get("KS22_DB_PATH", "data/ks22_memory.db")
MAX_MEMORIES = int(os.environ.get("KS22_MAX_MEMORIES", "500"))
FTS_ENABLED = os.environ.get("KS22_FTS_ENABLED", "true").lower() == "true"

# Embedding (Phase 2 — not yet active)
EMBEDDING_MODE = os.environ.get("KS22_EMBEDDING_MODE", "none")
EMBEDDING_URL = os.environ.get("KS22_EMBEDDING_URL", "")

# ============================================================
# Database
# ============================================================

SCHEMA_VERSION = 1

SCHEMA_SQL = """
CREATE TABLE IF NOT EXISTS schema_version (
    version    INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS memories (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    msg_id     TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL,
    source     TEXT NOT NULL DEFAULT '{}',
    timestamp  TEXT NOT NULL,
    metadata   TEXT NOT NULL DEFAULT '{}',
    embedding  BLOB,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_memories_agent
    ON memories(agent_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_memories_msg_id
    ON memories(agent_id, msg_id);

CREATE TABLE IF NOT EXISTS profiles (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    user_id    TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(agent_id, user_id)
);

CREATE TABLE IF NOT EXISTS episodes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id   TEXT NOT NULL,
    summary    TEXT NOT NULL,
    keywords   TEXT NOT NULL DEFAULT '',
    embedding  BLOB,
    start_time TEXT,
    end_time   TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_episodes_agent
    ON episodes(agent_id, created_at DESC);
"""

FTS_SQL = """
CREATE VIRTUAL TABLE IF NOT EXISTS episodes_fts USING fts5(
    summary,
    keywords,
    content=episodes,
    content_rowid=id
);

-- Sync triggers
CREATE TRIGGER IF NOT EXISTS episodes_ai AFTER INSERT ON episodes BEGIN
    INSERT INTO episodes_fts(rowid, summary, keywords)
    VALUES (new.id, new.summary, new.keywords);
END;

CREATE TRIGGER IF NOT EXISTS episodes_ad AFTER DELETE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
    VALUES ('delete', old.id, old.summary, old.keywords);
END;

CREATE TRIGGER IF NOT EXISTS episodes_au AFTER UPDATE ON episodes BEGIN
    INSERT INTO episodes_fts(episodes_fts, rowid, summary, keywords)
    VALUES ('delete', old.id, old.summary, old.keywords);
    INSERT INTO episodes_fts(rowid, summary, keywords)
    VALUES (new.id, new.summary, new.keywords);
END;
"""

_db: aiosqlite.Connection | None = None


async def get_db() -> aiosqlite.Connection:
    """Get or create the database connection."""
    global _db
    if _db is not None:
        return _db

    # Ensure parent directory exists
    db_dir = os.path.dirname(DB_PATH)
    if db_dir:
        os.makedirs(db_dir, exist_ok=True)

    _db = await aiosqlite.connect(DB_PATH)
    await _db.execute("PRAGMA journal_mode=WAL")
    await _db.execute("PRAGMA synchronous=NORMAL")

    # Apply schema
    await _db.executescript(SCHEMA_SQL)

    # Apply FTS if enabled
    if FTS_ENABLED:
        await _db.executescript(FTS_SQL)

    # Track schema version
    row = await _db.execute_fetchall(
        "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1"
    )
    current = row[0][0] if row else 0
    if current < SCHEMA_VERSION:
        await _db.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?)",
            (SCHEMA_VERSION,),
        )
        await _db.commit()

    return _db


async def close_db():
    """Close the database connection."""
    global _db
    if _db is not None:
        await _db.close()
        _db = None


# ============================================================
# Memory Operations
# ============================================================


def generate_mem_key(agent_id: str, message: dict) -> str:
    """Generate a unique key for a memory entry (KS2.2 compatible)."""
    ts = message.get("timestamp", datetime.now(timezone.utc).isoformat())
    content = message.get("content", "")
    hash_input = f"{agent_id}:{ts}:{content}"
    short_hash = hashlib.sha256(hash_input.encode()).hexdigest()[:8]
    return f"mem:{agent_id}:{ts}:{short_hash}"


async def do_store(agent_id: str, message: dict) -> dict:
    """Store a message in agent memory."""
    db = await get_db()

    msg_id = message.get("id", "")
    content = message.get("content", "")
    source = json.dumps(message.get("source", {}))
    timestamp = message.get(
        "timestamp", datetime.now(timezone.utc).isoformat()
    )
    metadata = json.dumps(message.get("metadata", {}))

    if not content:
        return {"ok": True, "skipped": True, "reason": "empty content"}

    # Deduplicate by msg_id if provided
    if msg_id:
        row = await db.execute_fetchall(
            "SELECT id FROM memories WHERE agent_id = ? AND msg_id = ? LIMIT 1",
            (agent_id, msg_id),
        )
        if row:
            return {"ok": True, "skipped": True, "reason": "duplicate msg_id"}

    await db.execute(
        """INSERT INTO memories (agent_id, msg_id, content, source, timestamp, metadata)
           VALUES (?, ?, ?, ?, ?, ?)""",
        (agent_id, msg_id, content, source, timestamp, metadata),
    )
    await db.commit()
    return {"ok": True}


async def do_recall(agent_id: str, query: str, limit: int) -> dict:
    """Recall relevant memories using multi-strategy search."""
    db = await get_db()
    results: list[dict] = []
    seen_ids: set[int] = set()

    # Strategy 1: FTS5 episode search (if enabled and query non-empty)
    if FTS_ENABLED and query.strip():
        fts_results = await _search_episodes_fts(db, agent_id, query, limit)
        for row in fts_results:
            if row["id"] not in seen_ids:
                results.append(row)
                seen_ids.add(row["id"])

    # Strategy 2: Profile lookup
    profile_rows = await db.execute_fetchall(
        "SELECT content FROM profiles WHERE agent_id = ? ORDER BY updated_at DESC LIMIT 3",
        (agent_id,),
    )
    for (profile_content,) in profile_rows:
        # Inject profile as a system-context memory
        results.append({
            "id": -1,
            "content": f"[Profile] {profile_content}",
            "source": {"System": "profile"},
            "timestamp": "",
        })

    # Strategy 3: Keyword match on memories (KS2.2 fallback)
    remaining = max(0, limit - len(results))
    if remaining > 0:
        memory_rows = await _search_memories_keyword(
            db, agent_id, query, remaining
        )
        for row in memory_rows:
            if row["id"] not in seen_ids:
                results.append(row)
                seen_ids.add(row["id"])

    # Truncate to limit and reverse for chronological order (oldest first for LLM)
    results = results[:limit]
    results.reverse()

    # Convert to ExivMessage-compatible format
    messages = []
    for r in results:
        msg: dict = {"content": r["content"]}
        if r.get("source"):
            msg["source"] = r["source"] if isinstance(r["source"], dict) else _try_parse_json(r["source"])
        if r.get("timestamp"):
            msg["timestamp"] = r["timestamp"]
        if r.get("msg_id"):
            msg["id"] = r["msg_id"]
        messages.append(msg)

    return {"messages": messages}


async def _search_episodes_fts(
    db: aiosqlite.Connection, agent_id: str, query: str, limit: int
) -> list[dict]:
    """Search episodes using FTS5."""
    # Sanitize and build FTS query: each word quoted for AND matching
    sanitized = re.sub(r'["\']', "", query)
    words = sanitized.split()
    if not words:
        return []

    fts_query = " ".join(f'"{w}"' for w in words)

    rows = await db.execute_fetchall(
        """SELECT e.id, e.summary, e.start_time
           FROM episodes_fts f
           JOIN episodes e ON f.rowid = e.id
           WHERE episodes_fts MATCH ?
           AND e.agent_id = ?
           ORDER BY rank
           LIMIT ?""",
        (fts_query, agent_id, limit),
    )

    return [
        {
            "id": row[0],
            "content": f"[Episode] {row[1]}",
            "source": {"System": "episode"},
            "timestamp": row[2] or "",
        }
        for row in rows
    ]


async def _search_memories_keyword(
    db: aiosqlite.Connection, agent_id: str, query: str, limit: int
) -> list[dict]:
    """Search memories using keyword matching (KS2.2 compatible fallback)."""
    if query.strip():
        # Keyword match
        rows = await db.execute_fetchall(
            """SELECT id, msg_id, content, source, timestamp
               FROM memories
               WHERE agent_id = ?
               AND content LIKE ?
               ORDER BY created_at DESC
               LIMIT ?""",
            (agent_id, f"%{query}%", MAX_MEMORIES),
        )
    else:
        # No query — return recent memories
        rows = await db.execute_fetchall(
            """SELECT id, msg_id, content, source, timestamp
               FROM memories
               WHERE agent_id = ?
               ORDER BY created_at DESC
               LIMIT ?""",
            (agent_id, limit),
        )

    results = []
    for row in rows:
        results.append({
            "id": row[0],
            "msg_id": row[1],
            "content": row[2],
            "source": row[3],
            "timestamp": row[4],
        })
        if len(results) >= limit:
            break

    return results


async def do_update_profile(agent_id: str, history: list[dict]) -> dict:
    """Phase 1 stub: store raw summary as profile (no LLM extraction)."""
    db = await get_db()

    if not history:
        return {"ok": True, "profiles_updated": 0}

    # Simple: concatenate user messages as profile content
    user_lines = []
    for msg in history:
        source = msg.get("source", {})
        if isinstance(source, str):
            source = _try_parse_json(source)
        if "User" in source or "user" in source:
            content = msg.get("content", "")
            if content:
                user_lines.append(content)

    if not user_lines:
        return {"ok": True, "profiles_updated": 0}

    profile_content = "\n".join(user_lines[-10:])  # Keep last 10 messages

    await db.execute(
        """INSERT INTO profiles (agent_id, user_id, content, updated_at)
           VALUES (?, '', ?, datetime('now'))
           ON CONFLICT(agent_id, user_id) DO UPDATE SET
               content = excluded.content,
               updated_at = excluded.updated_at""",
        (agent_id, profile_content),
    )
    await db.commit()
    return {"ok": True, "profiles_updated": 1}


async def do_archive_episode(agent_id: str, history: list[dict]) -> dict:
    """Phase 1: simple concatenation summary + keyword extraction (no LLM)."""
    db = await get_db()

    if not history:
        return {"ok": True, "episode_id": None}

    # Build text and extract simple keywords
    lines = []
    word_freq: dict[str, int] = {}
    for msg in history:
        content = msg.get("content", "")
        if content:
            source = msg.get("source", {})
            if isinstance(source, str):
                source = _try_parse_json(source)
            speaker = "User" if ("User" in source or "user" in source) else "Agent"
            lines.append(f"[{speaker}] {content}")
            # Simple word frequency for keywords
            for word in re.findall(r'\b\w{3,}\b', content.lower()):
                word_freq[word] = word_freq.get(word, 0) + 1

    if not lines:
        return {"ok": True, "episode_id": None}

    # Summary: first and last lines + total count
    summary_parts = []
    if len(lines) <= 5:
        summary_parts = lines
    else:
        summary_parts = lines[:2] + [f"... ({len(lines) - 4} messages) ..."] + lines[-2:]
    summary = "\n".join(summary_parts)

    # Keywords: top 10 by frequency (excluding common words)
    stopwords = {"the", "and", "for", "that", "this", "with", "are", "was", "has", "have",
                 "not", "but", "you", "your", "can", "will", "from", "they", "been", "more"}
    sorted_words = sorted(
        ((w, c) for w, c in word_freq.items() if w not in stopwords),
        key=lambda x: x[1],
        reverse=True,
    )
    keywords = " ".join(w for w, _ in sorted_words[:10])

    # Timestamps
    timestamps = [msg.get("timestamp", "") for msg in history if msg.get("timestamp")]
    start_time = min(timestamps) if timestamps else None
    end_time = max(timestamps) if timestamps else None

    cursor = await db.execute(
        """INSERT INTO episodes (agent_id, summary, keywords, start_time, end_time)
           VALUES (?, ?, ?, ?, ?)""",
        (agent_id, summary, keywords, start_time, end_time),
    )
    await db.commit()
    return {"ok": True, "episode_id": cursor.lastrowid}


def _try_parse_json(s: str) -> dict:
    """Try to parse a string as JSON, return empty dict on failure."""
    try:
        return json.loads(s)
    except (json.JSONDecodeError, TypeError):
        return {}


# ============================================================
# MCP Server
# ============================================================

server = Server("exiv-mcp-ks22")


@server.list_tools()
async def list_tools() -> list[Tool]:
    return [
        Tool(
            name="store",
            description="Store a message in agent memory for future recall.",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier",
                    },
                    "message": {
                        "type": "object",
                        "description": "ExivMessage to store (id, content, source, timestamp, metadata)",
                    },
                },
                "required": ["agent_id", "message"],
            },
        ),
        Tool(
            name="recall",
            description="Recall relevant memories using multi-strategy search (FTS5 + keyword).",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier",
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query (empty returns recent memories)",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max memories to return",
                        "default": 10,
                    },
                },
                "required": ["agent_id", "query"],
            },
        ),
        Tool(
            name="update_profile",
            description="Extract user facts from conversation and merge with existing profile.",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier",
                    },
                    "history": {
                        "type": "array",
                        "description": "Recent conversation messages",
                        "items": {"type": "object"},
                    },
                },
                "required": ["agent_id", "history"],
            },
        ),
        Tool(
            name="archive_episode",
            description="Summarize and archive a conversation episode for searchable recall.",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier",
                    },
                    "history": {
                        "type": "array",
                        "description": "Conversation messages to archive",
                        "items": {"type": "object"},
                    },
                },
                "required": ["agent_id", "history"],
            },
        ),
    ]


@server.call_tool()
async def call_tool(name: str, arguments: dict) -> list[TextContent]:
    try:
        if name == "store":
            result = await do_store(
                arguments.get("agent_id", ""),
                arguments.get("message", {}),
            )
        elif name == "recall":
            result = await do_recall(
                arguments.get("agent_id", ""),
                arguments.get("query", ""),
                arguments.get("limit", 10),
            )
        elif name == "update_profile":
            result = await do_update_profile(
                arguments.get("agent_id", ""),
                arguments.get("history", []),
            )
        elif name == "archive_episode":
            result = await do_archive_episode(
                arguments.get("agent_id", ""),
                arguments.get("history", []),
            )
        else:
            result = {"error": f"Unknown tool: {name}"}

        return [TextContent(type="text", text=json.dumps(result))]
    except Exception as e:
        return [
            TextContent(type="text", text=json.dumps({"error": str(e)}))
        ]


async def main():
    # Initialize DB on startup
    await get_db()
    try:
        async with stdio_server() as (read_stream, write_stream):
            await server.run(
                read_stream, write_stream, server.create_initialization_options()
            )
    finally:
        await close_db()


if __name__ == "__main__":
    asyncio.run(main())
