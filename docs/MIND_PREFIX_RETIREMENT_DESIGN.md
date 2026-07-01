# `mind.` Prefix Retirement — Design

**Status:** Proposed
**Scope:** CScheduler Scope #12 · Goal #142
**Author:** kernel team · 2026-07-01
**Non-goal / sequel:** full category-prefix retirement (`tool.` / `vision.` /
`voice.` / `io.`) is tracked separately as Goal #143 and is out of scope here.

Retire the `mind.` engine-id prefix so every reasoning engine is identified by a
single canonical **bare** id (`deepseek`, `local`, `ollama`, …). This removes the
id duality that is the root cause of the DeepSeek "not connected" bug (bug-425)
and the capability-gate denial of wizard-configured engines, and lets us delete
the `mind.`-normalization shims scattered through the kernel.

---

## 1. Problem & current state

Reasoning engines are identified inconsistently:

- **Catalog engines are bare.** The deployed ClotoHub catalog
  (`https://hub.cloto.dev/api/catalog`) lists the DeepSeek engine with id
  `deepseek`, and install registers a server under the catalog id verbatim
  (`crates/core/src/handlers/marketplace.rs` install validates
  `.find(|s| s.id == request.server_id)` and registers that id). Confirmed at
  runtime: `mcp_servers` holds `deepseek` (not `mind.deepseek`). This is the
  bug-388 / bug-396 de-prefixing already in effect.
- **Built-in engines keep the prefix.** `mind.local` and `mind.ollama` are
  registered by the kernel with the `mind.` prefix intact (confirmed: DB row
  `mind.local`; `mind.ollama` is special-cased by exact id at
  `crates/core/src/managers/mcp.rs:1503`).

So the id space is **split**: some engines are bare, some are `mind.`-prefixed.
Every consumer must paper over the gap, and the places that forget produce bugs.

### Symptoms this causes

- **bug-425 (scenario A) — frontend drop.** The agent console resolved granted
  engines with a prefix-only filter (`server_id.startsWith('mind.')`), so a
  canonically-keyed `deepseek` grant was dropped from the selector and the
  engine showed as "not connected". Fixed as a stopgap by switching to the
  tool-surface `isEngineServer` test (`dashboard/src/lib/serverCategory.ts`);
  the underlying duality remains.
- **scenario B — capability-gate denial.** The Setup Wizard persists the legacy
  alias `mind.deepseek` as both `default_engine_id` and the `server_grant` key
  (`dashboard/src/components/SetupWizard.tsx:40,101,190-191,218,224`;
  `dashboard/src/lib/presets.ts:21-24`). The kernel resolves the engine *server*
  by stripping `mind.` (`crates/core/src/handlers/system.rs:2252`), but the
  capability gate keys strictly on the resolved id `deepseek`
  (`crates/core/src/db/mcp.rs:522-597`, no normalization). A grant keyed
  `mind.deepseek` therefore does not cover `deepseek`, falls through to the
  server's `opt-in` `default_policy`, and is **denied** — so a fresh
  wizard-configured DeepSeek engine is unusable. PR #240 (bug-421 gate
  unification) is the trigger that routed engine `think` through this gate.
- **bug-419 class — grant matching.** Global-vs-granted engine matching only
  works because of ad-hoc `mind.`-stripping
  (`crates/core/src/handlers/system.rs:1519,1527`).

### The prefix is already redundant

Server classification no longer needs the prefix. Both the kernel
(`classify_tool` / `capability_dispatcher.rs`) and the frontend
(`serverCategory.ts` — `isEngineServer` keys on the `think` /
`think_with_tools` tool surface, `isMemoryServer` on the memory tool surface)
categorise by **tool surface**, not id prefix. `mind.` carries no information
that isn't already derivable, so retiring it is a de-facto no-op for
classification.

---

## 2. Root cause

A single engine has two spellings (`mind.deepseek` ≡ `deepseek`) and the codebase
half-normalizes between them. The fix is not to add more normalization (an alias
table, or teaching the gate to strip `mind.`) but to **collapse the two spellings
into one** — bare — and delete the normalization.

---

