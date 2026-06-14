# CPersona Recall Contamination — Redesign AB Test Report (2026-06-14)

> **Status:** Measurement complete. The Discord-recall redesign (session-start-gated
> recall + per-channel episodic loop), measured same-conditions against the pre-redesign
> baseline on DeepSeek, **regresses** topic-drift contamination (+28.5 pp severe).
> Root cause is unchanged from the [2026-04-24 report](RECALL_CONTAMINATION_AB_2026-04-24.md)
> §4.4: recall **precision**, not recall **timing**.
>
> **⚠️ CORRECTION — see §9 (added later the same day).** §2–§6 below were written
> while CPersona embeddings were **off** (mode=none) — so the §3 numbers are an
> FTS-only path, and the §6.2 "improve cpersona precision (RSF / MMR / gap filter)"
> recommendation is **superseded**. Follow-up investigation (with embeddings enabled,
> bge-m3) found the real root cause is **two concrete bugs** (env-key mismatch +
> FTS whitespace-split); cosine-magnitude techniques (RSF/hybrid/gap) do **not** fix
> it. §9 is authoritative for the conclusion and next steps.
> **Scope:** ClotoCore kernel recall path (`handlers/system.rs`) + CPersona recall pipeline.
> **Successor to:** [`RECALL_CONTAMINATION_AB_2026-04-24.md`](RECALL_CONTAMINATION_AB_2026-04-24.md)
> (which found ~23% severe drift and identified the cosine-distribution root cause).

## 1. Background

### 1.1 What the redesign changed

