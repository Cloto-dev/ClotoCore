"""
Cloto MCP Server: CPersona Memory
Persistent memory with FTS5 full-text search and pluggable vector embedding.
Evolved from CPersona 2.2 (Rust plugin) with 2.1 (ai_karin) architecture enhancements.

Phase 1: store, recall (FTS5 + keyword) — COMPLETE
Phase 2: Vector embedding integration (cosine similarity search) — COMPLETE
Phase 3: LLM-powered memory extraction (profile + episode summarization) — COMPLETE
Phase 4: Anti-contamination — memory boundary markers, timestamp annotations,
         anti-hallucination guardrails — COMPLETE
Phase 5: Background task queue (DB-persisted, crash-recoverable) — COMPLETE
"""

import asyncio
import json
import logging
import os
import re
import struct
import hashlib
import time
from collections import OrderedDict
from datetime import datetime, timezone

import sys

import aiosqlite
import httpx
from mcp.server.stdio import stdio_server

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry

logger = logging.getLogger(__name__)


def _clamp_limit(limit: int, cap: int) -> int:
    """Clamp a user-supplied limit to [0, cap], preventing negative bypass."""
    return min(max(0, limit), cap)


# ============================================================
# Configuration
# ============================================================

DB_PATH = os.environ.get("CPERSONA_DB_PATH", "data/cpersona.db")
MAX_MEMORIES = int(os.environ.get("CPERSONA_MAX_MEMORIES", "500"))
FTS_ENABLED = os.environ.get("CPERSONA_FTS_ENABLED", "true").lower() == "true"

# Embedding configuration
EMBEDDING_MODE = os.environ.get("CPERSONA_EMBEDDING_MODE", "none")
EMBEDDING_URL = os.environ.get("CPERSONA_EMBEDDING_URL", "")
EMBEDDING_API_KEY = os.environ.get("CPERSONA_EMBEDDING_API_KEY", "")
EMBEDDING_API_URL = os.environ.get(
    "CPERSONA_EMBEDDING_API_URL", "https://api.openai.com/v1/embeddings"
)
EMBEDDING_MODEL = os.environ.get("CPERSONA_EMBEDDING_MODEL", "text-embedding-3-small")

# Vector search threshold (cosine similarity, 0.0-1.0)
VECTOR_MIN_SIMILARITY = float(os.environ.get("CPERSONA_VECTOR_MIN_SIMILARITY", "0.3"))

# Embedding cache (query deduplication)
EMBEDDING_CACHE_SIZE = int(os.environ.get("CPERSONA_EMBEDDING_CACHE_SIZE", "256"))
EMBEDDING_CACHE_TTL = int(os.environ.get("CPERSONA_EMBEDDING_CACHE_TTL", "300"))  # seconds

# LLM proxy configuration (for Phase 3: memory extraction)
LLM_PROXY_URL = os.environ.get(
    "CPERSONA_LLM_PROXY_URL", "http://127.0.0.1:8082/v1/chat/completions"
)
LLM_PROVIDER = os.environ.get("CPERSONA_LLM_PROVIDER", "cerebras")
LLM_MODEL = os.environ.get("CPERSONA_LLM_MODEL", "gpt-oss-120b")

# Background task queue (Phase 5: crash-recoverable async processing)
TASK_QUEUE_ENABLED = os.environ.get("CPERSONA_TASK_QUEUE_ENABLED", "true").lower() == "true"
TASK_MAX_RETRIES = int(os.environ.get("CPERSONA_TASK_MAX_RETRIES", "3"))
TASK_RETRY_DELAY = int(os.environ.get("CPERSONA_TASK_RETRY_DELAY", "30"))  # seconds

# ============================================================
# Embedding Client
# ============================================================


