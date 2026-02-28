"""
Cloto MCP Server: KS2.2 Memory
Persistent memory with FTS5 full-text search and pluggable vector embedding.
Ported from plugins/ks22/src/lib.rs + ai_karin KS2.1 architecture.

Phase 1: store, recall (FTS5 + keyword), update_profile (stub), archive_episode (simple)
Phase 2: Vector embedding integration (cosine similarity search)
"""

import asyncio
import json
import logging
import os
import re
import struct
import hashlib
from datetime import datetime, timezone

import aiosqlite
from mcp.server import Server
from mcp.server.stdio import stdio_server
from mcp.types import TextContent, Tool

logger = logging.getLogger(__name__)

# ============================================================
# Configuration
# ============================================================

DB_PATH = os.environ.get("KS22_DB_PATH", "data/ks22_memory.db")
MAX_MEMORIES = int(os.environ.get("KS22_MAX_MEMORIES", "500"))
FTS_ENABLED = os.environ.get("KS22_FTS_ENABLED", "true").lower() == "true"

# Embedding configuration
EMBEDDING_MODE = os.environ.get("KS22_EMBEDDING_MODE", "none")
EMBEDDING_URL = os.environ.get("KS22_EMBEDDING_URL", "")
EMBEDDING_API_KEY = os.environ.get("KS22_EMBEDDING_API_KEY", "")
EMBEDDING_API_URL = os.environ.get(
    "KS22_EMBEDDING_API_URL", "https://api.openai.com/v1/embeddings"
)
EMBEDDING_MODEL = os.environ.get("KS22_EMBEDDING_MODEL", "text-embedding-3-small")

# Vector search threshold (cosine similarity, 0.0-1.0)
VECTOR_MIN_SIMILARITY = float(os.environ.get("KS22_VECTOR_MIN_SIMILARITY", "0.3"))

# ============================================================
# Embedding Client
# ============================================================


class EmbeddingClient:
    """Client for computing vector embeddings via HTTP or API."""

    def __init__(
        self,
        mode: str,
        http_url: str = "",
        api_key: str = "",
        api_url: str = "",
        model: str = "",
    ):
        self.mode = mode
        self._http_url = http_url
        self._api_key = api_key
        self._api_url = api_url
        self._model = model
        self._client = None

    async def initialize(self):
        """Create persistent HTTP client."""
        import httpx

        self._client = httpx.AsyncClient(timeout=30)
        logger.info(
            "EmbeddingClient initialized (mode=%s)", self.mode,
        )

    async def close(self):
        """Close HTTP client."""
        if self._client:
            await self._client.aclose()
            self._client = None

    async def embed(self, texts: list[str]) -> list[list[float]] | None:
        """Compute embeddings. Returns None on failure (graceful degradation)."""
        if self.mode == "none" or not self._client:
            return None

        try:
            if self.mode == "http":
                return await self._embed_via_http(texts)
            elif self.mode == "api":
                return await self._embed_via_api(texts)
            else:
                logger.warning("Unknown embedding mode: %s", self.mode)
                return None
        except Exception as e:
            logger.warning("Embedding request failed: %s", e)
            return None

    async def _embed_via_http(self, texts: list[str]) -> list[list[float]] | None:
        """Call the embedding server's HTTP endpoint."""
        response = await self._client.post(
            self._http_url,
            json={"texts": texts},
        )
        response.raise_for_status()
        data = response.json()
        return data.get("embeddings")

    async def _embed_via_api(self, texts: list[str]) -> list[list[float]] | None:
        """Call OpenAI-compatible embedding API directly."""
        import numpy as np

        response = await self._client.post(
            self._api_url,
            headers={
                "Authorization": f"Bearer {self._api_key}",
                "Content-Type": "application/json",
            },
            json={"model": self._model, "input": texts},
        )
        response.raise_for_status()
        data = response.json()
        embeddings = [item["embedding"] for item in data["data"]]

        # L2-normalize for consistent cosine similarity via dot product
        result = []
        for emb in embeddings:
            vec = np.array(emb, dtype=np.float32)
            norm = np.linalg.norm(vec)
            if norm > 1e-9:
                vec = vec / norm
            result.append(vec.tolist())

        return result

    @staticmethod
    def pack_embedding(embedding: list[float]) -> bytes:
        """Pack a float list into a BLOB (little-endian float32)."""
        return struct.pack(f"<{len(embedding)}f", *embedding)

    @staticmethod
    def unpack_embedding(blob: bytes) -> list[float]:
        """Unpack a BLOB into a float list."""
        n = len(blob) // 4
        return list(struct.unpack(f"<{n}f", blob))