The Discord-recall redesign (CSC Goal #119) hypothesised that **per-turn long-term
recall** is the contamination source, and gated it. The branches under test:

- **ClotoCore** `feat/discord-recall-gating` — Phase 2a: the kernel pulls long-term
  recall only on the **first turn** of a bridge session (`is_session_start =
  snapshot_transcript(session_key).is_empty()`); continuing turns skip auto-recall.
  Plus Phase 2b transcript bound, Phase 3a episode archival on Cold eviction, and
  Phase 3c session-start episode grounding (`channel`-scoped recall, `[Episode]`-prefix
  filtered).
- **clotohub-servers** `feat/discord-per-channel-session` — Phase 1 (1 channel = 1
  session) + Phase 4 operator-configurable recall discipline.
- **cpersona** `feat/episode-channel-scoping` — `episodes.channel` column.

### 1.2 Premise under test

> "Per-turn recall injects recalled memories as chat-turns every turn and drives
> topic-drift; gating recall to session-start will reduce contamination."

This report tests that premise empirically and finds it **inverted** on DeepSeek.

## 2. AB Test Methodology

### 2.1 Fixed inputs

- **Agent**: `agent.cpersona_bench` (dedicated bench agent; production agents untouched)
- **Engine**: `deepseek` (DeepSeek API via the `deepseek` MCP connector)
- **Memory**: `cpersona` (flat install, catalog version), embedding server up
- **Corpus**: the 19-memory `seed_corpus.json` (pan/croissant breakfast → raspberry
  pie → unrelated topics), the same contamination setup as 2026-04-24
- **Query set**: 14 queries / 6 categories (`scripts/recall_ab/query_set.json`),
  identical to 2026-04-24 (`この前のパンの話覚えてる?` … `週末の予定`)
- **N = 3 trials per query per arm**, each trial an **accumulating** session (the
  redesign's gate only acts on continuing turns, so a multi-turn session is required)
- **Common baseline**: a clean post-seed (19-memory) snapshot of the kernel's
  CPersona DB, restored before **both** arms via file snapshot/restore — never a
  destructive delete (`scripts/recall_ab/README.md`)

### 2.2 Arms

| Arm | Kernel build | Recall behaviour |
|---|---|---|
| `old` | `master` @ 4afb050 (bug-395 seal fix only) | per-turn recall (pre-redesign) |
| `new` | `feat/discord-recall-gating` (rebased on master) | session-start-gated recall + Phase 3 grounding |

The only variable is the kernel build. Engine, corpus, query set, classifier, and
starting memory state are identical. (`master` carries the bug-395 seal fix so the
`deepseek` connector verifies and connects under both arms.)

### 2.3 Metric

`scripts/recall_ab/run_ab.py` classifies each response via the §2.3 rubric of the
2026-04-24 report (heuristic): severe = unrelated-topic keyword present + elaboration;
mild = present but disclaimed; coherent = otherwise. `severe%` is severe ÷
(non-timeout, non-error) trials. The harness performs no destructive DB operations.

### 2.4 Methodology caveat — a corrected first pass (bug-396)

The first measurement pass reported a **false 0% drift for both arms**. Root cause:
the bench agent's `default_engine_id` was `mind.deepseek`, but the connector installs
under the ClotoHub catalog id `deepseek`, and the kernel resolves engines by **exact**
server-name match (`run_agentic_loop`: `if mcp.has_server(engine_id).await`,
`handlers/system.rs` ~L1369, no `mind.` alias). `has_server("mind.deepseek")` is false,
so every turn returned the literal string `"[Error] Engine 'mind.deepseek' not found"`
as its content. That string contains no contaminant keyword, so the heuristic
classifier scored all 42 turns `coherent` — a spurious 0%. This is filed as **bug-396**
(`qa/issue-registry.json`). The measurement below was re-run after repointing the bench
agent to `default_engine_id=deepseek`. **Lesson: the harness should treat an
`[Error]`/engine-error response as `error`, not `coherent`.**

## 3. Results

### 3.1 Aggregate (14 queries × 3 trials = 42 per arm, same clean-19 baseline)

| Arm | Coherent | Mild | Severe | Timeout | Error | Sev%/Completed |
|---|---|---|---|---|---|---|
| `old` (per-turn recall) | 21 | 9 | 12 | 0 | 0 | **28.6%** |
| `new` (session-start gated) | 9 | 9 | 24 | 0 | 0 | **57.1%** |

**Delta (new − old): +28.5 pp — a regression.** No completion-rate cost (0 timeout /
0 error in both arms), so this is not the L2 timeout-spike failure mode of 2026-04-24;
it is a pure drift increase. The `old` arm's 28.6% reproduces the 2026-04-24 baseline
(~23%) closely, validating the harness.

### 3.2 Per-query × arm (3 trials per cell)

`C` = coherent, `m` = mild, `S` = severe.

| Query | `old` | `new` |
|---|---|---|
| A1 `この前のパンの話覚えてる?` | CCS | SCS |
| A2 `昨日話したパンの件、どうなった?` | CmS | CSS |
| B3 `ラズベリーパイについて覚えてる?` | CCC | SSS |
| B4 `Raspberry Pi って何?` | SmS | mmS |
| C5 `パン` | CSm | SSS |
| C6 `朝食` | CSS | SCS |
| C7 `ラズベリーパイ` | CCC | mSS |
| D8 `昨日何話してたっけ` | SmS | SSS |
| D9 `このセッションで何話した?` | mmS | mSS |
| D10 `私の好きな食べ物は?` | mSS | mmC |
| E11 `git push の件` | CCC | CCS |
| E12 `Discord の話` | CCC | CCS |
| F13 `今日の天気` | CCC | SmC |
| F14 `週末の予定` | mmC | mmS |

Notable inversions: B3/C7 (on-topic raspberry-pie queries) and E11/E12 (git/Discord
queries unrelated to the food cluster) are clean under `old` but drift under `new`.

## 4. Findings

### 4.1 The redesign regresses on DeepSeek

Gating recall to session-start raised severe drift from 28.6% to 57.1%. The premise
("per-turn recall is the contamination source") is **inverted** for this engine.

### 4.2 Mechanism — per-turn recall is also *corrective grounding*

Reading both context-assembly paths (`old` = `master:handlers/system.rs` L442/L541;
`new` = `feat:handlers/system.rs` L449–621):

- **`old`** builds, every turn, `context = recall(current query) + T1 transcript`.
  The per-turn recall is **query-specific**: a "git push" turn recalls git memories
  and pulls the model back on-topic (E11 `CCC`).
- **`new`** recalls only on turn 1; continuing turns use `context = T1 transcript`
  alone.

Both arms keep turn-1's response in the accumulating T1 transcript. The decisive
difference is what turn 1 does and what corrects it afterward. Raw responses confirm:
the `new` arm's turn-1 recall (triggered by the bread query A1) surfaces the loosely
related cluster (pan/croissant/raspberry-pie/Raspberry Pi); DeepSeek fuses them into a
"double-meaning" narrative; that narrative is pinned in the transcript and, with **no
per-turn corrective recall**, dominates every subsequent turn — even unrelated ones
(E11/E12, F13/F14). Per-turn recall, while it can inject off-topic memories, also
provides per-query relevant grounding that self-corrects drift. Gating it removes the
correction.

### 4.3 Root cause is unchanged: recall *precision*, not *timing*

This confirms [2026-04-24 §4.4](RECALL_CONTAMINATION_AB_2026-04-24.md): the contaminant
(`人生で初めてラズベリーパイを食べた`, cos ≈ 0.31) sits just above the adaptive threshold
(≈ 0.29) for the bread query, because `パン`↔`パイ` share katakana, the past-tense
"…食べた" frame matches, and jina-v5-nano is not fine-tuned for Japanese food semantics.
No recall *timing* policy (per-turn vs session-start) changes which memories cross that
threshold. The 2026-04-24 L2 presentation change regressed; the 2026-06-14 timing change
regresses. Both are downstream of a precision problem.

## 5. Design-principles analysis (ARCHITECTURE.md §1, DEVELOPMENT.md §1.4)

The redesign and the candidate fixes were reviewed against the architecture manifesto.

- **§1.1 Core Minimalism — "the Kernel is the stage, not the actor."** Prohibits
  hard-coding "processing logic for specific memory formats" in the kernel. The Phase 3c
  grounding step filters recall results by the `[Episode]` content prefix **inside the
  kernel** (`handlers/system.rs`), i.e. the kernel interprets a memory-format convention.
  This is in tension with §1.1.
- **§1.4 Data Sovereignty — "the Kernel holds the data but does not interpret its
  contents."** Recall *relevance/precision* is interpretation of memory content; per the
  principle it belongs to the Memory capability (cpersona), not the kernel.
- **§1.2 Capability over Concrete Type — "not who it is, but what it can do."** The
  proposed bug-396 fix (c) — a kernel-side `mind.`-prefix alias in engine resolution —
  branches kernel logic on a name convention, which §1.2 discourages. The compliant
  fixes are data-side: **(a)** restore the catalog connector id to `mind.deepseek`
  (matching the `mind.local` convention agents already reference), or **(b)** update
  agents' `default_engine_id` to `deepseek`.

**Conclusion:** the principle-aligned locus for contamination work is the **CPersona
memory plugin** (recall precision), not kernel-side recall orchestration. The kernel
should call the Memory capability and let the plugin decide what is relevant.

## 6. Recommendation

1. **Do not land the recall-gating redesign as a contamination fix.** It regresses on
   DeepSeek and adds kernel-side memory logic that is in tension with §1.1/§1.4. (Its
   Phase 1 "1 channel = 1 session" bridge change is orthogonal and may stand on its own
   merits; the kernel gating/grounding is what this report argues against.)
2. **Pursue recall precision in cpersona** (the 2026-04-24 §5.3 "future work", all
   plugin-side): threshold recalibration against the jina-v5-nano distribution; MMR /
   diversity-aware reranking; a top-1-anchored relevance-gap filter (drop results far
   below the top match so a 0.59 hit suppresses a 0.31 contaminant while a query whose
   best hit is 0.30 still returns it); or a stronger / fine-tuned embedding model.
3. **Fix bug-396 data-side** ((a) catalog id or (b) agent config), not kernel-side (c).
4. **Harden the harness**: classify `[Error]`/engine-error responses as `error`, not
   `coherent`, so an engine misconfiguration can never masquerade as 0% drift again.

## 7. Reproducing the measurement

Prerequisites: ClotoCore reachable on `http://127.0.0.1:8081`; a bench agent with a
working reasoning engine (`default_engine_id` must equal the **registered** MCP server
name — see bug-396) and `cpersona` memory; embedding server up; DeepSeek credits
(~$2 for 2 × 42 trials).

```bash
# harness is branch-independent; copy out of the feat tree if checking out master:
git archive feat/discord-recall-gating:scripts/recall_ab | tar -x -C /tmp/recall_ab
cd /tmp/recall_ab
export CLOTO_API_KEY=...                       # from .env

# 1. seed clean-19 on whichever build is up, snapshot the kernel cpersona DB
python run_ab.py --agent-id agent.cpersona_bench --seed
sqlite3 <data>/cpersona.db ".backup /tmp/seed19.db"

# 2. OLD arm on master (bug-395 seal fix), then restore the snapshot
python run_ab.py --agent-id agent.cpersona_bench --arm old --trials 3
# 3. NEW arm on feat from the same snapshot
#    (restore /tmp/seed19.db over the kernel cpersona DB while the kernel is stopped)
python run_ab.py --agent-id agent.cpersona_bench --arm new --trials 3

# 4. compare
python compare.py results/results_old.json results/results_new.json
```

Run via `uv run --python 3.13 --with httpx python …` (uv's default 3.14 hangs
pytest-asyncio in this monorepo). Use `--python 3.13`.

## 8. Artifacts

- Harness: `scripts/recall_ab/` (query_set.json, seed_corpus.json, run_ab.py,
  compare.py, README.md)
- Results captured this run: `results_old.json` (28.6%), `results_new.json` (57.1%)
- Related: **bug-396** (`qa/issue-registry.json`) — `mind.deepseek` engine unresolvable
- Predecessor: [`RECALL_CONTAMINATION_AB_2026-04-24.md`](RECALL_CONTAMINATION_AB_2026-04-24.md)

---

## 9. Addendum & Correction (later 2026-06-14) — embeddings were off; two real bugs

After §1–§8 were written, a pre-prototype recall measurement (per the doctor's
request to *measure before fixing*) overturned the framing above. This section is
authoritative.

### 9.1 The §3 A/B ran with CPersona embeddings OFF

Kernel log (cpersona): `Embedding disabled (mode=none), using FTS5 + keyword only`.
`memories.embedding` was NULL for 0/79 rows. The embedding server (`tool.embedding`,
auto_bge_m3, `:8401`) was running but cpersona was **not wired to it**. So the §3
numbers (28.6% / 57.1%) reflect an **FTS-only** recall path, not the production
semantic path (2026-04-24 ran with embeddings on, jina-v5-nano). The NEW-vs-OLD
*regression* still holds mechanistically (§4.2 — it is about transcript pinning +
loss of per-turn corrective recall, independent of recall content), but the absolute
rates are not production-representative.

### 9.2 With embeddings enabled (bge-m3, local cosine), the contaminant IS recalled — and cosine can't separate it

Wired cpersona to `:8401` via `CPERSONA_EMBEDDING_MODE=http`, re-seeded, re-measured
the bread query `この前のパンの話覚えてる?`:

| cosine | memory |
|---|---|
| 0.783 / 0.769 / 0.762 | パン屋 / クロワッサン (correct) |
| **0.751 / 0.747** | **ラズベリーパイ (contaminant)** |

The contaminant sits ~0.01–0.03 below the correct match. No gap/ratio/RSF threshold
separates 0.76 from 0.75 (`gap/top ≈ 0.04 ≪ autocut 0.15`; `ratio ≈ 0.96 ≫ 0.7`).
**bge-m3 semantically conflates パン/ラズベリーパイ** (katakana + food/pie + past-tense
frame), reproducing 2026-04-24 §4.4 even more tightly. ⇒ **cosine-magnitude techniques
(RSF, hybrid, gap filter) cannot fix this** — there is no magnitude to exploit.

### 9.3 The discriminating signal is keyword — and it is broken by two bugs

The only signal that separates the two (the contaminant does **not** contain `パン`)
is the keyword/FTS retriever. It is dead for Japanese due to:

- **Bug A — embedding env-key mismatch (semantic recall silently off).** The catalog /
  connector docs declare the generic keys `EMBEDDING_MODE` / `EMBEDDING_HTTP_URL`
  (with "falls back to `CPERSONA_EMBEDDING_MODE`"), but `cpersona/config.py` reads
  **only** `CPERSONA_EMBEDDING_MODE` / `CPERSONA_EMBEDDING_URL`. Setting the documented
  generic keys is a no-op → embeddings stay `none`. (Default is `none` too.)
  *Location:* `cpersona/config.py:10-11`; catalog `optional_env_vars` for cpersona.
- **Bug B — FTS keyword query splits on whitespace (Japanese keyword search dead).**
  `cpersona/memory_handlers.py::_search_memories_keyword` does
  `words = re.sub(r"[^\w\s]","",query).split()` then `" ".join(f'"{w}"' …)`. Japanese
  has no spaces → the whole sentence becomes **one** quoted FTS phrase → matches only
  an exact full-sentence substring → ~0 hits. The `trigram` tokenizer (already in use —
  `memories_fts … tokenize='trigram'`, so this is **not** a tokenizer-choice bug) is
  wasted because the query is never broken into searchable n-grams. ASCII queries
  (`git push`, `Discord`) split fine → keyword works → RRF discriminates → clean
  (matches the §3.2 pattern: E11/E12 are `CCC`).

### 9.4 Corrected recommendation

1. **Fix Bug B** — build the FTS query for spaceless languages: split CJK runs into
   overlapping bi-/tri-grams as separate OR terms (or per-segment terms) so the
   `trigram` index is actually exercised. This restores the keyword retriever that
   lets RRF separate topically-distinct-but-semantically-near memories. **Highest
   leverage; it is what actually fixes the bread/raspberry case.**
2. **Fix Bug A** — make cpersona read the generic `EMBEDDING_MODE` / `EMBEDDING_HTTP_URL`
   (with `CPERSONA_*` as documented fallback), or correct the catalog/docs to the
   `CPERSONA_*` names. Either way, ensure marketplace-installed cpersona can actually
   enable semantic recall.
3. **Do NOT pursue RSF / hybrid / gap-filter for this contamination** — the cosines are
   near-equal; magnitude carries no separating signal here. (RSF may still help other
   cases where cosine *is* discriminative; it is simply not the fix for this one.)
4. The kernel-side recall-gating redesign remains a non-fix (§6.1).

### 9.5 Status / next (pending doctor's go-ahead)

Formally register Bug A + Bug B (cpersona has no `qa/issue-registry.json`; needs a
location decision), then prototype Bug B (Japanese FTS query construction) and
re-measure on the embeddings-on bench. Environment used: kernel launched headless
with `CPERSONA_EMBEDDING_MODE=http CPERSONA_EMBEDDING_URL=http://127.0.0.1:8401/embed
CPERSONA_VECTOR_SEARCH_MODE=local`.

---

*Report author: ClotoCore Project*
*Measurement date: 2026-06-14*
*Arms: `master` @ 4afb050 (old) vs `feat/discord-recall-gating` rebased (new); engine `deepseek`; agent `agent.cpersona_bench`; clean-19 common baseline; N=3.*
*§9 correction: embeddings were off during §3; real root cause = Bug A (env-key mismatch) + Bug B (FTS whitespace-split). bge-m3 re-measurement confirms cosine cannot separate the contaminant.*
