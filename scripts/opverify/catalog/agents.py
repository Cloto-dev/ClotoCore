"""Agents domain — full create → observe → power → delete lifecycle, plus the
per-agent configuration surface the dashboard drives.

Success = every stage is externally observable: the created agent appears in
the list, power toggle reports the new state, and after delete it is gone.
No LLM is involved (power toggle only flips a flag), so this is a phase-0
spine operation. Field-name gotchas: create uses ``default_engine``; power
uses ``enabled``; the list/response echo ``default_engine_id``.

The added operations cover the rest of ``/api/agents/{id}/…``:

* ``update``       — ``POST /api/agents/{id}`` answers ``{}``, so the proof
                     that an edit stuck is a re-read of the list.
* ``mcp_access``   — the agent-centric bulk grant replacement, read back
                     through ``GET /api/mcp/access/by-agent/{id}``.
* ``last_usage``   — the token-usage badge the dashboard polls after a turn.
* ``visemes``      — a pure function (text → lip-sync timeline), no TTS.
* ``recall_precision`` — the optional, feature-detected memory knob; needs a
                     memory server, so it stands the probe up (not phase 0,
                     like every other probe-backed operation here).
"""

from __future__ import annotations

from . import Operation, RunContext, register
from ._probe import register_probe, teardown_probe

_TEST_NAME = "opverify-lifecycle-agent"
_DEFAULT_AGENT = "agent.cloto_default"


def _find(agents, agent_id):
    return next((a for a in agents if a.get("id") == agent_id), None)


@register
class AgentsList(Operation):
    domain = "agents"
    name = "list"
    covers = ["GET /api/agents"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get("/api/agents")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, list), f"agents list not an array: {result!r}"
        assert result, "agents list empty (expected seeded default agent)"
        for a in result:
            assert "id" in a and "enabled" in a, f"agent missing fields: {a!r}"


