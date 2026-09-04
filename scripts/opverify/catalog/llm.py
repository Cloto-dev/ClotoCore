"""LLM provider domain — the centralized key/model management surface
(MGP §13.4). Runs in seed mode, where ``deepseek`` carries a real key.

* ``inspect``       — list providers (deepseek present, key masked to a
                      ``has_key`` flag) and list its upstream models.
* ``configure``     — set the provider's model and prove it stuck, then run a
                      real ``/test`` connection (reachable + auth_ok) against
                      the upstream using the copied key.
* ``knobs``         — the remaining per-provider writes (key set/delete,
                      context length, thinking mode). None of them contacts an
                      upstream; each is a DB write whose only observable is
                      ``GET /api/llm/providers``.

``knobs`` cannot be a phase-0 operation, and not because of an LLM: an
``llm_providers`` row is only ever created by a marketplace install ingesting a
connector's provider block (``db::upsert_llm_provider_meta``), so a fresh empty
DB has **zero** providers and there is nothing for the routes to address.
"""

from __future__ import annotations

from . import Operation, RunContext, register

_PROVIDER = "deepseek"
_MODEL = "deepseek-chat"


def _provider_list(client):
    """GET /api/llm/providers returns ``{providers: [...]}`` (unlike
    /api/agents which returns a bare list) — normalise to the list."""
    resp = client.get("/api/llm/providers")
    if isinstance(resp, dict):
        return resp.get("providers", [])
    return resp if isinstance(resp, list) else []


def _find(providers, pid):
    return next((p for p in providers if p.get("id") == pid), None)


@register
class LlmInspect(Operation):
    domain = "llm"
    name = "inspect"
    covers = [
        "GET /api/llm/providers",
        "GET /api/llm/providers/{id}/models",
    ]
    phase0 = False
    needs_seed = True

    def drive(self, ctx: RunContext):
        providers = _provider_list(ctx.client)
        models = ctx.client.get(f"/api/llm/providers/{_PROVIDER}/models", timeout=30.0)
        return {"providers": providers, "models": models}

    def assert_success(self, ctx: RunContext, result):
        providers = result["providers"]
        assert isinstance(providers, list) and providers, (
            f"providers not a non-empty list: {providers!r}"
        )
        p = _find(providers, _PROVIDER)
        assert p is not None, f"provider {_PROVIDER!r} not present"
        assert p.get("has_key") is True, (
            f"{_PROVIDER} has no key in seed mode (has_key={p.get('has_key')!r})"
        )
        # kernel masks keys — the raw value must never appear in the payload.
        assert "api_key" not in p, "provider payload leaked a raw api_key field"
        models = result["models"]
        # models may be {"models": [...]} or a bare list depending on upstream.
        model_list = models.get("models") if isinstance(models, dict) else models
        assert isinstance(model_list, list) and model_list, (
            f"no upstream models returned for {_PROVIDER}: {models!r}"
        )


@register
class LlmConfigure(Operation):
    domain = "llm"
    name = "configure"
    covers = [
        "POST /api/llm/providers/{id}/model",
        "POST /api/llm/providers/{id}/test",
    ]
    phase0 = False
    needs_seed = True

    def drive(self, ctx: RunContext):
        c = ctx.client
        c.post(f"/api/llm/providers/{_PROVIDER}/model", body={"model_id": _MODEL})
        after = _find(_provider_list(c), _PROVIDER)
        test = c.post(f"/api/llm/providers/{_PROVIDER}/test", timeout=30.0)
        return {"after_model": (after or {}).get("model_id"), "test": test}

    def assert_success(self, ctx: RunContext, result):
        assert result["after_model"] == _MODEL, (
            f"model did not persist: {result['after_model']!r} != {_MODEL!r}"
        )
        test = result["test"]
        assert isinstance(test, dict), f"test payload not an object: {test!r}"
        assert test.get("reachable") is True, (
            f"provider {_PROVIDER} unreachable: {test!r}"
        )
        assert test.get("auth_ok") is True, (
            f"provider {_PROVIDER} auth failed (bad/expired key?): {test!r}"
        )