class EmbeddingClient:
    """Client for computing vector embeddings via HTTP or API.

    Includes a TTL-based LRU cache for single-text queries (recall dedup).
    """

    def __init__(
        self,
        mode: str,
        http_url: str = "",
        api_key: str = "",
        api_url: str = "",
        model: str = "",
        cache_size: int = EMBEDDING_CACHE_SIZE,
        cache_ttl: int = EMBEDDING_CACHE_TTL,
    ):
        self.mode = mode
        self._http_url = http_url
        self._api_key = api_key
        self._api_url = api_url
        self._model = model
        self._client = None
        # LRU cache: key=text_hash, value=(embedding, timestamp)
        self._cache: OrderedDict[str, tuple[list[float], float]] = OrderedDict()
        self._cache_size = cache_size
        self._cache_ttl = cache_ttl
        self.cache_hits = 0
        self.cache_misses = 0

    async def initialize(self):
        """Create persistent HTTP client."""
        import httpx

        self._client = httpx.AsyncClient(timeout=30)
        logger.info(
            "EmbeddingClient initialized (mode=%s, cache=%d, ttl=%ds)",
            self.mode, self._cache_size, self._cache_ttl,
        )

    async def close(self):
        """Close HTTP client."""
        if self._client:
            await self._client.aclose()
            self._client = None

    def _cache_key(self, text: str) -> str:
        return hashlib.sha256(text.encode()).hexdigest()[:16]

    def _cache_get(self, text: str) -> list[float] | None:
        """Look up a single text in cache. Returns embedding or None."""
        key = self._cache_key(text)
        entry = self._cache.get(key)
        if entry is None:
            return None
        embedding, ts = entry
        if time.monotonic() - ts > self._cache_ttl:
            del self._cache[key]
            return None
        # Move to end (most recently used)
        self._cache.move_to_end(key)
        return embedding

    def _cache_put(self, text: str, embedding: list[float]) -> None:
        """Store a single text→embedding in cache."""
        key = self._cache_key(text)
        self._cache[key] = (embedding, time.monotonic())
        self._cache.move_to_end(key)
        while len(self._cache) > self._cache_size:
            self._cache.popitem(last=False)

    async def embed(self, texts: list[str]) -> list[list[float]] | None:
        """Compute embeddings with LRU cache for single-text queries.

        Cache is used only for single-text calls (the common recall path).
        Batch calls bypass cache to avoid complexity.
        """
        if self.mode == "none" or not self._client:
            return None

        # Single-text cache path
        if len(texts) == 1:
            cached = self._cache_get(texts[0])
            if cached is not None:
                self.cache_hits += 1
                return [cached]
            self.cache_misses += 1

        try:
            if self.mode == "http":
                result = await self._embed_via_http(texts)
            elif self.mode == "api":
                result = await self._embed_via_api(texts)
            else:
                logger.warning("Unknown embedding mode: %s", self.mode)
                return None
        except (httpx.RequestError, httpx.HTTPStatusError, ValueError, KeyError) as e:
            logger.warning("Embedding request failed: %s", e)
            return None

        # Cache single-text results
        if result and len(texts) == 1 and len(result) == 1:
            self._cache_put(texts[0], result[0])

        return result

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
# LLM Proxy (Phase 3: Memory Extraction)
# ============================================================


async def call_llm_proxy(prompt: str, system: str = "You are a memory extraction assistant.") -> str | None:
    """Call the kernel LLM proxy for memory extraction tasks.

    Returns the LLM response text, or None on failure (graceful degradation).
    """
    try:
        async with httpx.AsyncClient(timeout=60) as client:
            resp = await client.post(
                LLM_PROXY_URL,
                json={
                    "model": LLM_MODEL,
                    "messages": [
                        {"role": "system", "content": system},
                        {"role": "user", "content": prompt},
                    ],
                    "stream": False,
                },
                headers={
                    "X-LLM-Provider": LLM_PROVIDER,
                    "Content-Type": "application/json",
                },
            )
            resp.raise_for_status()
            return resp.json()["choices"][0]["message"]["content"]
    except Exception as e:
        logger.warning("LLM proxy call failed: %s", e)
        return None