## 3. Design overview

1. **Canonical id = bare.** Every engine is identified by its bare id. Catalog
   engines already are; built-in engines (`local`, `ollama`) are changed to
   match.
2. **One-shot DB migration** renames existing `mind.local` / `mind.ollama` rows
   to bare across the three places they are stored (§6), modelled on the bug-388
   `memory.cpersona → cpersona` repair.
3. **Remove the now-redundant `mind.`-normalization shims** (§5). Once ids are
   uniform these are no-ops; deleting them removes the foot-gun that let the gate
   diverge.
4. **Producers write bare** — kernel built-in registration, Setup Wizard,
   presets, config examples (§5, §7).
5. **Compat is one-shot, not ongoing.** After the migration there is a single id
   form; the kernel stops accepting/normalizing the `mind.` spelling. See §8.

---

## 4. Retired ids

| Legacy id | Canonical id | Kind | Evidence |
| --- | --- | --- | --- |
| `mind.deepseek` | `deepseek` | catalog | DB + deployed catalog (already bare) |
| `mind.cerebras` | `cerebras` | catalog | same catalog de-prefix rule |
| `mind.groq` | `groq` | catalog | same |
| `mind.claude` | `claude` | catalog | same |
| `mind.local` | `local` | **built-in** | DB row `mind.local`; kernel-registered |
| `mind.ollama` | `ollama` | **built-in** | `mcp.rs:1503` exact-id special-case |

Only the two built-ins actually change spelling; catalog engines are already
bare. The migration (§6) therefore targets `mind.local` / `mind.ollama` rows.
Catalog engines never had a `mind.` row to rename.

`cerebras` / `groq` / `claude` are not in the deployed catalog at the moment
(only `deepseek` is). They are slated for re-upload to ClotoHub shortly and will
land with bare ids under the same rule, so the wizard keeps offering them
(§7) and no special handling is needed — a fresh install simply won't find them
until the catalog re-adds them.

---

## 5. Backend changes (Task #127)

**Built-in registration → bare.** Register the local / ollama engines as `local`
/ `ollama`. Update the hard-coded id checks:

- `crates/core/src/managers/mcp.rs:1503` — `id == "mind.ollama"` (post-connect
  `OLLAMA_MODEL` sync) → `"ollama"`.
- `crates/core/src/handlers/mcp.rs:1041` — `mind.local` reference.
- `crates/core/src/handlers/llm.rs` — the `mind.ollama` model relay comment/paths
  (`:64,108`).
- `crates/core/src/handlers/system.rs` tests referencing `mind.local` /
  `mind.deepseek` (`:4048,4061,4062,4074,4075,4085`).

**Delete the `mind.`-strip shims.** After unification these are no-ops (a bare id
`strip_prefix("mind.")` yields `None` → `unwrap_or(id)` returns the id unchanged),
so removing them is behaviour-preserving cleanup:

- `crates/core/src/managers/mcp.rs:2985`
- `crates/core/src/handlers/system.rs:1527` (grant-match `norm`)
- `crates/core/src/handlers/system.rs:2252` (engine-server resolution)
- `crates/core/src/handlers/system.rs:3061,3401` (LLM-proxy `provider_id`)

Each removal must be paired with a test asserting the bare id flows straight
through (the resolution / gate / provider-routing paths keep working with bare
ids only).

**Consensus config.** `CONSENSUS_ENGINES` is env-driven with no hard-coded
default (`crates/core/src/config.rs:308`), so there is no in-code default to
change; update the example strings in `crates/core/src/installer.rs:59` (and any
docs) from `mind.deepseek,mind.cerebras` to bare.

---

## 6. DB migration (Task #128)

A one-shot, idempotent rename modelled on the bug-388 repair
(`crates/core/src/db/mod.rs:281-366`, `repair_cpersona_rename_collision`).

**Targets** (all three storage sites for an engine id):

1. `mcp_servers.name` — `mind.local` → `local`, `mind.ollama` → `ollama`.
2. `mcp_access_control.server_id` — same rename on `server_grant` / `tool_grant`
   rows.
