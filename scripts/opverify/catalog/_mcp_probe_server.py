"""Hermetic stdio MCP server used by the ``mcp.lifecycle`` and
``permissions.decide`` operations.

It is registered against a running kernel via ``POST /api/mcp/servers`` with
``command="python3"``; the kernel resolves that bare name to its own
mcp-equipped venv (``resolve_python_command`` →
``data/mcp-servers/.venv/bin/python3``), so this file only needs the ``mcp``
SDK that venv already carries — no system-python dependency.

It exposes a single trivial tool, ``ping`` → ``"pong"``, enough to prove the
full register → connect → tool-discovery → call → stop → reap path. Speaks the
modern low-level ``Server`` API (3-arg ``app.run`` with initialization
options), which matches the SDK version the kernel venv pins.

With ``--declare-perms a,b`` it additionally advertises MGP server
capabilities (``capabilities.experimental.mgp`` — the Python-SDK-compatible
path the kernel reads) declaring the ``permissions`` extension and the given
required permissions. That drives the kernel's MGP Permission Flow (§3): the
kernel opens one pending permission request per permission and then *refuses
the connection* until they are approved — exactly the state the
``permissions.decide`` op needs to exercise the approve/deny mutations. The
declared MGP ``version`` matches the kernel's ``MGP_VERSION`` so negotiation
succeeds.

With ``--memory`` it additionally advertises the memory-capability tool set
(``list_memories`` / ``import_memories`` / ``update_memory`` /
``delete_memory`` / ``list_episodes`` / ``delete_episode`` /
``get_recall_precision`` / ``set_recall_precision``). The kernel's
``classify_tool`` maps those names onto ``CapabilityType::Memory`` purely by
tool name, so registering this server makes it *the* memory server for the
run — which is the only way the memory REST routes can be driven at all: every
one of them dispatches through the Memory capability and answers 400 or an
empty fallback when nothing provides it.

The store is a real in-process store, not a stub that always says yes: writes
are visible to the next read, a delete removes the row, and an unknown id
comes back ``{"ok": false}``. That is what makes ``assert_success`` able to
prove the kernel's mutation actually landed rather than that a 200 came back.

The memory tools are advertised through the MGP ``tools_for_capability``
manifest (Pattern-C), not left to the kernel's name-based ``classify_tool``
heuristic, because the heuristic does not know ``import_memories``: it is
absent from that match arm, so ``call_capability_tool(Memory,
"import_memories")`` finds no route and ``POST /api/memories/import`` answers
500 for *any* server that relies on name classification. Declaring the mapping
is the documented path a real memory server takes, and it is the only way this
route is reachable at all.

``lock_memory`` / ``unlock_memory`` are advertised **deliberately not at
all**. The kernel treats them as optional and falls back to its own
``memory_locks`` table when the server lacks them (``handlers::lock_memory``),
and that fallback is where the kernel-owned invariant lives — a locked memory
must be refused a delete. Advertising the tools would move both the lock and
the refusal into this fixture, where a bug in the kernel could not be seen.
"""

import asyncio
import json
import os
import sys

# Running this file as a script puts its own directory (``catalog/``) on
# ``sys.path[0]``, where the sibling ``mcp.py`` (the opverify MCP domain
# module) would shadow the real ``mcp`` SDK. Scrub our own dir first so the
# venv's ``mcp`` package resolves, not the sibling module.
_self_dir = os.path.dirname(os.path.abspath(__file__))
sys.path[:] = [p for p in sys.path if os.path.abspath(p or ".") != _self_dir]

import mcp.types as types  # noqa: E402
from mcp.server import Server  # noqa: E402
from mcp.server.stdio import stdio_server  # noqa: E402

# Kept in sync with mcp_mgp::MGP_VERSION; negotiate() requires semver
# compatibility with the kernel's client version for the extension to activate.
_MGP_VERSION = "0.6.0"

app = Server("opverify-lifecycle-probe")


def _declared_perms(argv):
    for i, a in enumerate(argv):
        if a == "--declare-perms" and i + 1 < len(argv):
            return [p for p in argv[i + 1].split(",") if p]
    return []


# ---------------------------------------------------------------- memory mode
# An in-process memory store. Ids are ints because the kernel's memory routes
# bind `Path<i64>` (`/api/memories/{id}`), so anything else 400s at the router
# before a handler is reached.
_MEMORIES = {}
_EPISODES = {}
_PRECISION = {}
_NEXT_ID = [1]

# Seeded so `list_episodes` / `delete_episode` have a real target on a fresh
# instance; memories start empty so `import_memories` is what creates them.
_SEED_EPISODES = 2


def _memory_mode(argv):
    return "--memory" in argv


def _seed_episodes():
    for _ in range(_SEED_EPISODES):
        eid = _NEXT_ID[0]
        _NEXT_ID[0] += 1
        _EPISODES[eid] = {
            "id": eid,
            "agent_id": "agent.cloto_default",
            "summary": f"opverify seeded episode {eid}",
            "created_at": eid,
        }


