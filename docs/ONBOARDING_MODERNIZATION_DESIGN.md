# Onboarding Modernization — Admin-Key Handover + Hub-Synced Presets

**Status**: Draft (design approved 2026-07-17)
**Related**: `DEFENDER_DESIGN.md` (the sudo-mode uninstall gate assumes the
user was handed the admin key — §2 here is its prerequisite),
`MCP_PLUGIN_ARCHITECTURE.md`, `docs/CATEGORY_PREFIX_RETIREMENT_DESIGN.md`

---

## 1. Motivation

Two independent findings, one root cause each:

**(a) The admin key is invisible to the human.** The desktop dashboard
authenticates silently — `getAutoApiKey()` (`lib/tauri.ts`) fetches the key
via Tauri invoke into sessionStorage on mount. The key exists and the *app*
can always use it, but the *user* has typically never seen it: on CLI installs
it scrolls past once; on desktop installs there is no display path at all.
Any feature that asks the user to present the admin key (the sudo-mode
uninstall gate in `DEFENDER_DESIGN.md` §7) would lock desktop users out.

**(b) Setup presets have drifted from the catalog.** `lib/presets.ts`
hardcodes server-ID arrays (`MINIMAL/STANDARD/ADVANCED/EXPERT_SERVERS`) and
per-preset `defaultEngine`. Audit on 2026-07-17 found the arrays referencing
`embedding` (the monorepo variant is deprecated — extracted to CEmbedding
2026-06-28) and `cpersona` (extracted to its own canonical repo; hub-catalog
status needs verification), while newer catalog entries (`cscheduler`, `groq`,
`ollama`, `claude`, `local`, `discord`) are absent from every preset. This is
the same defect class as the April 2026 incident (EXPERT preset referencing
the nonexistent `voice.tts`). Root cause: **the hardcoded preset list is a
second source of truth competing with the catalog**, and it goes stale on
every catalog change. Impact path is real: SetupWizard Step 5 feeds these IDs
straight into `/marketplace/batch-install`, so ghost IDs surface as silent
failures during a new user's first-run setup.

## 2. Admin-key handover

The fix for (a) is not key *generation* (the key already exists by wizard
time) but **handover** — the same grammar as 2FA recovery-code presentation:

### 2.1 Wizard step: "Administrator key"

A new step in the SetupWizard (before the final Quick Guide step):

```
This is your ClotoCore administrator key. You will be asked to
present it for critical operations (e.g. complete uninstall).

  [ ●●●●●●●●●●●● 👁 ]   [Copy]   [Regenerate]

  ☑ I have saved this key in a safe place        [Next]
```

- Masked by default; reveal toggle; copy button.
- **Regenerate** rotates the key server-side (invalidates the old one) — this
  also resolves the "key scrolled past during CLI install" case and gives
  users an owned rotation primitive. Regeneration rewrites `.env` and the
  webview's session copy atomically.
- The save-confirmation checkbox gates Next (deliberateness, not enforcement —
  the step is skippable via the wizard's existing skip path, because §2.2
  provides the permanent retrieval route).

### 2.2 Settings → Security: key display

`SecuritySection.tsx` gains a masked key row with reveal + copy + regenerate —
the permanent retrieval path referenced by the sudo-mode dialog ("where do I
find this key?"). Implementation is a few lines on top of the existing
`getAutoApiKey()`.

### 2.3 Honest security framing

Because the webview can programmatically read the key, displaying it adds no
new exposure, and requiring it in dialogs is a *deliberateness gate*, not a
security boundary (the boundary is the server-side `X-API-Key` check). Docs
and UI copy must not overclaim.

## 3. Hub-synced presets (collections)

### 3.1 Shape

Presets move from client-hardcoded ID arrays to a **`collections` block served
by the ClotoHub catalog** (first-party, hub.cloto.dev):

```json
"collections": [
  {
    "id": "standard",
    "icon": "layers",
    "servers": ["cron", "terminal", "websearch", "agent_utils"],
    "default_engine": "cerebras"
  }
]
```

Curation updates (new recommendations, ghost removal, engine defaults) then
propagate to all users **without a ClotoCore release** — removing the
structural cause of drift, which was curation being pinned to the client
release cycle.

### 3.2 Resolution chain (offline first-boot is a hard constraint)

The wizard runs immediately after install; the network may be down. Setup must
never brick on fetch failure:

1. Live hub catalog — existing kernel fetch (`marketplace.rs`, in-memory
   cache + `Stale` fallback already implemented)
2. Kernel's cached copy
3. **Bundled snapshot** — a catalog snapshot baked in at build time; the
   guaranteed floor

### 3.3 Structural ghost-ID elimination

- At render/install time the client intersects `collection.servers` with the
  IDs actually present in the resolved catalog — a ghost ID cannot be
  displayed or submitted; it drops out (with a dev-mode warning).
- The authoritative guard lives **hub-side in CI**: at publish time, validate
  `collections[*].servers ⊆ catalog IDs` (and `default_engine` ∈ catalog).
  Deterministic, runs where the data changes.

### 3.4 Curation policy (decided 2026-07-17)

- **Trust gating — adopted**: collections may only reference entries at or
  above a configured `trust_level`; the existing Magic Seal / trust
  infrastructure doubles as the recommendation quality gate.
- **Heavyweight-server exclusion — not reintroduced**: the `setup_default`
  field was deliberately removed in April 2026 and stays removed. Collections
  are positive curation (what to recommend), not exclusion semantics.
- `default_engine` per collection is hub-data too — newer engines (groq,
  ollama, claude, local) become recommendable without client changes.

### 3.5 Two consumption surfaces, one data source

The preset *definition* is already single-sourced (`lib/presets.ts`); the two
surfaces have distinct semantics that are preserved:

| Surface | Semantics |
| --- | --- |
| SetupWizard (steps 4–5) | what to **install** (+ grant) — batch-install |
| ServerAccessSection (agent config) | what to **grant** to an agent — server_grant set; `detectPreset` highlights the active match |

Both consume the same resolved collections. `detectPreset`'s exact-set
matching is kept (its "no match on any extra grant" behavior is the honest
answer and unchanged by this design).

## 4. Immediate hotfix (does not wait for §3)

Ghost IDs in `lib/presets.ts` are a live first-run defect and get fixed
directly against the current hub catalog (verify `embedding` / `cpersona`
status against what the kernel actually fetches, remove or replace, confirm
batch-install path has no silent failure).

## 5. Phasing

| Phase | Deliverable |
| --- | --- |
| 0 (hotfix) | §4 — presets.ts vs live catalog reconciliation |
| 1 | §2 — wizard admin-key step + SecuritySection key display (+ regenerate endpoint) |
| 2 | §3 — hub `collections` block + hub CI validation + client resolution chain + bundled snapshot |

Phase 1 unblocks `DEFENDER_DESIGN.md` Phase 3 (sudo-mode gate); Phase 2 is
independent of the defender line.

## 6. Open questions

- Bundled-snapshot refresh policy (every release build vs pinned + CI staleness
  alarm).
- Whether `Regenerate` requires current-key confirmation when invoked outside
  the wizard (Settings path) — leaning yes, same sudo-mode grammar.
- Hub `collections` versioning if client-side schema evolves (start additive,
  `collections_version` field reserved).