def _format_history(history: list[dict]) -> str:
    """Format conversation history into readable text for LLM prompts."""
    lines = []
    for msg in history:
        content = msg.get("content", "")
        if not content:
            continue
        source = msg.get("source", {})
        if isinstance(source, str):
            try:
                source = json.loads(source)
            except (json.JSONDecodeError, TypeError):
                source = {}
        if isinstance(source, dict) and ("User" in source or "user" in source):
            lines.append(f"[User] {content}")
        else:
            lines.append(f"[Agent] {content}")
    return "\n".join(lines)


# ============================================================
# Background Task Queue (Phase 5)
# ============================================================


class MemoryTaskQueue:
    """DB-persisted background task queue with crash recovery.

    Tasks (update_profile, archive_episode) are serialized to SQLite on enqueue,
    processed asynchronously in FIFO order, and deleted on success.
    On startup, any pending tasks from a previous crash are automatically recovered.

    Ported from KS2.1 (ai_karin) MemoryWorker — adapted from Rust/tokio to Python/asyncio.
    """

    def __init__(self):
        self._event = asyncio.Event()
        self._running = False
        self._task: asyncio.Task | None = None

    async def start(self):
        """Start the background processing loop."""
        self._running = True
        self._task = asyncio.create_task(self._loop())
        # Signal to process any pending tasks from before crash
        self._event.set()
        logger.info("MemoryTaskQueue: started (max_retries=%d, retry_delay=%ds)", TASK_MAX_RETRIES, TASK_RETRY_DELAY)

    async def stop(self):
        """Stop the background loop gracefully."""
        self._running = False
        self._event.set()  # Wake up the loop so it can exit
        if self._task:
            try:
                await asyncio.wait_for(self._task, timeout=5.0)
            except asyncio.TimeoutError:
                self._task.cancel()
                logger.warning("MemoryTaskQueue: forced shutdown after timeout")

    async def enqueue(self, task_type: str, agent_id: str, payload: list[dict]) -> int:
        """Enqueue a task. Returns task ID."""
        db = await get_db()
        cursor = await db.execute(
            "INSERT INTO pending_memory_tasks (task_type, agent_id, payload) VALUES (?, ?, ?)",
            (task_type, agent_id, json.dumps(payload)),
        )
        await db.commit()
        task_id = cursor.lastrowid
        logger.info("MemoryTaskQueue: enqueued %s for agent %s (task_id=%d)", task_type, agent_id, task_id)
        self._event.set()
        return task_id

    async def get_status(self) -> dict:
        """Get queue status for monitoring."""
        db = await get_db()
        rows = await db.execute_fetchall("SELECT COUNT(*) FROM pending_memory_tasks")
        pending = rows[0][0] if rows else 0
        return {
            "enabled": True,
            "pending": pending,
            "max_retries": TASK_MAX_RETRIES,
            "retry_delay": TASK_RETRY_DELAY,
        }

    async def _loop(self):
        """Main processing loop — waits for signal, drains all pending tasks."""
        while self._running:
            await self._event.wait()
            self._event.clear()

            while self._running:
                task = await self._fetch_next()
                if task is None:
                    break

                task_id, task_type, agent_id, payload, retries = task
                logger.info(
                    "MemoryTaskQueue: processing %s (task_id=%d, agent=%s, retry=%d/%d)",
                    task_type, task_id, agent_id, retries, TASK_MAX_RETRIES,
                )
                try:
                    if task_type == "update_profile":
                        await do_update_profile(agent_id, payload)
                    elif task_type == "archive_episode":
                        await do_archive_episode(agent_id, payload)
                    else:
                        logger.error("MemoryTaskQueue: unknown task type %s, discarding", task_type)

                    # Task succeeded — delete it
                    await self._delete_task(task_id)
                    logger.info("MemoryTaskQueue: completed %s (task_id=%d)", task_type, task_id)
                except Exception as e:
                    logger.error("MemoryTaskQueue: task %d (%s) failed: %s", task_id, task_type, e)
                    if retries + 1 >= TASK_MAX_RETRIES:
                        logger.error("MemoryTaskQueue: task %d exceeded max retries, discarding", task_id)
                        await self._delete_task(task_id)
                    else:
                        await self._increment_retry(task_id)
                        # Back off before processing the next task
                        await asyncio.sleep(TASK_RETRY_DELAY)

    async def _fetch_next(self) -> tuple | None:
        db = await get_db()
        rows = await db.execute_fetchall(
            "SELECT id, task_type, agent_id, payload, retries "
            "FROM pending_memory_tasks ORDER BY id ASC LIMIT 1"
        )
        if not rows:
            return None
        task_id, task_type, agent_id, payload_json, retries = rows[0]
        payload = json.loads(payload_json)
        return (task_id, task_type, agent_id, payload, retries)

    async def _delete_task(self, task_id: int):
        db = await get_db()
        await db.execute("DELETE FROM pending_memory_tasks WHERE id = ?", (task_id,))
        await db.commit()

    async def _increment_retry(self, task_id: int):
        db = await get_db()
        await db.execute(
            "UPDATE pending_memory_tasks SET retries = retries + 1 WHERE id = ?",
            (task_id,),
        )
        await db.commit()