_MEMORY_TOOLS = [
    ("list_memories", "List stored memories, newest first."),
    ("import_memories", "Import memories from a JSONL file path."),
    ("update_memory", "Replace a memory's content."),
    ("delete_memory", "Delete a memory by id."),
    ("list_episodes", "List stored episodes."),
    ("delete_episode", "Delete an episode by id."),
    ("get_recall_precision", "Read an agent's recall precision."),
    ("set_recall_precision", "Set an agent's recall precision."),
]


def _handle_memory_tool(name, args):
    """Dispatch a memory-capability call. Returns a JSON-serialisable dict, or
    None when `name` is not a memory tool."""
    if name == "list_memories":
        agent_id = args.get("agent_id") or ""
        limit = int(args.get("limit") or 100)
        rows = [
            m
            for m in _MEMORIES.values()
            if not agent_id or m["agent_id"] == agent_id
        ]
        rows.sort(key=lambda m: m["id"], reverse=True)
        rows = rows[:limit]
        return {"memories": rows, "count": len(rows)}

    if name == "import_memories":
        path = args.get("input_path") or ""
        target = args.get("target_agent_id") or ""
        dry_run = bool(args.get("dry_run"))
        try:
            with open(path, encoding="utf-8") as fh:
                raw = fh.read()
        except OSError as e:
            return {"ok": False, "error": f"cannot read {path}: {e}", "imported": 0}
        imported = []
        for line in raw.splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                return {"ok": False, "error": "invalid JSONL", "imported": 0}
            if dry_run:
                imported.append(-1)
                continue
            mid = _NEXT_ID[0]
            _NEXT_ID[0] += 1
            _MEMORIES[mid] = {
                "id": mid,
                "agent_id": target or rec.get("agent_id") or "",
                "content": rec.get("content", ""),
                "created_at": mid,
                "locked": False,
            }
            imported.append(mid)
        return {
            "ok": True,
            "imported": len(imported),
            "ids": imported,
            "dry_run": dry_run,
        }

    if name == "update_memory":
        mid = int(args.get("memory_id", -1))
        row = _MEMORIES.get(mid)
        if row is None:
            return {"ok": False, "error": f"memory {mid} not found"}
        row["content"] = args.get("content", "")
        return {"ok": True, "id": mid, "content": row["content"]}

    if name == "delete_memory":
        mid = int(args.get("memory_id", -1))
        if _MEMORIES.pop(mid, None) is None:
            return {"ok": False, "error": f"memory {mid} not found"}
        return {"ok": True, "deleted_id": mid}

    if name == "list_episodes":
        rows = sorted(_EPISODES.values(), key=lambda e: e["id"], reverse=True)
        limit = int(args.get("limit") or 50)
        rows = rows[:limit]
        return {"episodes": rows, "count": len(rows)}

    if name == "delete_episode":
        eid = int(args.get("episode_id", -1))
        if _EPISODES.pop(eid, None) is None:
            return {"ok": False, "error": f"episode {eid} not found"}
        return {"ok": True, "deleted_id": eid}

    if name == "get_recall_precision":
        agent_id = args.get("agent_id") or ""
        return {
            "agent_id": agent_id,
            "precision": _PRECISION.get(agent_id, "balanced"),
        }

    if name == "set_recall_precision":
        agent_id = args.get("agent_id") or ""
        precision = args.get("precision") or ""
        if precision:
            _PRECISION[agent_id] = precision
        else:
            _PRECISION.pop(agent_id, None)
        return {
            "agent_id": agent_id,
            "precision": _PRECISION.get(agent_id, "balanced"),
        }

    return None


@app.list_tools()
async def list_tools():
    tools = [
        types.Tool(
            name="ping",
            description="Return pong.",
            inputSchema={"type": "object", "properties": {}},
        )
    ]
    if _memory_mode(sys.argv):
        tools += [
            types.Tool(
                name=n,
                description=d,
                inputSchema={"type": "object", "properties": {}},
            )
            for n, d in _MEMORY_TOOLS
        ]
    return tools


@app.call_tool()
async def call_tool(name, arguments):
    if _memory_mode(sys.argv):
        result = _handle_memory_tool(name, arguments or {})
        if result is not None:
            return [types.TextContent(type="text", text=json.dumps(result))]
    return [types.TextContent(type="text", text="pong")]


async def main():
    memory = _memory_mode(sys.argv)
    if memory:
        _seed_episodes()
    perms = _declared_perms(sys.argv)
    mgp = None
    if perms:
        mgp = {
            "version": _MGP_VERSION,
            "extensions": ["permissions"],
            "permissions_required": perms,
        }
    if memory:
        mgp = mgp or {"version": _MGP_VERSION, "extensions": []}
        # Pattern-C: name the capability each tool serves instead of hoping the
        # kernel's name heuristic recognises it (it does not know
        # `import_memories`).
        mgp["tools_for_capability"] = {"Memory": [n for n, _ in _MEMORY_TOOLS]}
    experimental = {"mgp": mgp} if mgp else None
    opts = app.create_initialization_options(experimental_capabilities=experimental)
    async with stdio_server() as (read, write):
        await app.run(read, write, opts)


if __name__ == "__main__":
    asyncio.run(main())