_embedding_client: EmbeddingClient | None = None

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

    # Compute embedding before insert (so we can include it in the INSERT)
    embedding_blob = None
    if _embedding_client:
        try:
            embeddings = await _embedding_client.embed([content])
            if embeddings and embeddings[0]:
                embedding_blob = EmbeddingClient.pack_embedding(embeddings[0])
        except Exception as e:
            logger.warning("Embedding failed during store: %s", e)

    await db.execute(
        """INSERT INTO memories (agent_id, msg_id, content, source, timestamp, metadata, embedding)
           VALUES (?, ?, ?, ?, ?, ?, ?)""",
        (agent_id, msg_id, content, source, timestamp, metadata, embedding_blob),
    )
    await db.commit()
    return {"ok": True}


async def do_recall(agent_id: str, query: str, limit: int) -> dict:
    """Recall relevant memories using multi-strategy search."""
    db = await get_db()
    results: list[dict] = []
    seen_ids: set = set()

    # Strategy 0: Vector search (if embedding available and query non-empty)
    if _embedding_client and query.strip():
        vector_results = await _search_vector(db, agent_id, query, limit)
        for row in vector_results:
            rid = row.get("_rid", row["id"])
            if rid not in seen_ids:
                results.append(row)
                seen_ids.add(rid)

    # Strategy 1: FTS5 episode search (if enabled and query non-empty)
    if FTS_ENABLED and query.strip():
        fts_results = await _search_episodes_fts(db, agent_id, query, limit)
        for row in fts_results:
            rid = ("ep", row["id"])
            if rid not in seen_ids:
                results.append(row)
                seen_ids.add(rid)

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
            rid = ("mem", row["id"])
            if rid not in seen_ids:
                results.append(row)
                seen_ids.add(rid)

    # Truncate to limit and reverse for chronological order (oldest first for LLM)
    results = results[:limit]
    results.reverse()

    # Convert to ClotoMessage-compatible format
    messages = []
    for r in results:
        msg: dict = {"content": r["content"]}
        if r.get("source"):
            msg["source"] = r["source"] if isinstance(r["source"], dict) else _try_parse_json(r["source"])
        if r.get("timestamp"):
            msg["timestamp"] = r["timestamp"]
        if r.get("msg_id"):
            msg["id"] = r["msg_id"]
        # Remove internal tracking keys
        r.pop("_rid", None)
        messages.append(msg)

    return {"messages": messages}


async def _search_vector(
    db: aiosqlite.Connection, agent_id: str, query: str, limit: int
) -> list[dict]:
    """Search memories and episodes using vector cosine similarity."""
    import numpy as np

    # 1. Compute query embedding
    embeddings = await _embedding_client.embed([query])
    if not embeddings or not embeddings[0]:
        return []
    query_vec = np.array(embeddings[0], dtype=np.float32)
    query_dim = len(query_vec)

    candidates: list[tuple[float, dict]] = []

    # 2. Search memory embeddings
    rows = await db.execute_fetchall(
        """SELECT id, msg_id, content, source, timestamp, embedding
           FROM memories
           WHERE agent_id = ? AND embedding IS NOT NULL
           ORDER BY created_at DESC
           LIMIT ?""",
        (agent_id, MAX_MEMORIES),
    )

    for row in rows:
        mem_id, msg_id, content, source, timestamp, blob = row
        try:
            mem_vec = np.frombuffer(blob, dtype=np.float32)
            if len(mem_vec) != query_dim:
                continue  # Dimension mismatch (provider changed)
            sim = float(np.dot(query_vec, mem_vec))
            if sim >= VECTOR_MIN_SIMILARITY:
                candidates.append((sim, {
                    "id": mem_id,
                    "_rid": ("mem", mem_id),
                    "msg_id": msg_id,
                    "content": content,
                    "source": source,
                    "timestamp": timestamp,
                }))
        except Exception:
            continue

    # 3. Search episode embeddings
    ep_rows = await db.execute_fetchall(
        """SELECT id, summary, start_time, embedding
           FROM episodes
           WHERE agent_id = ? AND embedding IS NOT NULL
           ORDER BY created_at DESC
           LIMIT ?""",
        (agent_id, MAX_MEMORIES),
    )

    for row in ep_rows:
        ep_id, summary, start_time, blob = row
        try:
            ep_vec = np.frombuffer(blob, dtype=np.float32)
            if len(ep_vec) != query_dim:
                continue
            sim = float(np.dot(query_vec, ep_vec))
            if sim >= VECTOR_MIN_SIMILARITY:
                candidates.append((sim, {
                    "id": ep_id,
                    "_rid": ("ep", ep_id),
                    "content": f"[Episode] {summary}",
                    "source": {"System": "episode"},
                    "timestamp": start_time or "",
                }))
        except Exception:
            continue

    # 4. Sort by similarity descending, return top-K
    candidates.sort(key=lambda x: x[0], reverse=True)
    return [c[1] for c in candidates[:limit]]