_task_queue: MemoryTaskQueue | None = None


# ============================================================
# Database
# ============================================================

SCHEMA_VERSION = 2

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

CREATE TABLE IF NOT EXISTS pending_memory_tasks (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    task_type  TEXT NOT NULL,
    agent_id   TEXT NOT NULL,
    payload    TEXT NOT NULL,
    retries    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
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
    """Generate a unique key for a memory entry (2.1-compatible)."""
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
        except (httpx.RequestError, httpx.HTTPStatusError, ValueError, TypeError) as e:
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

    # Strategy 3: Keyword match on memories (2.2 fallback)
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

    # Convert to ClotoMessage-compatible format with timestamp annotations
    messages = []
    for r in results:
        content = r["content"]
        # Annotate with localized timestamp so LLM knows when this memory is from
        ts_raw = r.get("timestamp", "")
        if ts_raw:
            annotation = _format_memory_timestamp(ts_raw)
            if annotation:
                content = f"[Memory from {annotation}] {content}"

        msg: dict = {"content": content}
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
        except (ValueError, TypeError) as e:
            logger.debug("Skipping memory %s: vector decode error: %s", mem_id, e)
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
        except (ValueError, TypeError) as e:
            logger.debug("Skipping episode %s: vector decode error: %s", ep_id, e)
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
    """Search memories using keyword matching (2.2-compatible fallback)."""
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
    """Extract user facts from conversation via LLM and merge into profile."""
    db = await get_db()

    if not history:
        return {"ok": True, "profiles_updated": 0}

    conversation = _format_history(history)
    if not conversation.strip():
        return {"ok": True, "profiles_updated": 0}

    # Fetch existing profile
    rows = await db.execute_fetchall(
        "SELECT content FROM profiles WHERE agent_id = ? AND user_id = '' LIMIT 1",
        (agent_id,),
    )
    existing = rows[0][0] if rows else ""

    # LLM-driven fact extraction and merge
    prompt = (
        "Extract facts about the user from the following conversation.\n"
        "Output a concise profile in bullet-point format.\n"
        "MERGE with existing facts — keep all existing information unless explicitly contradicted.\n\n"
        f"Existing profile:\n{existing or '(none)'}\n\n"
        f"Conversation:\n{conversation}"
    )
    result = await call_llm_proxy(prompt)

    if result is None:
        # LLM unavailable — fallback to simple concatenation
        user_lines = []
        for msg in history:
            source = msg.get("source", {})
            if isinstance(source, str):
                source = _try_parse_json(source)
            if isinstance(source, dict) and ("User" in source or "user" in source):
                content = msg.get("content", "")
                if content:
                    user_lines.append(content)
        if not user_lines:
            return {"ok": True, "profiles_updated": 0}
        result = "\n".join(user_lines[-10:])

    await db.execute(
        """INSERT INTO profiles (agent_id, user_id, content, updated_at)
           VALUES (?, '', ?, datetime('now'))
           ON CONFLICT(agent_id, user_id) DO UPDATE SET
               content = excluded.content,
               updated_at = excluded.updated_at""",
        (agent_id, result),
    )
    await db.commit()
    return {"ok": True, "profiles_updated": 1}


async def do_archive_episode(agent_id: str, history: list[dict]) -> dict:
    """Summarize conversation via LLM and archive as episode with keywords + embedding."""
    db = await get_db()

    if not history:
        return {"ok": True, "episode_id": None}

    conversation = _format_history(history)
    if not conversation.strip():
        return {"ok": True, "episode_id": None}

    # LLM-driven summarization
    summary_prompt = (
        "Summarize the following conversation concisely (800-1200 characters).\n"
        "Preserve proper nouns, dates, decisions, and key technical details.\n\n"
        f"{conversation}"
    )
    summary = await call_llm_proxy(summary_prompt)

    if summary is None:
        # LLM unavailable — fallback to simple concatenation
        lines = []
        for msg in history:
            content = msg.get("content", "")
            if content:
                source = msg.get("source", {})
                if isinstance(source, str):
                    source = _try_parse_json(source)
                speaker = "User" if isinstance(source, dict) and ("User" in source or "user" in source) else "Agent"
                lines.append(f"[{speaker}] {content}")
        if len(lines) <= 5:
            summary = "\n".join(lines)
        else:
            summary = "\n".join(lines[:2] + [f"... ({len(lines) - 4} messages) ..."] + lines[-2:])

    # LLM-driven keyword extraction
    keyword_prompt = (
        "Extract 5-10 search keywords from the following summary.\n"
        "Choose words suitable for full-text search (FTS5). Output space-separated keywords only.\n\n"
        f"{summary}"
    )
    keywords_result = await call_llm_proxy(keyword_prompt)

    if keywords_result is None:
        # Fallback: word frequency
        word_freq: dict[str, int] = {}
        for msg in history:
            for word in re.findall(r'\b\w{3,}\b', msg.get("content", "").lower()):
                word_freq[word] = word_freq.get(word, 0) + 1
        stopwords = {"the", "and", "for", "that", "this", "with", "are", "was", "has", "have",
                     "not", "but", "you", "your", "can", "will", "from", "they", "been", "more"}
        sorted_words = sorted(
            ((w, c) for w, c in word_freq.items() if w not in stopwords),
            key=lambda x: x[1], reverse=True,
        )
        keywords = " ".join(w for w, _ in sorted_words[:10])
    else:
        keywords = keywords_result.strip()

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


def _format_memory_timestamp(ts_raw: str) -> str | None:
    """Convert an ISO-8601 timestamp to a human-readable local time annotation.

    Uses the OS-local timezone (no hardcoded TZ). Returns None on parse failure.
    """
    if not ts_raw:
        return None
    try:
        # Parse ISO-8601 (handles +00:00, Z, and offset formats)
        dt = datetime.fromisoformat(ts_raw.replace("Z", "+00:00"))
        # Convert to OS-local timezone
        local_dt = dt.astimezone()
        tz_name = local_dt.strftime("%Z")  # e.g. "JST", "EST", "CET"
        return local_dt.strftime(f"%Y-%m-%d %H:%M {tz_name}")
    except (ValueError, OSError):
        return None


def _try_parse_json(s: str) -> dict:
    """Try to parse a string as JSON, return empty dict on failure."""
    try:
        return json.loads(s)
    except (json.JSONDecodeError, TypeError):
        return {}


# ============================================================
# MCP Server
# ============================================================

registry = ToolRegistry("cloto-mcp-cpersona")


# Tool registrations are below all do_* functions (auto_tool evaluates at definition time)


async def do_list_memories(agent_id: str, limit: int) -> dict:
    """List recent memories for dashboard display."""
    db = await get_db()
    if agent_id:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, msg_id, content, source, timestamp, created_at "
            "FROM memories WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
            (agent_id, _clamp_limit(limit, 500)),
        )
    else:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, msg_id, content, source, timestamp, created_at "
            "FROM memories ORDER BY created_at DESC LIMIT ?",
            (_clamp_limit(limit, 500),),
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
            (agent_id, _clamp_limit(limit, 200)),
        )
    else:
        rows = await db.execute_fetchall(
            "SELECT id, agent_id, summary, keywords, start_time, end_time, created_at "
            "FROM episodes ORDER BY created_at DESC LIMIT ?",
            (_clamp_limit(limit, 200),),
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


async def do_delete_memory(memory_id: int, agent_id: str = "") -> dict:
    """Delete a single memory by ID.

    When agent_id is provided (non-empty), enforces ownership: only deletes
    if the memory belongs to that agent. When empty (dashboard/admin calls),
    deletes unconditionally.
    """
    db = await get_db()
    if agent_id:
        cursor = await db.execute(
            "DELETE FROM memories WHERE id = ? AND agent_id = ?",
            (memory_id, agent_id),
        )
    else:
        cursor = await db.execute("DELETE FROM memories WHERE id = ?", (memory_id,))
    await db.commit()
    if cursor.rowcount == 0:
        return {"error": f"Memory {memory_id} not found or not owned by agent"}
    return {"ok": True, "deleted_id": memory_id}


async def do_delete_agent_data(agent_id: str) -> dict:
    """Delete ALL data for a specific agent (memories, profiles, episodes).

    Called by the kernel during agent deletion to prevent orphaned data.
    Requires a non-empty agent_id to prevent accidental full-table wipe.
    """
    if not agent_id:
        return {"error": "agent_id is required for bulk deletion"}

    db = await get_db()
    mem_cursor = await db.execute(
        "DELETE FROM memories WHERE agent_id = ?", (agent_id,)
    )
    prof_cursor = await db.execute(
        "DELETE FROM profiles WHERE agent_id = ?", (agent_id,)
    )
    ep_cursor = await db.execute(
        "DELETE FROM episodes WHERE agent_id = ?", (agent_id,)
    )
    await db.commit()

    result = {
        "ok": True,
        "agent_id": agent_id,
        "deleted_memories": mem_cursor.rowcount,
        "deleted_profiles": prof_cursor.rowcount,
        "deleted_episodes": ep_cursor.rowcount,
    }
    logger.info(
        "Deleted agent data for %s: %d memories, %d profiles, %d episodes",
        agent_id,
        mem_cursor.rowcount,
        prof_cursor.rowcount,
        ep_cursor.rowcount,
    )
    return result


async def do_delete_episode(episode_id: int, agent_id: str = "") -> dict:
    """Delete a single episode by ID (FTS5 triggers handle index cleanup).

    When agent_id is provided (non-empty), enforces ownership.
    """
    db = await get_db()
    if agent_id:
        cursor = await db.execute(
            "DELETE FROM episodes WHERE id = ? AND agent_id = ?",
            (episode_id, agent_id),
        )
    else:
        cursor = await db.execute("DELETE FROM episodes WHERE id = ?", (episode_id,))
    await db.commit()
    if cursor.rowcount == 0:
        return {"error": f"Episode {episode_id} not found or not owned by agent"}
    return {"ok": True, "deleted_id": episode_id}


# --- Tool registrations (must be after all do_* definitions) ---

registry.auto_tool("store", "Store a message in agent memory for future recall.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier"},
        "message": {"type": "object", "description": "ClotoMessage to store (id, content, source, timestamp, metadata)"},
    },
    "required": ["agent_id", "message"],
}, do_store, [("agent_id", str), ("message", dict)])

