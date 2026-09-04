"""Operation catalog — the set of real operations the harness drives to
success. Each domain lives in its own module (agents.py, chat.py, mcp.py …)
and registers one or more :class:`Operation` instances.

An operation declares the HTTP routes it exercises via ``covers`` so the
coverage ratchet (``opverify.coverage``) can prove the catalog stays
exhaustive: every meaningful kernel route must be claimed by some
operation, or the ratchet fails.

"Success" means *the operation actually took effect* — asserted in
``assert_success`` — not merely that nothing crashed.
"""

from __future__ import annotations

import importlib
from dataclasses import dataclass, field
from typing import Any, List

from ..client import ClotoClient

# Catalog submodules to import (each appends to REGISTRY on import).
_MODULES = [
    "health",
    "agents",
    "memory",
    "events",
    "mcp",
    "chat",
    "llm",
    "cron",
    "marketplace",
    "system",
    "settings",
    "plugins",
    "permissions",
    "setup",
]

REGISTRY: List["Operation"] = []


def register(cls):
    """Class decorator: instantiate and add to the global registry."""
    REGISTRY.append(cls())
    return cls


@dataclass
class RunContext:
    """Shared state threaded through every operation in a single run."""

    client: ClotoClient
    target: Any  # opverify.deploy.RunningTarget
    scratch: dict = field(default_factory=dict)
    logs: List[str] = field(default_factory=list)

    def log(self, msg: str) -> None:
        self.logs.append(msg)


class Operation:
    """Base class for a single operation-to-success check."""

    domain: str = ""
    name: str = ""
    covers: List[str] = []
    # Included in the phase-0 "spine" slice (a small, LLM-free subset used to
    # prove the harness end-to-end before the full catalog exists).
    phase0: bool = False
    # Execution rank. Operations run in registration order within a rank, and
    # ranks run ascending, so an operation that changes state every *later*
    # operation depends on (the admin key rotation) can declare that it must
    # run after the rest without the catalog's module order encoding it.
    # Default 0 = "no ordering opinion", which is every other operation.
    order: int = 0
    # True when the operation cannot run against a fresh empty DB because it
    # needs state only the operator's seeded DB carries (a real provider key, a
    # reasoning engine row). The runner uses this to decide whether to boot
    # from a rewired *copy* of the dev DB. It is declared per operation rather
    # than inferred from the domain: `chat.messages` is a chat operation that
    # only exercises persistence and must see an empty transcript, so a
    # domain-level guess would seed a run that needs no seed — and hand every
    # other phase-0 operation a DB full of the operator's real state.
    needs_seed: bool = False

    @property
    def key(self) -> str:
        return f"{self.domain}.{self.name}"

    def drive(self, ctx: RunContext) -> Any:
        """Perform the operation. Return any value needed by assert_success."""
        raise NotImplementedError

    def assert_success(self, ctx: RunContext, result: Any) -> None:
        """Raise AssertionError / ApiError if the operation did not truly
        take effect."""
        raise NotImplementedError

    def teardown(self, ctx: RunContext) -> None:
        """Best-effort cleanup; must not raise for normal missing state."""
        return None


def load_all() -> List[Operation]:
    """Import every catalog submodule (populating REGISTRY) and return it."""
    REGISTRY.clear()
    for mod in _MODULES:
        importlib.import_module(f"{__name__}.{mod}")
    return list(REGISTRY)


def select(operations: List[Operation], phase0_only: bool = False) -> List[Operation]:
    """Filter to the requested slice and put the result in execution order.

    ``sorted`` is stable, so operations that share a rank keep their
    registration order — the ordering attribute only moves the few operations
    that genuinely have to run late, and changes nothing else.
    """
    selected = [op for op in operations if op.phase0] if phase0_only else list(operations)
    return sorted(selected, key=lambda op: op.order)