async def _search_episodes_fts(
    db: aiosqlite.Connection, agent_id: str, query: str, limit: int
) -> list[dict]:
    """Search episodes using FTS5."""
    # Sanitize: strip everything except alphanumeric, CJK, and whitespace
    # to prevent FTS5 operator injection (AND/OR/NOT/NEAR/*/^/- etc.)
    sanitized = re.sub(r'[^\w\s]', "", query, flags=re.UNICODE)
    words = sanitized.split()
    if not words:
        return []

    # Each word quoted for phrase matching; quotes inside words already stripped
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
    """Simple concatenation summary + keyword extraction (no LLM)."""
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

    # Compute embedding for episode summary
    embedding_blob = None
    if _embedding_client and summary:
        try:
            embeddings = await _embedding_client.embed([summary])
            if embeddings and embeddings[0]:
                embedding_blob = EmbeddingClient.pack_embedding(embeddings[0])
        except Exception as e:
            logger.warning("Embedding failed for episode: %s", e)

    cursor = await db.execute(
        """INSERT INTO episodes (agent_id, summary, keywords, start_time, end_time, embedding)
           VALUES (?, ?, ?, ?, ?, ?)""",
        (agent_id, summary, keywords, start_time, end_time, embedding_blob),
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

server = Server("cloto-mcp-ks22")


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
                        "description": "ClotoMessage to store (id, content, source, timestamp, metadata)",
                    },
                },
                "required": ["agent_id", "message"],
            },
        ),
        Tool(
            name="recall",
            description="Recall relevant memories using multi-strategy search (vector + FTS5 + keyword).",
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
        Tool(
            name="list_memories",
            description="List recent memories for an agent (for dashboard display).",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier (empty for all agents)",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max memories to return",
                        "default": 100,
                    },
                },
                "required": [],
            },
        ),
        Tool(
            name="list_episodes",
            description="List archived episodes for an agent (for dashboard display).",
            inputSchema={
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "Agent identifier (empty for all agents)",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max episodes to return",
                        "default": 50,
                    },
                },
                "required": [],
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
        elif name == "list_memories":
            result = await do_list_memories(
                arguments.get("agent_id", ""),
                arguments.get("limit", 100),
            )
        elif name == "list_episodes":
            result = await do_list_episodes(
                arguments.get("agent_id", ""),
                arguments.get("limit", 50),
            )
        else:
            result = {"error": f"Unknown tool: {name}"}

        return [TextContent(type="text", text=json.dumps(result))]
    except Exception as e:
        return [
            TextContent(type="text", text=json.dumps({"error": str(e)}))
        ]


async def do_list_memories(agent_id: str, limit: int) -> dict:
    """List recent memories for dashboard display."""
    db = await get_db()
    if agent_id:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, msg_id, content, source, timestamp, created_at "
            "FROM memories WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
            (agent_id, min(limit, 500)),
        )
    else:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, msg_id, content, source, timestamp, created_at "
            "FROM memories ORDER BY created_at DESC LIMIT ?",
            (min(limit, 500),),
        )
    memories = []
    for row in rows:
        source = {}
        try:
            source = json.loads(row[4]) if row[4] else {}
        except (json.JSONDecodeError, TypeError):
            pass
        memories.append({
            "id": row[0],
            "agent_id": row[1],
            "content": row[3],
            "source": source,
            "timestamp": row[5],
            "created_at": row[6],
        })
    return {"memories": memories, "count": len(memories)}


async def do_list_episodes(agent_id: str, limit: int) -> dict:
    """List archived episodes for dashboard display."""
    db = await get_db()
    if agent_id:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, summary, keywords, start_time, end_time, created_at "
            "FROM episodes WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
            (agent_id, min(limit, 200)),
        )
    else:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, summary, keywords, start_time, end_time, created_at "
            "FROM episodes ORDER BY created_at DESC LIMIT ?",
            (min(limit, 200),),
        )
    episodes = []
    for row in rows:
        episodes.append({
            "id": row[0],
            "agent_id": row[1],
            "summary": row[2],
            "keywords": row[3],
            "start_time": row[4],
            "end_time": row[5],
            "created_at": row[6],
        })
    return {"episodes": episodes, "count": len(episodes)}


async def main():
    global _embedding_client

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )

    # Initialize embedding client
    if EMBEDDING_MODE != "none":
        _embedding_client = EmbeddingClient(
            mode=EMBEDDING_MODE,
            http_url=EMBEDDING_URL,
            api_key=EMBEDDING_API_KEY,
            api_url=EMBEDDING_API_URL,
            model=EMBEDDING_MODEL,
        )
        await _embedding_client.initialize()
        logger.info("Embedding client ready (mode=%s)", EMBEDDING_MODE)
    else:
        logger.info("Embedding disabled (mode=none), using FTS5 + keyword only")

    # Initialize DB on startup
    await get_db()
    try:
        async with stdio_server() as (read_stream, write_stream):
            await server.run(
                read_stream, write_stream, server.create_initialization_options()
            )
    finally:
        await close_db()
        if _embedding_client:
            await _embedding_client.close()


if __name__ == "__main__":
    asyncio.run(main())
