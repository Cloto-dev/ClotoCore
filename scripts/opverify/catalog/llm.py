"""LLM provider domain — the centralized key/model management surface
(MGP §13.4). Runs in seed mode, where ``deepseek`` carries a real key.

* ``inspect``       — list providers (deepseek present, key masked to a
                      ``has_key`` flag) and list its upstream models.
* ``configure``     — set the provider's model and prove it stuck, then run a
                      real ``/test`` connection (reachable + auth_ok) against
                      the upstream using the copied key.
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
