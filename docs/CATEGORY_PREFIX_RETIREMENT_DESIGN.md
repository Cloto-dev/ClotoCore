# Category-Prefix Retirement (`tool.` / `vision.` / `voice.` / `io.` / `output.`) — Design

**Status:** Proposed
**Scope:** an earlier decision · an earlier decision
**Author:** kernel team · 2026-07-04
**Predecessor / template:** `docs/MIND_PREFIX_RETIREMENT_DESIGN.md` (an earlier decision, shipped
2026-07-01 as ClotoCore PR #247 + clotohub-servers PR #30). This document completes the
programme started there: retire **every** remaining category prefix so a server has exactly
one canonical **bare** id, and classification is derived exclusively from the tool surface.

---

## 1. Problem & current state

After the `mind.` retirement the id space is still split, now along the category axis:

- **The deployed hub catalog is fully bare.** All 10 published connectors on
  `https://hub.cloto.dev/api/catalog` (the kernel's default marketplace source,
  `handlers/marketplace.rs DEFAULT_CATALOG_URL`) carry bare ids: `terminal`, `websearch`,
  `cscheduler`, `cpersona`, plus the six engines. Marketplace install registers the catalog
  id **verbatim** (`marketplace.rs:473`), so every hub-installed server lands bare.
- **The static registry (`clotohub-servers/registry.json`) still declares 12 prefixed ids**
  (`tool.terminal`, `tool.agent_utils`, `tool.cron`, `tool.embedding`, `tool.websearch`,
  `tool.imagegen`, `tool.cscheduler`, `vision.capture`, `vision.gaze_webcam`, `voice.stt`,
  `output.avatar`, `io.discord`), while the per-server manifests
  (`pyproject.toml [tool.cloto.mgp].id`) are already bare.
- **Existing databases hold both spellings.** A representative dev DB contains nine
  prefixed `mcp_servers` rows (`tool.agent_utils`, `tool.cron`, `tool.embedding`,
  `tool.imagegen`, `tool.research`, `vision.capture`, `vision.gaze_webcam`, `voice.stt`,
  `io.discord`) with 17 grants keyed to them — **and** bare twins (`terminal`,
  `websearch`, `cscheduler`) installed later through the hub. This is exactly the
  duality that produced bug-425 for engines, reproduced at category scale.
- **The Setup Wizard installs prefixed ids against a bare catalog.**
  `SetupWizard.tsx ALL_SELECTABLE_SERVER_IDS` batch-installs `tool.terminal`,
  `vision.capture`, `voice.stt`, … — none of which exist in the deployed catalog. Those
  wizard entries are silently dead today; bare-ing them is a fix, not merely hygiene.

### The prefix is already redundant (and partly dead)

- The kernel classifier (`capability_dispatcher.rs::classify_tool`) has prefix arms for
  `vision.` / `stt.` / `output.` — but the `stt.` arm matches **no real server** (the real
  id is `voice.stt`), so STT classification already runs on the `transcribe` tool-name
  fallback in production. `tool.` and `io.` have **no** classifier arm anywhere: they were
  never consulted for behavior.
- Search categories and latency tiers (`mcp_tool_discovery.rs`) key on the **last**
  dot-segment (`split('.').next_back()`), so `tool.terminal` and `terminal` already behave
  identically there.
- The frontend classifies by tool surface (`serverCategory.ts`) since bug-425.

---

## 2. Root cause

Same as the `mind.` case: one server, two spellings, half-normalized consumers. The fix is
to collapse to the bare spelling everywhere and delete the prefix consultation, not to add
more normalization.

---

## 3. Design overview

1. **Canonical id = bare = the server's directory / hub-connector id.** This is the id the
   catalog already serves and the id `servers/<id>` installs under.
2. **One-shot DB migration** de-prefixes existing rows and grants across the two storage
   sites, merging into bare twins where they exist (§6).
3. **Delete every prefix classifier arm** — kernel `classify_tool` loses all five prefix
   arms (`memory.` / `mind.` / `vision.` / `stt.` / `output.`); the frontend
   `serverCategory.ts` loses its `mind.` / `memory.` arms. Tool-surface classification
   becomes the **sole** mechanism (§5, §7).
4. **Normalize legacy config ingestion** so an old `mcp.toml(.migrated)` cannot resurrect
   prefixed rows after the migration (§5).
5. **Producers write bare** — registry.json, Setup Wizard, presets, docs (§7, §8).
6. **Compat is one-shot.** After migration there is a single spelling; the kernel neither
   emits nor normalizes prefixed forms (§9).

---

## 4. Retired ids

| Legacy id | Canonical id | Notes |
| --- | --- | --- |
| `tool.terminal` | `terminal` | bare twin already installed via hub → **merge** |
| `tool.websearch` | `websearch` | bare twin already installed via hub → **merge** |
| `tool.cscheduler` | `cscheduler` | bare twin already installed via hub → **merge** |
| `tool.cron` | `cron` | |
| `tool.embedding` | `embedding` | |
| `tool.imagegen` | `imagegen` | |
| `tool.agent_utils` | `agent_utils` | manifest currently says `agent-utils` — fixed to match (§8) |
| `tool.research` | `research` | legacy orphan (absent from registry.json); generic strip covers it |
| `vision.capture` | `capture` | |
| `vision.gaze_webcam` | `gaze` | **explicit mapping** — see below |
| `voice.stt` | `stt` | |
| `voice.tts` | — (dropped) | wizard-only phantom: no registry entry, no server, no catalog row |
| `output.avatar` | `avatar` | |
| `io.discord` | `discord` | |

> **Amendment (2026-07-05) — connector id charset.** The `agent_utils`
> decision below collided with a constraint this document missed: the
> connector v1 schema restricted ids to kebab-case
> (`[a-z0-9]([a-z0-9-]*[a-z0-9])?`, MGP_CONNECTOR.md §3), so the snake_case
> canonical id was valid host-side but rejected at hub import. Resolved
> spec-side (option A, approved 2026-07-05): the spec charset was widened to
> `[a-z0-9]([a-z0-9_-]*[a-z0-9])?` with kebab-case as the RECOMMENDED style
> (MGP_CONNECTOR.md §3.3, mgp-sdk v0.6.1) — the id-unification doctrine of
> this document requires the host and wire charsets to be congruent, and
> hyphens carry the inverse host-side hazard (illegal in derived env-var
> names such as `{ID}_MODEL`). **Rule going forward: a canonical server id
> must satisfy the §3.3 charset — which every `servers/<id>` directory name
> that follows host conventions already does.**

**`gaze` naming decision.** The gaze server today has a four-way name spread:
registry id `vision.gaze_webcam`, directory `servers/gaze`, manifest id `gaze-webcam`,
future hub connector derived from the manifest. A mechanical prefix strip would mint a
fifth spelling (`gaze_webcam`). The canonical id is **`gaze`** — the only value consistent
with the install directory and the `servers/<id>` layout every other connector follows.
The DB migration maps `vision.gaze_webcam → gaze` as an explicit special case, and the
manifest id is corrected to `gaze` (§8). Same rule fixes `agent-utils → agent_utils`
(canonical follows the directory's snake_case, keeping ids shell/env-safe).

**`voice.tts` disposition.** It appears only in `SetupWizard.tsx`
(`ALL_SELECTABLE_SERVER_IDS`, `MANUAL_START_SERVERS`); there is no such server, registry
entry, or catalog row (TTS ships inside the `avatar` server via VOICEVOX). The wizard
entry is removed rather than renamed — installing it can never succeed.

---

## 5. Backend changes (an earlier decision)

**Delete all five prefix arms in `classify_tool`**
(`managers/capability_dispatcher.rs:314-329`). Verified fallback coverage — each affected
category's real server advertises the fallback tool name (checked in
`clotohub-servers` sources, 2026-07-04):

| Category | Deleted arm | Fallback arm | Real tool on the bare server |
| --- | --- | --- | --- |
| Vision | `vision.` | `analyze_image \| capture_screenshot` | `capture` exposes `analyze_image` ✓ |
| Stt | `stt.` (already dead — real id was `voice.stt`) | `transcribe` | `stt` exposes `transcribe` ✓ |
| Speech | `output.` | `speak` | `avatar` exposes `speak` ✓ |
| Reasoning | `mind.` (ids already bare) | `think \| think_with_tools` | ✓ (an earlier decision) |
| Memory | `memory.` (ids already bare) | `store \| recall \| …` | ✓ (bug-388) |

Two adjustments while touching the fallback table:

- **Add `capture_screen` to the Vision arm.** The existing `capture_screenshot` literal
  matches no real tool (the capture server's tool is `capture_screen`); the literal is
  kept for compatibility and the real name added, so screenshot capability classifies.
- **`gaze` intentionally becomes unclassified.** Under the `vision.` arm its tools
  (`start_tracking`, …) were nominally Vision, but nothing could resolve them via the
  capability index (no `analyze_image` surface), so dropping the classification is
  behavior-preserving. Gaze remains an ordinary tool server.

**Normalize legacy config ingestion (resurrection guard).**
`migrate_config_file_to_db` / `repair_config_loaded_servers` (`managers/mcp.rs:247,336`)
re-read `mcp.toml` / `mcp.toml.migrated` whenever `command = 'config-loaded'` placeholder
rows exist, and upsert servers **under the id spelled in the file**. On an upgraded
install whose `.migrated` file predates this change, that path would re-insert
`tool.terminal` etc. *after* the §6 migration ran. Fix: strip the legacy category
prefixes (and apply the `vision.gaze_webcam → gaze` mapping) at this ingestion boundary.
This is the config-file analog of the DB migration — a one-shot legacy normalizer at the
only place old spellings can still enter, not an ongoing alias.

**No change needed (verified):**

- `mcp_tool_discovery.rs` `extract_categories` / `classify_latency_tier` — both key on the
  last dot-segment, identical for prefixed and bare ids.
- `validate_server_name` (`handlers/mcp.rs:357`) — accepts both dotted and bare ids;
  only its doc-comment example is refreshed.
- Inbound notification routing — the kernel stamps `McpNotification.server_id` from the
  **connection identity** (`managers/mcp_client.rs:341`, consumed at `lib.rs:920`), never
  from the payload, so the Discord server's self-declared `server_id` param (§8) is
  informational, not a kernel routing key.
- Marketplace install — registers the catalog id verbatim; the catalog is already bare.

**Comment/doc refresh:** `handlers/mcp.rs:360` (naming-convention example),
`handlers/agents.rs:440`, `handlers/system.rs:3092` (`vision.capture` references).

**Tests.** classify_tool unit tests gain bare-id cases per category (capture/stt/avatar
tool surfaces) and drop/convert the prefix-arm fixtures; config-ingestion normalization
gets a unit test (prefixed `mcp.toml` entry lands bare); migration integration tests per
§6. Existing test fixtures with arbitrary dotted ids (`tool.optin`, `tool.foo`,
`output.coqui`, …) stay — dots in ids remain legal; only their *classification* semantics
change, and those fixtures rely on tool names already.

---

## 6. DB migration (an earlier decision)

`migrations/20260704*_retire_category_prefixes.sql`, same detach → rename → re-attach
shape as `20260701120000_retire_mind_prefix.sql` (FK-safe at every statement boundary,
transaction/autocommit agnostic).

**Rename rule:** strip everything up to and including the first `.` for ids matching
`tool.%` / `vision.%` / `voice.%` / `io.%` / `output.%`, **except** the explicit mapping
`vision.gaze_webcam → gaze`.

**Targets:**

1. `mcp_servers.name` — the nine-ish prefixed rows. `OR IGNORE` + delete-leftover handles
   the **bare-twin merge** (`tool.terminal` next to hub-installed `terminal`: the bare row
   — usually the newer, hub-sealed install — wins; the prefixed row is dropped).
2. `mcp_access_control.server_id` — every grant under a prefixed id, de-prefixed and
   deduped against existing bare grants (same `NOT EXISTS` guard as the mind migration).
3. **Residue sweep (gap left by an earlier decision):** `cron_jobs.engine_id` stores engine ids but
   was not covered by the mind migration — de-prefix any lingering `mind.%` values here.
   (`agents.default_engine_id` needs no category pass: it only ever held engine ids.)

**Requirements** (unchanged from the template): idempotent; collision-safe (bare twin
merge); fresh-DB tolerant; **CRLF** before first `cargo run` / `tauri dev`; never deletes
a `.db` or truncates a table. The three historical seed migrations that wrote `tool.*`
ids (`20260304200000`, `20260304200001`, `20260309100000`) are frozen history — applied
checksums must not change; this migration renames what they seeded.

**Tests:** apply-on-legacy, bare-twin merge (terminal case), gaze explicit mapping,
grant dedup, cron_jobs residue, fresh-DB no-op, idempotency — in-memory SQLite with
`foreign_keys=ON`, mirroring the mind migration's suite.

---

## 7. Frontend changes (an earlier decision)

- `lib/presets.ts:7-13` — all four preset arrays to bare ids.
- `components/SetupWizard.tsx` — `ALL_SELECTABLE_SERVER_IDS` to bare, `voice.tts` entry
  **removed** (§4); `MANUAL_START_SERVERS` set to bare (`gaze`, `capture`, `imagegen`,
  `stt` — `voice.tts` removed); `serverTKey` (`server_${id.replace('.', '_')}`) keeps its
  shape but now produces `server_terminal` instead of `server_tool_terminal` → the wizard
  i18n keys in `en`/`ja` locale bundles are **renamed in lockstep**.
- `pages/McpServersPage.tsx:49-67` — the prefix → sort-order map dies with the prefixes.
  Replacement ordering: engines first, memory second (both via the tool-surface
  `serverCategory.ts` tests), then everything else alphabetically. This preserves the
  intent (engines/memory float to the top) without any id inspection.
- `vrm/VrmViewerPage.tsx:153` — the avatar channel gate `serverId === 'output.avatar'`
  → `'avatar'` (plus the two comments).
- `components/AgentConsole.tsx:362` — the engine-event gate
  `engine_id?.startsWith('mind.')` is **already broken** for bare engines; replace the
  prefix test with presence of `engine_id` (an event carrying `engine_id` is an engine
  event by construction).
- `lib/serverCategory.ts:35,41` — drop the `mind.` / `memory.` prefix arms; tool-surface
  becomes the sole classifier (the transitional fallback the mind design deferred to
  "after Phase 2" — this is that removal). Same for the `mind.` fallbacks in
  `presets.ts:42` and `AgentPluginWorkspace.tsx:151` (a server absent from the installed
  list can no longer be presumed an engine by spelling; `detectPreset` compares against
  the bare arrays instead).
- `lib/format.ts displayServerId` — unchanged (no-op on bare ids; still useful for any
  third-party dotted id).
- Tests: `serverCategory.test.ts` prefix-arm cases convert to tool-surface cases;
  `format.test.ts` fixtures stay valid.

---

## 8. Multi-repo Phase 2 — clotohub-servers (an earlier decision)

Following the PR #30 pattern (edit the `id` value only; `category` / `directory` fields
untouched):

- `registry.json` — the 12 prefixed ids → bare per §4 (`vision.gaze_webcam` → `gaze`).
- `servers/discord/src/main.rs:821,836,849` — the three self-declared
  `"server_id": "io.discord"` literals in `mgp.lifecycle` notification params → `discord`
  (informational for event-bus subscribers; kernel routing is connection-identity based,
  §5) + the module doc comment (`main.rs:4`).
- `servers/avatar/src/main.rs:4` — doc comment only.
- `servers/gaze/server.py:25` — `ToolRegistry("vision.gaze_webcam")` → `"gaze"` (this is
  the MCP `serverInfo.name`); `servers/gaze/pyproject.toml` id `gaze-webcam` → `gaze`.
- `servers/agent_utils/pyproject.toml` id `agent-utils` → `agent_utils`.
- `tools/bench/cpersona_bench_setup.py:229` — `tool.embedding` → `embedding` (the grant
  id PR #30 deliberately deferred).
- `README.md` catalog table + `mcp.toml` example; `servers/example/server.py` docstring;
  `docs/DISCORD_CONTEXT_SEARCH_DESIGN.md` references to the two real ids. The dated
  changelog line in `tools/bench/README.md` is preserved as history (PR #30 precedent).
- Version bumps for the servers whose shipped bytes change (discord, gaze, agent_utils),
  after their checks are green (verify-then-bump rule).

Unpublished hub connector rows created under the old manifest ids (`gaze-webcam`,
`agent-utils`) are inert — they are not in the catalog; the corrected ids take effect on
the next import/publish.

---

## 9. Compatibility & sequencing

- **Repo coupling is loose.** The kernel migration + code and the clotohub-servers flip
  can land in either order: kernel routing never trusts payload ids (§5), the catalog is
  already bare, and registry.json's prefixed ids only matter for future installs from
  that static file. An old Discord binary running against a migrated kernel keeps
  working; only the informational `server_id` field inside its lifecycle params is stale
  until it is updated.
- **Old wizard / new catalog:** already broken today (prefixed install ids vs bare
  catalog); this change fixes the pairs that exist in the catalog (`terminal`,
  `websearch`) and leaves not-yet-published connectors exactly as reachable as before.
- **Downgrade** to a pre-migration build is not supported (standard for schema
  migrations).
- After this change the kernel neither emits nor consults any category prefix. Dotted ids
  remain *legal* (third-party servers may use any name); they simply carry no semantics.

---

## 10. Fixed points

1. Canonical server id is **bare** and equals the `servers/<id>` directory / hub connector
   id; category prefixes are retired, not aliased.
2. `vision.gaze_webcam → gaze` is an explicit mapping, not a mechanical strip; the gaze
   and agent_utils manifests are corrected to their directory names.
3. Classification is tool-surface only: `classify_tool` and `serverCategory.ts` keep **no**
   prefix arm. (Exception unchanged from an earlier decision: `is_engine_server`'s `mind.` shortcut
   in `handlers/system.rs` stays as offline-grant back-compat — it classifies grants, not
   servers, and tool-surface needs a live connection.)
4. The migration is idempotent, collision-safe (bare-twin merge), fresh-DB tolerant,
   CRLF-encoded, and never deletes a `.db` wholesale; frozen seed migrations are not
   edited.
5. Legacy config ingestion (`mcp.toml(.migrated)`) normalizes prefixed ids at the
   boundary, so no post-migration path can reintroduce a prefixed row.
6. `voice.tts` is removed, not renamed — there is no such server to install.

---

## 11. Task mapping (an earlier decision)

| Task | This doc |
| --- | --- |
| #151 [design] | this document |
| #152 [backend] | §5 |
| #153 [DB migration] | §6 |
| #154 [frontend] | §7 |
| #155 [Phase 2 / clotohub-servers] | §8 |