registry.auto_tool("recall", "Recall relevant memories using multi-strategy search (vector + FTS5 + keyword).", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier"},
        "query": {"type": "string", "description": "Search query (empty returns recent memories)"},
        "limit": {"type": "integer", "description": "Max memories to return", "default": 10},
    },
    "required": ["agent_id", "query"],
}, do_recall, [("agent_id", str), ("query", str), ("limit", int, 10)])

async def do_update_profile_or_queue(agent_id: str, history: list) -> dict:
    """Enqueue profile update if task queue is enabled, otherwise run synchronously."""
    if _task_queue and TASK_QUEUE_ENABLED:
        task_id = await _task_queue.enqueue("update_profile", agent_id, history)
        return {"ok": True, "queued": True, "task_id": task_id}
    return await do_update_profile(agent_id, history)


async def do_archive_episode_or_queue(agent_id: str, history: list) -> dict:
    """Enqueue episode archival if task queue is enabled, otherwise run synchronously."""
    if _task_queue and TASK_QUEUE_ENABLED:
        task_id = await _task_queue.enqueue("archive_episode", agent_id, history)
        return {"ok": True, "queued": True, "task_id": task_id}
    return await do_archive_episode(agent_id, history)


async def do_get_queue_status() -> dict:
    """Get the status of the background task queue."""
    if _task_queue and TASK_QUEUE_ENABLED:
        return await _task_queue.get_status()
    return {"enabled": False, "pending": 0}


