"""Memory domain.

There is no direct create-memory REST route — memories are produced by the
Memory-capability MCP server (CPersona) via chat/store flows. On a fresh
isolated instance the list is therefore empty, but the *read path* still
returns a well-formed shape (memories[] + count + capabilities), which is a
valid operation-to-success check for phase 0.

The mutation half (``import`` / ``update`` / ``lock`` / ``unlock`` /
``delete``, plus episode deletion) needs a memory server to dispatch to: every
one of those routes goes through ``CapabilityType::Memory``, and with no
provider the kernel answers 400 or an empty fallback, so there is nothing to
assert. ``mutations`` therefore stands up the hermetic probe in ``--memory``
mode, which is a real in-process store — a write is visible to the next read
and an unknown id comes back ``{"ok": false}`` — and drives the whole chain
against it. Not phase 0: the probe needs the kernel's mcp-equipped venv, the
same dependency that keeps ``mcp.lifecycle`` out of the spine slice.
"""

from __future__ import annotations

from . import Operation, RunContext, register
from ._probe import register_probe, teardown_probe


@register
class MemoryList(Operation):
    domain = "memory"
    name = "list"
    covers = ["GET /api/memories"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/memories")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"memories payload not an object: {result!r}"
        assert isinstance(result.get("memories"), list), (
            f"memories[] missing/not a list: {result!r}"
        )
        assert "count" in result, f"count missing: {result!r}"
        assert isinstance(result.get("capabilities"), dict), (
            f"capabilities missing/not an object: {result!r}"
        )


@register
class MemoryEpisodes(Operation):
    domain = "memory"
    name = "episodes"
    covers = ["GET /api/episodes"]
    # Same class as `memory.list`: with no memory server the handler answers
    # its documented empty fallback rather than failing, so the read path is
    # assertable on a fresh empty DB.
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/episodes")

    def assert_success(self, ctx: RunContext, result):
        # accept a bare list or an {episodes: [...]} envelope.
        episodes = result.get("episodes") if isinstance(result, dict) else result
        assert isinstance(episodes, list), (
            f"episodes read did not return a list: {result!r}"
        )


_MEM_SERVER = "opverify-memory-probe"
_IMPORT_A = "opverify imported memory A"
_IMPORT_B = "opverify imported memory B"
_EDITED = "opverify imported memory A (edited)"


def _by_id(payload):
    rows = (payload or {}).get("memories") or []
    return {r.get("id"): r for r in rows if isinstance(r, dict)}


@register
class MemoryMutations(Operation):
    """The memory mutation surface, driven end to end against a probe server.

    import → list → update → lock → (delete refused) → unlock → delete →
    list-episodes → delete-episode.

    Two things are being proved, and they are different:

    * the kernel *forwards* correctly — the imported rows come back out of
      ``GET /api/memories`` with the content that was imported, the edit is
      visible on the next read, and the deleted row is gone from the next read.
      A handler that answered 200 without dispatching would fail all three.
    * the kernel's *own* lock invariant holds. The probe deliberately does not
      advertise ``lock_memory`` / ``unlock_memory``, so the kernel falls back to
      its ``memory_locks`` table and owns both the lock and the refusal. A
      locked memory must be refused a delete **and** an edit; after unlock both
      must work again. That refusal lives nowhere else in the test suite.
    """

    domain = "memory"
    name = "mutations"
    covers = [
        "POST /api/memories/import",
        "PUT /api/memories/{id}",
        "DELETE /api/memories/{id}",
        "POST /api/memories/{id}/lock",
        "POST /api/memories/{id}/unlock",
        "DELETE /api/episodes/{id}",
    ]
    phase0 = False

    def drive(self, ctx: RunContext):
        c = ctx.client
        ctx.scratch["memory_probe"] = _MEM_SERVER
        _reg, row = register_probe(
            c,
            _MEM_SERVER,
            extra_args=("--memory",),
            description="opverify memory-capability probe",
        )
        status = (row or {}).get("status")
        if status != "Connected":
            return {"status": status}

        jsonl = "\n".join(
            [
                '{"content": "%s"}' % _IMPORT_A,
                '{"content": "%s"}' % _IMPORT_B,
            ]
        )
        imported = c.post(
            "/api/memories/import",
            body={"data": jsonl, "agent_id": "agent.cloto_default"},
            timeout=30.0,
        )

        listed = c.get("/api/memories")
        rows = _by_id(listed)
        target = min(rows) if rows else None

        edited = c.put(f"/api/memories/{target}", body={"content": _EDITED})
        after_edit = _by_id(c.get("/api/memories"))

        locked = c.post(f"/api/memories/{target}/lock")
        after_lock = _by_id(c.get("/api/memories"))
        delete_locked_status, delete_locked_body = c.request_raw(
            "DELETE", f"/api/memories/{target}"
        )
        edit_locked_status, _ = c.request_raw(
            "PUT", f"/api/memories/{target}", body={"content": "must not stick"}
        )
        still_there = _by_id(c.get("/api/memories"))

        unlocked = c.post(f"/api/memories/{target}/unlock")
        after_unlock = _by_id(c.get("/api/memories"))
        c.delete(f"/api/memories/{target}")
        after_delete = _by_id(c.get("/api/memories"))

        episodes_before = _episodes(c.get("/api/episodes"))
        episode_id = episodes_before[0].get("id") if episodes_before else None
        c.delete(f"/api/episodes/{episode_id}")
        episodes_after = _episodes(c.get("/api/episodes"))

        return {
            "status": status,
            "imported": imported,
            "listed": rows,
            "target": target,
            "edited": edited,
            "after_edit": after_edit,
            "locked": locked,
            "after_lock": after_lock,
            "delete_locked_status": delete_locked_status,
            "delete_locked_body": delete_locked_body,
            "edit_locked_status": edit_locked_status,
            "still_there": still_there,
            "unlocked": unlocked,
            "after_unlock": after_unlock,
            "after_delete": after_delete,
            "episode_id": episode_id,
            "episode_ids_before": [e.get("id") for e in episodes_before],
            "episode_ids_after": [e.get("id") for e in episodes_after],
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["status"] == "Connected", (
            f"memory probe did not connect: {result['status']!r} — the memory "
            f"mutation routes have nothing to dispatch to"
        )

        assert (result["imported"] or {}).get("imported") == 2, (
            f"import did not report two rows: {result['imported']!r}"
        )
        contents = {r.get("content") for r in result["listed"].values()}
        assert {_IMPORT_A, _IMPORT_B} <= contents, (
            f"imported memories are not readable back: {sorted(contents)!r} "
            f"(the kernel wrote the JSONL somewhere the server never read)"
        )

        target = result["target"]
        assert target is not None, "no memory id to mutate"
        assert result["after_edit"][target]["content"] == _EDITED, (
            f"edit did not stick: "
            f"{result['after_edit'][target]['content']!r} != {_EDITED!r}"
        )

        assert (result["locked"] or {}).get("lock_level") == "kernel", (
            f"lock did not take the kernel fallback path: {result['locked']!r} "
            f"(the probe advertises no lock_memory, so it must)"
        )
        assert result["after_lock"][target].get("locked") is True, (
            f"the list does not report the memory as locked: "
            f"{result['after_lock'][target]!r}"
        )
        assert result["delete_locked_status"] == 400, (
            f"a locked memory was deletable: HTTP "
            f"{result['delete_locked_status']} {result['delete_locked_body'][:200]}"
        )
        assert result["edit_locked_status"] == 400, (
            f"a locked memory was editable: HTTP {result['edit_locked_status']}"
        )
        assert result["still_there"][target]["content"] == _EDITED, (
            "the refused write changed the memory anyway: "
            f"{result['still_there'][target]!r}"
        )

        assert (result["unlocked"] or {}).get("lock_level") == "kernel", (
            f"unlock did not take the kernel fallback path: {result['unlocked']!r}"
        )
        assert result["after_unlock"][target].get("locked") is not True, (
            f"the memory is still reported locked after unlock: "
            f"{result['after_unlock'][target]!r}"
        )
        assert target not in result["after_delete"], (
            f"memory {target} survived the delete: "
            f"{sorted(result['after_delete'])}"
        )
        assert len(result["after_delete"]) == len(result["listed"]) - 1, (
            f"delete removed the wrong number of rows: "
            f"{sorted(result['after_delete'])} from {sorted(result['listed'])}"
        )

        assert result["episode_id"] is not None, (
            "the probe exposed no episode to delete"
        )
        assert result["episode_id"] not in result["episode_ids_after"], (
            f"episode {result['episode_id']} survived the delete: "
            f"{result['episode_ids_after']}"
        )
        assert len(result["episode_ids_after"]) == len(
            result["episode_ids_before"]
        ) - 1, (
            f"episode delete changed the set by more than one row: "
            f"{result['episode_ids_before']} -> {result['episode_ids_after']}"
        )

    def teardown(self, ctx: RunContext):
        name = ctx.scratch.pop("memory_probe", None)
        if name:
            teardown_probe(ctx.client, name)


def _episodes(payload):
    if isinstance(payload, dict):
        return payload.get("episodes") or []
    return payload or []