@register
class LlmKnobs(Operation):
    """Key / context-length / thinking-mode, set and read back.

    Driven against a provider that has **no key**, and every write is undone,
    so the operation is a no-op on the instance it ran against: the dummy key
    is deleted (returning the provider to keyless, which is exactly where it
    started) and the two other knobs are restored to their prior values. That
    matters because these run inside a *copy* of the operator's DB, where the
    other providers carry real keys — picking a keyless one means nothing real
    is ever overwritten and nothing has to be remembered to be put back.

    The key is asserted through ``has_key`` only. The kernel never returns the
    value (it masks it, and ``llm.inspect`` asserts the raw field is absent),
    so `has_key` flipping false→true→false across set/delete is the whole
    observable, and it is enough: it distinguishes a stored write from a
    discarded one.
    """

    domain = "llm"
    name = "knobs"
    covers = [
        "POST /api/llm/providers/{id}/key",
        "DELETE /api/llm/providers/{id}/key",
        "POST /api/llm/providers/{id}/context-length",
        "POST /api/llm/providers/{id}/thinking-mode",
    ]
    phase0 = False
    # A fresh empty DB has zero `llm_providers` rows (they are created by
    # marketplace install), so there is nothing to address without a seed.
    needs_seed = True

    _DUMMY_KEY = "opverify-dummy-key-not-a-credential"
    _CONTEXT = 4242

    def drive(self, ctx: RunContext):
        c = ctx.client
        providers = _provider_list(c)
        target = next((p for p in providers if not p.get("has_key")), None)
        if target is None:
            return {"target": None, "provider_count": len(providers)}
        pid = target["id"]
        ctx.scratch["llm_knob_provider"] = pid
        ctx.scratch["llm_knob_restore"] = {
            "context_length": target.get("context_length"),
            "thinking_mode": target.get("thinking_mode"),
        }

        c.post(f"/api/llm/providers/{pid}/key", body={"api_key": self._DUMMY_KEY})
        after_key = _find(_provider_list(c), pid)

        c.post(
            f"/api/llm/providers/{pid}/context-length",
            body={"context_length": self._CONTEXT},
        )
        after_context = _find(_provider_list(c), pid)

        booted_mode = target.get("thinking_mode")
        new_mode = "on" if booted_mode != "on" else "off"
        c.post(f"/api/llm/providers/{pid}/thinking-mode", body={"value": new_mode})
        after_mode = _find(_provider_list(c), pid)

        bad_mode_status, _ = c.request_raw(
            "POST",
            f"/api/llm/providers/{pid}/thinking-mode",
            body={"value": "opverify-not-a-mode"},
        )
        bad_context_status, _ = c.request_raw(
            "POST",
            f"/api/llm/providers/{pid}/context-length",
            body={"context_length": -1},
        )
        after_refusals = _find(_provider_list(c), pid)

        c.delete(f"/api/llm/providers/{pid}/key")
        after_delete = _find(_provider_list(c), pid)

        # Restore the two knobs to whatever the provider carried before.
        restore = ctx.scratch.get("llm_knob_restore") or {}
        c.post(
            f"/api/llm/providers/{pid}/context-length",
            body={"context_length": restore.get("context_length")},
        )
        c.post(
            f"/api/llm/providers/{pid}/thinking-mode",
            body={"value": restore.get("thinking_mode") or "auto"},
        )
        restored = _find(_provider_list(c), pid)
        ctx.scratch.pop("llm_knob_provider", None)
        ctx.scratch.pop("llm_knob_restore", None)

        return {
            "target": pid,
            "booted": target,
            "after_key": after_key,
            "after_context": after_context,
            "new_mode": new_mode,
            "after_mode": after_mode,
            "bad_mode_status": bad_mode_status,
            "bad_context_status": bad_context_status,
            "after_refusals": after_refusals,
            "after_delete": after_delete,
            "restored": restored,
        }

    def assert_success(self, ctx: RunContext, result):
        assert result["target"] is not None, (
            f"no keyless LLM provider to drive the knobs against "
            f"({result.get('provider_count')} provider(s) present). A fresh "
            f"empty DB has none at all — providers are created by marketplace "
            f"install, so this operation needs a seeded instance."
        )
        assert result["booted"].get("has_key") is False, (
            f"the chosen provider was not keyless: {result['booted']!r}"
        )
        assert result["after_key"].get("has_key") is True, (
            f"setting a key did not flip has_key: {result['after_key']!r}"
        )
        assert "api_key" not in result["after_key"], (
            "the provider payload leaked a raw api_key field after a key was set"
        )
        assert result["after_context"].get("context_length") == self._CONTEXT, (
            f"context_length did not persist: "
            f"{result['after_context'].get('context_length')!r}"
        )
        assert result["after_mode"].get("thinking_mode") == result["new_mode"], (
            f"thinking_mode did not persist: "
            f"{result['after_mode'].get('thinking_mode')!r} != "
            f"{result['new_mode']!r}"
        )
        assert result["bad_mode_status"] == 400, (
            f"an unknown thinking_mode was accepted: HTTP "
            f"{result['bad_mode_status']}"
        )
        assert result["bad_context_status"] == 400, (
            f"a non-positive context_length was accepted: HTTP "
            f"{result['bad_context_status']}"
        )
        assert (
            result["after_refusals"].get("thinking_mode") == result["new_mode"]
            and result["after_refusals"].get("context_length") == self._CONTEXT
        ), (
            f"a refused write still changed the provider: "
            f"{result['after_refusals']!r}"
        )
        assert result["after_delete"].get("has_key") is False, (
            f"deleting the key did not clear has_key: {result['after_delete']!r}"
        )
        booted, restored = result["booted"], result["restored"]
        assert restored.get("context_length") == booted.get("context_length"), (
            f"context_length was not restored: "
            f"{restored.get('context_length')!r} != "
            f"{booted.get('context_length')!r}"
        )
        assert restored.get("thinking_mode") == booted.get("thinking_mode"), (
            f"thinking_mode was not restored: "
            f"{restored.get('thinking_mode')!r} != "
            f"{booted.get('thinking_mode')!r}"
        )

    def teardown(self, ctx: RunContext):
        pid = ctx.scratch.pop("llm_knob_provider", None)
        restore = ctx.scratch.pop("llm_knob_restore", None)
        if not pid:
            return
        for call in (
            lambda: ctx.client.delete(f"/api/llm/providers/{pid}/key"),
            lambda: ctx.client.post(
                f"/api/llm/providers/{pid}/context-length",
                body={"context_length": (restore or {}).get("context_length")},
            ),
            lambda: ctx.client.post(
                f"/api/llm/providers/{pid}/thinking-mode",
                body={"value": (restore or {}).get("thinking_mode") or "auto"},
            ),
        ):
            try:
                call()
            except Exception:  # noqa: BLE001
                pass