registry.auto_tool("update_profile", "Extract user facts from conversation and merge with existing profile.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier"},
        "history": {"type": "array", "description": "Recent conversation messages", "items": {"type": "object"}},
    },
    "required": ["agent_id", "history"],
}, do_update_profile_or_queue, [("agent_id", str), ("history", list)])

registry.auto_tool("archive_episode", "Summarize and archive a conversation episode for searchable recall.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier"},
        "history": {"type": "array", "description": "Conversation messages to archive", "items": {"type": "object"}},
    },
    "required": ["agent_id", "history"],
}, do_archive_episode_or_queue, [("agent_id", str), ("history", list)])

registry.auto_tool("list_memories", "List recent memories for an agent (for dashboard display).", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier (empty for all agents)"},
        "limit": {"type": "integer", "description": "Max memories to return", "default": 100},
    },
    "required": [],
}, do_list_memories, [("agent_id", str), ("limit", int, 100)])

registry.auto_tool("list_episodes", "List archived episodes for an agent (for dashboard display).", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent identifier (empty for all agents)"},
        "limit": {"type": "integer", "description": "Max episodes to return", "default": 50},
    },
    "required": [],
}, do_list_episodes, [("agent_id", str), ("limit", int, 50)])

registry.auto_tool("delete_agent_data", "Delete ALL data (memories, profiles, episodes) for a specific agent. Used by kernel during agent deletion.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent ID whose data should be purged"},
    },
    "required": ["agent_id"],
}, do_delete_agent_data, [("agent_id", str)])