@register
class AgentsLifecycle(Operation):
    domain = "agents"
    name = "lifecycle"
    covers = [
        "POST /api/agents",
        "POST /api/agents/{id}/power",
        "DELETE /api/agents/{id}",
    ]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        # 1. create
        created = c.post(
            "/api/agents",
            body={
                "name": _TEST_NAME,
                "description": "opverify lifecycle probe agent",
                "default_engine": "cerebras",
            },
        )
        agent_id = created["id"]
        ctx.scratch["lifecycle_agent_id"] = agent_id

        # 2. appears in list
        listed = c.get("/api/agents")
        present = _find(listed, agent_id) is not None

        # 3. power on / off
        on = c.post(f"/api/agents/{agent_id}/power", body={"enabled": True})
        off = c.post(f"/api/agents/{agent_id}/power", body={"enabled": False})

        # 4. delete
        c.delete(f"/api/agents/{agent_id}")
        after = c.get("/api/agents")
        gone = _find(after, agent_id) is None
        ctx.scratch.pop("lifecycle_agent_id", None)

        return {
            "agent_id": agent_id,
            "present_after_create": present,
            "power_on": on,
            "power_off": off,
            "gone_after_delete": gone,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["agent_id"], "no agent id returned from create"
        assert result["present_after_create"], "created agent not found in list"
        assert result["power_on"].get("enabled") is True, (
            f"power-on did not report enabled=true: {result['power_on']!r}"
        )
        assert result["power_off"].get("enabled") is False, (
            f"power-off did not report enabled=false: {result['power_off']!r}"
        )
        assert result["gone_after_delete"], "agent still present after delete"

    def teardown(self, ctx: RunContext):
        agent_id = ctx.scratch.pop("lifecycle_agent_id", None)
        if agent_id:
            try:
                ctx.client.delete(f"/api/agents/{agent_id}")
            except Exception:
                pass


@register
class AgentsUpdate(Operation):
    """Edit an agent and prove the edit persisted.

    The default agent is protected against name/description edits by the
    handler, so this drives a throwaway agent of its own. ``update`` answers a
    bare ``{}`` — the only honest proof is reading the agent back out of the
    list and finding the new values there.
    """

    domain = "agents"
    name = "update"
    covers = ["POST /api/agents/{id}"]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        created = c.post(
            "/api/agents",
            body={
                "name": "opverify-update-agent",
                "description": "opverify update probe agent",
                "default_engine": "cerebras",
            },
        )
        agent_id = created["id"]
        ctx.scratch["update_agent_id"] = agent_id

        before = _find(c.get("/api/agents"), agent_id)
        c.post(
            f"/api/agents/{agent_id}",
            body={
                "name": "opverify-update-agent-renamed",
                "description": "opverify update probe agent (edited)",
                "default_engine_id": "deepseek",
            },
        )
        after = _find(c.get("/api/agents"), agent_id)

        c.delete(f"/api/agents/{agent_id}")
        ctx.scratch.pop("update_agent_id", None)
        return {"agent_id": agent_id, "before": before, "after": after}

    def assert_success(self, ctx: RunContext, result):
        before, after = result["before"], result["after"]
        assert before is not None, "created agent missing from list before update"
        assert after is not None, "agent missing from list after update"
        assert after.get("name") == "opverify-update-agent-renamed", (
            f"name did not persist: {after.get('name')!r}"
        )
        assert after.get("description") == "opverify update probe agent (edited)", (
            f"description did not persist: {after.get('description')!r}"
        )
        assert after.get("default_engine_id") == "deepseek", (
            f"default_engine_id did not persist: {after.get('default_engine_id')!r} "
            f"(was {before.get('default_engine_id')!r})"
        )

    def teardown(self, ctx: RunContext):
        agent_id = ctx.scratch.pop("update_agent_id", None)
        if agent_id:
            try:
                ctx.client.delete(f"/api/agents/{agent_id}")
            except Exception:  # noqa: BLE001
                pass


@register
class AgentsMcpAccess(Operation):
    """Replace an agent's whole ``server_grant`` set in one call.

    ``PUT /api/agents/{id}/mcp-access`` is the bulk path the dashboard uses
    instead of 2N single-grant calls. The response only reports a count, so
    success is asserted from the read side (``GET /api/mcp/access/by-agent``):
    the granted ids are present as ``server_grant`` entries afterwards, and a
    second PUT with a *different* set replaces rather than accumulates — the
    property that makes it a "put" and not a "post".

    Everything is asserted **relative to the grant set the instance already
    had**, and that set is written back at the end. A fresh instance has none;
    a seeded one carries the operator's real grants, and an operation that
    assumed "empty" would either fail there or, worse, quietly discard them.

    The handler inserts a ``config-loaded`` placeholder row in ``mcp_servers``
    for any id that has none, so the grant's foreign key holds; teardown
    removes those placeholders and restores the original grants.
    """

    domain = "agents"
    name = "mcp_access"
    covers = ["PUT /api/agents/{id}/mcp-access"]
    phase0 = True

    _FIRST = ["opverify-grant-a", "opverify-grant-b"]
    _SECOND = ["opverify-grant-b"]

    def drive(self, ctx: RunContext):
        c = ctx.client
        agent = _DEFAULT_AGENT
        ctx.scratch["mcp_access_placeholders"] = list(
            dict.fromkeys(self._FIRST + self._SECOND)
        )

        original = _grants(c.get(f"/api/mcp/access/by-agent/{agent}"))
        ctx.scratch["mcp_access_original"] = sorted(original)

        first = c.put(
            f"/api/agents/{agent}/mcp-access",
            body={"granted_server_ids": self._FIRST},
        )
        after_first = _grants(c.get(f"/api/mcp/access/by-agent/{agent}"))
        second = c.put(
            f"/api/agents/{agent}/mcp-access",
            body={"granted_server_ids": self._SECOND},
        )
        after_second = _grants(c.get(f"/api/mcp/access/by-agent/{agent}"))

        # Put the agent back on the grant set it booted with.
        c.put(
            f"/api/agents/{agent}/mcp-access",
            body={"granted_server_ids": sorted(original)},
        )
        restored = _grants(c.get(f"/api/mcp/access/by-agent/{agent}"))
        ctx.scratch.pop("mcp_access_original", None)

        return {
            "original": original,
            "first_count": first,
            "after_first": after_first,
            "second_count": second,
            "after_second": after_second,
            "restored": restored,
        }

    def assert_success(self, ctx: RunContext, result):
        assert not (set(self._FIRST) & result["original"]), (
            f"the probe grant ids collide with real ones: {result['original']}"
        )
        assert result["first_count"].get("count") == len(self._FIRST), (
            f"first PUT reported {result['first_count']!r}, wanted count="
            f"{len(self._FIRST)}"
        )
        assert result["after_first"] == set(self._FIRST), (
            f"grants after first PUT: {sorted(result['after_first'])} != "
            f"{sorted(self._FIRST)} — a PUT that merged instead of replacing "
            f"would still contain {sorted(result['original'])}"
        )
        assert result["after_second"] == set(self._SECOND), (
            f"second PUT did not replace the grant set: "
            f"{sorted(result['after_second'])} != {sorted(self._SECOND)} "
            f"(a PUT that accumulates would still contain "
            f"{sorted(set(self._FIRST) - set(self._SECOND))})"
        )
        assert result["restored"] == result["original"], (
            f"the agent's original grants were not restored: "
            f"{sorted(result['restored'])} != {sorted(result['original'])}"
        )

    def teardown(self, ctx: RunContext):
        original = ctx.scratch.pop("mcp_access_original", None)
        if original is not None:
            try:
                ctx.client.put(
                    f"/api/agents/{_DEFAULT_AGENT}/mcp-access",
                    body={"granted_server_ids": original},
                )
            except Exception:  # noqa: BLE001
                pass
        for name in ctx.scratch.pop("mcp_access_placeholders", []) or []:
            try:
                ctx.client.delete(f"/api/mcp/servers/{name}", timeout=15.0)
            except Exception:  # noqa: BLE001
                pass


@register
class AgentsLastUsage(Operation):
    """The context-usage badge's source.

    Read-only and process-scoped: ``last_usage`` is an in-memory map filled by
    the agentic loop, so a fresh instance answers ``{"usage": null}``. The
    assertion is that the route answers with the envelope at all (the key
    present, not merely a 200 with some other shape) — a handler that stopped
    emitting ``usage`` would break the badge silently.
    """

    domain = "agents"
    name = "last_usage"
    covers = ["GET /api/agents/{id}/last-usage"]
    phase0 = True

    def drive(self, ctx: RunContext):
        return ctx.client.get(f"/api/agents/{_DEFAULT_AGENT}/last-usage")

    def assert_success(self, ctx: RunContext, result):
        assert isinstance(result, dict), f"last-usage not an object: {result!r}"
        assert "usage" in result, f"last-usage payload has no 'usage' key: {result!r}"
        usage = result["usage"]
        assert usage is None or isinstance(usage, dict), (
            f"usage is neither null nor an object: {usage!r}"
        )


@register
class AgentsVisemes(Operation):
    """Text → lip-sync timeline.

    A pure kernel function (``crate::viseme::generate_timeline``) — no TTS, no
    audio, no network. Success is a timeline that actually describes the input:
    non-empty entries, a positive total duration, and entries whose offsets do
    not run backwards. Empty text must not invent phonemes.
    """

    domain = "agents"
    name = "visemes"
    covers = ["POST /api/agents/{id}/visemes"]
    phase0 = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        spoken = c.post(
            f"/api/agents/{_DEFAULT_AGENT}/visemes",
            body={"text": "hello opverify"},
        )
        empty = c.post(f"/api/agents/{_DEFAULT_AGENT}/visemes", body={"text": ""})
        return {"spoken": spoken, "empty": empty}

    def assert_success(self, ctx: RunContext, result):
        spoken = result["spoken"]
        assert isinstance(spoken, dict), f"viseme timeline not an object: {spoken!r}"
        entries = spoken.get("entries")
        assert isinstance(entries, list) and entries, (
            f"no viseme entries produced for non-empty text: {spoken!r}"
        )
        duration = spoken.get("total_duration_ms")
        assert isinstance(duration, (int, float)) and duration > 0, (
            f"timeline has no positive duration: {duration!r}"
        )
        starts = [
            e.get("start_ms")
            for e in entries
            if isinstance(e, dict) and isinstance(e.get("start_ms"), (int, float))
        ]
        assert len(starts) == len(entries), (
            f"a viseme entry carries no numeric start_ms: {entries[:4]!r}"
        )
        assert starts == sorted(starts), (
            f"viseme start times are not monotonic: {starts[:12]}"
        )
        last = entries[-1]
        assert duration >= last["start_ms"] + last["duration_ms"], (
            f"total_duration_ms {duration} is shorter than the last entry "
            f"(start {last['start_ms']} + duration {last['duration_ms']})"
        )
        empty_entries = (result["empty"] or {}).get("entries")
        assert empty_entries == [], (
            f"empty text produced visemes: {result['empty']!r}"
        )


@register
class AgentsRecallPrecision(Operation):
    """The recall-precision knob (knob 3), end to end through the dispatcher.

    Both routes are OPTIONAL and feature-detected: with no memory server the
    kernel answers 400 ("does not support recall precision"), which is why a
    memory probe is stood up first. Success is a genuine round trip — set to a
    value the server did not previously hold, then read it back through the
    *other* route and get that value. A kernel that dropped the request, or
    forwarded it to the wrong server, cannot produce that.

    Not phase 0: registering a probe MCP server needs the kernel's mcp-equipped
    venv, the same environment dependency that keeps ``mcp.lifecycle`` and
    ``permissions.decide`` out of the spine slice.
    """

    domain = "agents"
    name = "recall_precision"
    covers = [
        "GET /api/agents/{id}/recall-precision",
        "POST /api/agents/{id}/recall-precision",
    ]
    phase0 = False

    _SERVER = "opverify-precision-probe"

    def drive(self, ctx: RunContext):
        c = ctx.client
        ctx.scratch["precision_probe"] = self._SERVER
        _reg, row = register_probe(
            c,
            self._SERVER,
            extra_args=("--memory",),
            description="opverify recall-precision probe (memory capability)",
        )

        initial = c.get(f"/api/agents/{_DEFAULT_AGENT}/recall-precision")
        written = c.post(
            f"/api/agents/{_DEFAULT_AGENT}/recall-precision",
            body={"precision": "strict"},
        )
        read_back = c.get(f"/api/agents/{_DEFAULT_AGENT}/recall-precision")
        # A second agent must not inherit the first agent's override — the
        # value is keyed per agent, and the route carries the agent in its path.
        other = c.get("/api/agents/agent.opverify_absent/recall-precision")

        return {
            "status": (row or {}).get("status"),
            "initial": initial,
            "written": written,
            "read_back": read_back,
            "other_agent": other,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["status"] == "Connected", (
            f"memory probe did not connect: {result['status']!r} — recall "
            f"precision cannot be dispatched without a memory server"
        )
        assert (result["initial"] or {}).get("precision") == "balanced", (
            f"probe did not start at its default precision: {result['initial']!r}"
        )
        assert (result["written"] or {}).get("precision") == "strict", (
            f"set_recall_precision did not report the new value: "
            f"{result['written']!r}"
        )
        assert (result["read_back"] or {}).get("precision") == "strict", (
            f"the value did not survive the round trip: {result['read_back']!r} "
            f"(the kernel forwarded a write nobody can read back)"
        )
        assert (result["other_agent"] or {}).get("precision") == "balanced", (
            f"an unrelated agent picked up the override: {result['other_agent']!r} "
            f"(the agent id in the path is not reaching the memory server)"
        )

    def teardown(self, ctx: RunContext):
        name = ctx.scratch.pop("precision_probe", None)
        if name:
            teardown_probe(ctx.client, name)


def _grants(access_payload) -> set:
    """The ``server_grant`` server ids in a by-agent access payload."""
    entries = (access_payload or {}).get("entries") or []
    return {
        e.get("server_id")
        for e in entries
        if isinstance(e, dict) and e.get("entry_type") == "server_grant"
    }
