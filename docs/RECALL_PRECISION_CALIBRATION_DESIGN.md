# Recall Precision: Calibrate the Curve, Expose One Knob (Design)

> **Status:** Design, approved 2026-06-26. Successor to
> [`RECALL_CONTAMINATION_AB_2026-06-14.md`](RECALL_CONTAMINATION_AB_2026-06-14.md).
> **Scope:** CPersona recall pipeline (`memory_handlers.py`, `vector.py`,
> `admin_handlers.py`) + ClotoCore per-agent recall config.

## 1. Summary

The CPersona v2.4.24/25 auto-calibration fix (vector-similarity threshold
`0.30 → 0.593`, separation method) corrected the **wrong gate for the recall modes
production actually runs**. Measured on a local bge-m3 fp32 reproduction (249-memory
corpus), the calibrated threshold is **inert in `rsf` and `rrf`** and changes recall
output **only in `cascade`**. The contamination that v2.4.24/25 resolved in production
was reduced by the `rrf → rsf` switch and the FTS-CJK fix (bug-002), not by the
threshold calibration.

The fix forward is a single principle:

> **Calibrate the curve (data-derived). Expose the point (one policy knob).**

- The mapping from *score space* to *relevance space* — the operating **curve** — is
  determined mechanically by the corpus and the embedding model. It must be
  auto-calibrated, with no human-set constants.
- *Where on the curve to operate* — the precision-vs-recall trade-off — is a
  use-case value judgement. It is the one irreducible human input and should be a
  single explicit knob, not a buried magic constant.

## 2. The finding: why the calibrate fix is inert in fusion modes

Two gates determine precision in the fusion modes (`rsf`, `rrf`), and the calibrated
vector threshold drives **neither**:

| Gate | Current basis | Calibrated threshold used? |
| --- | --- | --- |
| ① Pre-fusion vector floor | `_get_vector_threshold(agent) × RRF_THRESHOLD_FACTOR` (`0.5`) | Scaled below the bge-m3 null mean (~0.51) → does not bite |
| ② Post-fusion quality gate | `_adaptive_min_score(memory_count)` — a pool-size heuristic (`0.5 − t·0.3`, t = log(N+1)/log(500); ≈0.23 at N=255) | **No** — independent of calibration |

Mechanism for ①: on anisotropic bge-m3, almost every pair scores ~0.51, so a floor of
`0.30×0.5 = 0.15` or `0.593×0.5 = 0.296` admits essentially the same candidates; and
`_search_vector` returns the top-`limit` by cosine regardless, so the floor never
trims the returned set. Only the **full** threshold (`cascade`, no `×0.5`) sits above
the null mean and bites.

### 2.1 Experimental evidence (local bge-m3 fp32, 249→255-memory corpus)

`Arm A` = stale floor 0.30; `Arm B` = production-calibrated 0.593. Off-topic
admissions / recall size, per recall mode:

| Mode | Arm A | Arm B | Discriminates? |
| --- | --- | --- | --- |
| `cascade` (full threshold) | 2 broad / 20 off-kw | 3 / 5 | **yes** |
| `rsf` (×0.5 scaled) | 0 / 5 | 0 / 5 | no (identical) |
| `rrf` (×0.5 scaled) | 2 / 20 | 2 / 20 | no (identical) |

Setting `RRF_THRESHOLD_FACTOR = 1.0` makes `rsf` discriminate (off/rec `5/42 → 2/15`),
confirming the `0.5` scaling is precisely what neuters calibration in the fusion modes.

A separate axis result: **`rsf` is materially cleaner than `rrf`** (0 broad / 5 off-kw
vs 2 / 20). RSF preserves score margins so the keyword channel down-ranks contaminants
— this is the real contamination reducer, independent of the threshold.

> **Caveat on the local calibration.** Separation calibration on this corpus returned
> `0.483` (below the null mean) because all 249 memories were planted with near-identical
> timestamps, degenerating the temporal-adjacency positive proxy. Arm B therefore used
> the production-observed `0.593`. A faithful local calibration needs realistic temporal
> structure.

## 3. The principle and the three orthogonal knobs

Recall configurability has **three orthogonal axes**. Two are use-case knobs; the
third was a hidden magic constant and is split into a calibrated curve plus one
knob:

| Axis | Determined by | Treatment | Origin |
| --- | --- | --- | --- |
| **Timing** — when to recall (always / session_start / +active / manual) | use-case | knob 1 | Per-agent recall knobs |
| **Session scope** — boundary (channel / per_user / thread) | deployment | knob 2 | Per-agent recall knobs |
| **Precision curve** — score↔relevance map (the ROC) | corpus + embedding model | **calibrate (no human input)** | This design |
| **Precision point** — where on the curve (precision↔recall weight) | use-case value judgement | **knob 3 (new)** | This design |