registry.auto_tool("delete_memory", "Delete a single memory by ID. Ownership is enforced when agent_id is provided.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent ID for ownership verification (injected by kernel)"},
        "memory_id": {"type": "integer", "description": "Memory ID to delete"},
    },
    "required": ["memory_id"],
}, do_delete_memory, [("memory_id", int), ("agent_id", str)])

registry.auto_tool("delete_episode", "Delete a single episode by ID. Ownership is enforced when agent_id is provided.", {
    "type": "object",
    "properties": {
        "agent_id": {"type": "string", "description": "Agent ID for ownership verification (injected by kernel)"},
        "episode_id": {"type": "integer", "description": "Episode ID to delete"},
    },
    "required": ["episode_id"],
}, do_delete_episode, [("episode_id", int), ("agent_id", str)])

registry.auto_tool("get_queue_status", "Get the status of the background task queue (pending tasks, retry config).", {
    "type": "object",
    "properties": {},
}, do_get_queue_status, [])


async def main():
    global _embedding_client, _task_queue

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

    # Start background task queue (Phase 5)
    if TASK_QUEUE_ENABLED:
        _task_queue = MemoryTaskQueue()
        await _task_queue.start()
    else:
        logger.info("Task queue disabled")

    try:
        async with stdio_server() as (read_stream, write_stream):
            await registry.server.run(
                read_stream, write_stream, registry.server.create_initialization_options()
            )
    finally:
        if _task_queue:
            await _task_queue.stop()
        await close_db()
        if _embedding_client:
            await _embedding_client.close()


if __name__ == "__main__":
    asyncio.run(main())
