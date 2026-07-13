"""Chat bootstrap — prepare an *isolated* throwaway DB in which exactly one
LLM reasoning engine is live, so a real chat can be driven to success without
touching any real user state.

Why this is needed
------------------
An LLM provider is only usable when four things line up: an ``llm_providers``
row **with an api key**, a matching reasoning-engine row in ``mcp_servers``
(id == provider id), the agent's ``default_engine_id`` pointing at it, and a
``server_grant`` for the agent. The only source of a real key is the operator's
own dev DB. Booting a *copy* of that DB carries the key along (never read or
printed — the operator permits copy+use, not disclosure), but a naive copy also
re-activates **every** MCP engine, including ``cpersona`` / ``cscheduler`` /
``embedding`` whose subprocesses open real sqlite DBs — state pollution.

What this does
--------------
Copies the dev DB to a throwaway path, then on the copy:

* ``is_active = 0`` on every ``mcp_servers`` row except the kept engine(s) —
  the *single* switch (``db/mcp.rs`` ``WHERE is_active = 1``) that stops a
  subprocess from spawning at all (non-destructive: grants' ``ON DELETE
  CASCADE`` is not triggered, unlike a row delete).
* rewrites the kept engine's hard-coded LLM-proxy port (``127.0.0.1:8082`` in
  its ``env``) to a private port, so the harness can bind a per-run proxy and
  coexist with a running GUI / parallel runs.
* points the target agent's ``default_engine_id`` at the kept engine and keeps
  it + its provider enabled.

The api key column rides along inside ``.backup`` and is **never** selected,
logged, or otherwise surfaced.
"""

from __future__ import annotations

import json
import os
import shutil
import sqlite3
from typing import Iterable, List, Optional

# Engines safe to keep live for chat: pure-HTTP reasoning engines that POST to
# the in-process LLM proxy and open no local DB. ``deepseek`` is the only one
# seeded with both an engine row and a key in the dev DB.
DEFAULT_KEEP_ENGINES = ("deepseek",)
DEFAULT_AGENT = "agent.cloto_default"


# Real DBs the deactivated engines would open if they ever spawned — the
# isolation oracle asserts these stay byte-identical across a run. Only paths
# that exist are fingerprinted; the list is deliberately broad (defence in
# depth — if a prune ever regresses, the oracle catches the mutation).
def default_protected_paths(repo_root: str) -> List[str]:
    home = os.path.expanduser("~")
    candidates = [
        os.path.join(repo_root, "target/debug/data/cpersona.db"),
        os.path.join(repo_root, "target/debug/data/cscheduler.db"),
        os.path.join(repo_root, "target/release/data/cpersona.db"),
        os.path.join(repo_root, "target/release/data/cscheduler.db"),
        os.path.join(home, ".claude/cpersona.db"),
        os.path.join(home, ".claude/cscheduler.db"),
    ]
    return [p for p in candidates if os.path.exists(p)]


def copy_seal_key(seed_db: str, sandbox_parent: str) -> Optional[str]:
    """Copy the install's ``seal.key`` (sits next to the dev DB) into the
    throwaway sandbox parent so kept engines' Magic Seals verify against the
    *real* HMAC key instead of a freshly-generated one.

    ``load_or_generate_seal_key`` resolves ``{sandbox_base_dir.parent}/seal.key``
    (mgp-seal/src/key.rs step 2). Pure file copy — the key bytes are never read
    or logged. Returns the destination path, or None if no seal.key exists.
    """
    src = os.path.join(os.path.dirname(os.path.abspath(seed_db)), "seal.key")
    if not os.path.exists(src):
        return None
    dst = os.path.join(sandbox_parent, "seal.key")
    shutil.copyfile(src, dst)
    return dst


def _rewrite_proxy_port(env_json: str, proxy_port: int) -> str:
    """Rewrite any ``127.0.0.1:8082`` / ``localhost:8082`` in an engine env
    JSON blob to the chosen private proxy port. Returns the (possibly
    unchanged) JSON string. The env holds no secret (provider id + proxy URL
    only)."""
    try:
        env = json.loads(env_json) if env_json else {}
    except json.JSONDecodeError:
        return env_json
    if not isinstance(env, dict):
        return env_json
    changed = False
    for k, v in list(env.items()):
        if isinstance(v, str):
            for host in ("127.0.0.1", "localhost"):
                needle = f"{host}:8082"
                repl = f"{host}:{proxy_port}"
                if needle in v:
                    env[k] = v.replace(needle, repl)
                    changed = True
    return json.dumps(env) if changed else env_json


def prepare_chat_db(
    src_db: str,
    dst_db: str,
    proxy_port: int,
    keep_engines: Iterable[str] = DEFAULT_KEEP_ENGINES,
    agent_id: str = DEFAULT_AGENT,
) -> str:
    """Copy ``src_db`` → ``dst_db`` and rewire the copy for isolated chat.

    Returns the kept engine id the agent was pointed at. Never reads an api
    key value.
    """
    keep = tuple(keep_engines)
    if not keep:
        raise ValueError("keep_engines must be non-empty")

    # 1. consistent copy (read-only source open; keys ride along untouched).
    src = sqlite3.connect(f"file:{src_db}?mode=ro", uri=True)
    try:
        dst = sqlite3.connect(dst_db)
        try:
            src.backup(dst)
        finally:
            dst.close()
    finally:
        src.close()

    # 2. rewire the copy only.
    conn = sqlite3.connect(dst_db)
    try:
        placeholders = ",".join("?" for _ in keep)
        cur = conn.cursor()

        # deactivate every engine except the kept one(s) → only they spawn.
        cur.execute(
            f"UPDATE mcp_servers SET is_active = 0 WHERE name NOT IN ({placeholders})",
            keep,
        )

        # rewrite hard-coded proxy port in kept engines' env.
        cur.execute(
            f"SELECT name, env FROM mcp_servers WHERE name IN ({placeholders})",
            keep,
        )
        for name, env_json in cur.fetchall():
            new_env = _rewrite_proxy_port(env_json or "{}", proxy_port)
            if new_env != (env_json or "{}"):
                cur.execute(
                    "UPDATE mcp_servers SET env = ? WHERE name = ?",
                    (new_env, name),
                )

        # point the agent at the primary kept engine + keep everything enabled.
        primary = keep[0]
        cur.execute(
            "UPDATE agents SET default_engine_id = ?, enabled = 1 WHERE id = ?",
            (primary, agent_id),
        )
        # Disable every *other* agent: the 30s heartbeat pings all enabled
        # agents, which would fire stray (real, billable) LLM calls and pollute
        # the event history with unrelated ThoughtResponses.
        cur.execute(
            "UPDATE agents SET enabled = 0 WHERE id != ?",
            (agent_id,),
        )
        cur.execute(
            f"UPDATE llm_providers SET enabled = 1 WHERE id IN ({placeholders})",
            keep,
        )
        conn.commit()
        return primary
    finally:
        conn.close()