3. `agents.default_engine_id` — this is a **column** (confirmed
   `PRAGMA table_info(agents)`), not metadata JSON, so a direct `UPDATE`.

**Requirements:**

- **Idempotent** — running twice is a no-op (guard on the legacy row existing).
- **Collision-safe** — if both `mind.local` and `local` already exist (e.g. a
  partially-migrated DB), merge/prefer the bare row and drop the legacy one,
  exactly as the cpersona repair handles the `memory.cpersona` + `cpersona`
  collision.
- **Fresh-DB tolerant** — a database with no `mind.*` engine rows is unaffected.
- **CRLF** — any new `crates/core/migrations/*.sql` file MUST be converted to
  CRLF before the first `cargo run` / `tauri dev` (ClotoCore SQLx migration rule)
  or the checksum will mismatch and the kernel will refuse to boot.
- **Destructive-DB rule** — no preemptive `DELETE` / truncate of `.db` files; the
  migration only renames/merges specific rows.

Tests: apply-on-legacy, collision-merge, fresh-DB no-op — mirroring
`db/mod.rs:455-547`.

---

## 7. Frontend changes (Task #129)

- `dashboard/src/components/SetupWizard.tsx:40` `ENGINE_IDS` — catalog engines to
  bare (`cerebras`, `groq`, `deepseek`, `claude`), built-ins to bare (`local`,
  `ollama`); `:101` default `selectedEngine` `mind.deepseek` → `deepseek`.
- `dashboard/src/lib/presets.ts:21-24` `defaultEngine` — `mind.cerebras` /
  `mind.deepseek` → bare.
- `engineTKey` (`SetupWizard.tsx:63`, `engine_${id.replace('mind.', '')}`) is a
  no-op on a bare id (`deepseek` → `engine_deepseek`), so the i18n keys are
  unchanged. No `wizard.json` change required.
- `dashboard/src/lib/serverCategory.ts:35` — the `s.id.startsWith('mind.')`
  clause in `isEngineServer` becomes dead once ids are bare; it MAY be left as a
  harmless transitional fallback and removed after Phase 2, keeping the
  tool-surface test as the sole classifier.
- bug-425's console fix is already tool-surface based, so it needs no further
  change.

---

## 8. Compatibility

"De-facto retirement" means: the **ongoing** `mind.`-normalization is removed, but
a **one-shot** migration (§6) is mandatory — without it every existing
`mind.local` / `mind.ollama` grant would silently stop matching after the shims
are deleted. After the migration runs there is exactly one id spelling; the
kernel neither emits nor normalizes the `mind.` form. Downgrading to a
pre-migration build is not supported (standard for schema migrations).

---

## 9. Multi-repo Phase 2 (Task #130)

The built-in `local` / `ollama` engine servers live in `clotohub-servers` and
must advertise bare ids there too (the catalog is already bare for `deepseek`).
Sequence after ClotoCore Phase 1 lands: update the server repo, confirm the
`hub.cloto.dev` catalog, and ship via the companion phased-release process.
Tier A; push/PR only on explicit instruction.

---

## 10. Fixed points

1. Canonical engine id is **bare**; the `mind.` spelling is retired, not aliased.
2. Only built-ins (`local`, `ollama`) change spelling; catalog engines are
   already bare — the migration targets `mind.local` / `mind.ollama` only.
3. Classification stays tool-surface based (`isEngineServer` / `classify_tool`);
   no id prefix is consulted for categorisation.
4. The migration is idempotent, collision-safe, fresh-DB tolerant, CRLF-encoded,
   and never deletes a `.db` wholesale.
5. Shim removal is behaviour-preserving *given* unified ids — each removal is
   pinned by a bare-id passthrough test.
6. Full category-prefix retirement (`tool.`/`vision.`/`voice.`/`io.`) is **not**
   in scope — Goal #143.

---

## 11. Task mapping (Goal #142)

| Task | This doc |
| --- | --- |
| #126 [design] | this document |
| #127 [backend] | §5 |
| #128 [DB migration] | §6 |
| #129 [frontend] | §7 |
| #130 [Phase 2 / clotohub-servers] | §9 |