This split is the same data-vs-policy doctrine the project applied when it removed
effort-routing / escalation from the CScheduler data layer: *the curve is data and must
be derived; the operating point is policy and must be an explicit, single choice.*

## 4. Reframe: timing is not the contamination lever

[`RECALL_CONTAMINATION_AB_2026-06-14.md`](RECALL_CONTAMINATION_AB_2026-06-14.md) §4.3
already concluded the root cause is recall **precision, not timing**, and §4.2 showed
per-turn recall is also *corrective grounding*. This design makes the ownership
explicit:

- **Timing (knob 1)** is a cost / latency / corrective-grounding lever — **not** the
  contamination fix. Gating recall to session-start regressed drift on DeepSeek
  (+28.5 pp); it does not belong to contamination.
- **Precision (knob 3 + the calibrated curve)** owns contamination. It lives in the
  CPersona memory capability (where relevance interpretation belongs per
  ARCHITECTURE §1.4), not in kernel-side recall orchestration.

The Discord-recall redesign branch (`feat/discord-recall-gating`) therefore stays
"measured, not landing" as a contamination fix; its Phase-1 bridge change may stand on
its own cost/UX merits.

## 5. The calibration work

Apply the existing separation methodology (null distribution vs. a temporal-adjacency
positive proxy, Youden-J operating point — already in `admin_handlers.do_calibrate_threshold`)
to **two** distributions instead of one:

- **(a) cosine distribution → vector floor.** Already calibrated (used directly in
  `cascade`; scaled by `RRF_THRESHOLD_FACTOR` in fusion modes).
- **(b) NEW: fused-score distribution → post-fusion quality gate.** Replace or augment
  `_adaptive_min_score(memory_count)` with a separation-calibrated threshold computed on
  the `_rsf_score` / `_rrf_score` distribution (null pairs vs. positive-proxy pairs).
  The fused score is already normalised to the cosine `[0,1]` scale (divided by
  `n_active`), so the same machinery applies.

With (b) in place, calibration drives precision in **every** mode: `cascade` via the
vector floor, `rsf`/`rrf` via the post-fusion gate. The pre-fusion floor can stay
permissive (its job is candidate *recall*, not precision), so `RRF_THRESHOLD_FACTOR`
need not change — though see §7.

Further magic constants (`RRF_K`, `AUTOCUT_MIN_GAP_RATIO`, `AUTOCUT_MIN_RESULTS`,
episode-penalty rates) are candidates for the same separation treatment over time.

### 5.1 The one knob: precision point (knob 3)

Calibration yields a curve; Youden-J marks the *balanced* operating point. Knob 3 is a
single per-agent scalar β that shifts the point on the **same** curve and maps with one
meaning onto **both** calibrated gates:

- `strict` — higher specificity (β > 1, or a target false-positive rate). Minimises
  contamination; accepts more misses.
- `balanced` (default) — Youden-J.
- `lenient` — higher sensitivity. Minimises misses; accepts more contamination.

Concretely: pick the operating point that maximises `sensitivity + β·specificity`, or
the point at a target FPR. β is the *only* value the system cannot derive from data —
it encodes the use-case's relative cost of a contaminant vs. a missed memory.

## 6. Placement

- **Calibration layer — CPersona.** Per-agent, keyed by embedding dimension,
  recalibrated on model/corpus change at startup (extends the v2.4.24 sidecar +
  startup-guard mechanism to gate (b)).
- **Knobs — per-agent config.** knob 1/2 and knob 3 as agent metadata,
  surfaced in the Dashboard agent-config UI (deferred-save pattern). knob 3 spelled
  e.g. `recall_precision: strict | balanced | lenient` (or a raw β).

## 7. Open questions

1. **Production `RRF_THRESHOLD_FACTOR`.** Unconfirmed (remote host unreachable at
   design time). If `0.5` (code default), gate (b) is the sole precision driver in
   `rsf` and (a) is decorative there; if `1.0`, both gates carry knob 3. Verify with
   `systemctl show cpersona -p Environment`.
2. **Positive-proxy quality / cold start.** Temporal-adjacency degenerates without real
   time structure (see §2.1). Define a fallback (fixed default + minimum sample size)
   for sparse / fresh agents, and consider a stronger proxy (e.g. LLM-judged pairs).
3. **β → operating-point formula.** Settle the exact mapping (weighted-J vs. target-FPR)
   and the discrete-label → β values.
4. **Whether to also raise `RRF_THRESHOLD_FACTOR`.** If knob 3 should move the pre-fusion
   floor too (not only the post-fusion gate), the `0.5` scaling needs revisiting.

## 8. Relationship to existing tracks

- **Recall redesign v1**: closed on the contamination axis; redesign held,
  not landing. This design relocates contamination ownership to precision.
- **Per-agent recall knobs**: extended from 2 knobs to 3 (+ calibrate
  layer); blocked by this design.
- **This design (the calibration work)**: the curve-calibration that must land
  before knob 3 can offer a meaningful operating point.
