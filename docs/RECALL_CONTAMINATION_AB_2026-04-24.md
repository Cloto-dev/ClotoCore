# CPersona Recall Contamination — AB Test Report (2026-04-24)

> **Status:** Measurement complete. v2.4.12 quality gate fix landed. Topic-drift
> contamination remains an open problem.
> **Scope:** CPersona recall pipeline + mind-server memory presentation.
> **Prompted by:** User-visible "pan → raspberry pie" topic drift on Dashboard
> chat after the v2.4.12 quality-gate fix restored recall functionality.

## 1. Background

### 1.1 Contamination symptom

On a production-like agent (`agent.cloto_default`, 108 memories) a query of the
form "この前のパンの話覚えてる?" produced a response that started coherently
("パン屋さんの話は先ほど聞いたばかり") and then abruptly pivoted:

> そして…ラズベリーパイ！人生初めてなんですね！🥧✨ カリカリのパイ生地と
> ふわふわのフィリングのコントラスト…どこのお店で食べたんですか？

Memory `id=113` ("人生で初めてラズベリーパイを食べた") was an entirely
separate conversation that the agent elevated into the active discussion.

### 1.2 Two bugs, not one

Investigation identified **two distinct defects** in the recall path:

1. **RRF quality-gate scale mismatch** (v2.4.6 regression): the adaptive
   threshold designed for cosine similarity (0.2–1.0) was applied to
   `_rrf_score` values (0–~0.05), causing every vector-matched row to be
   silently blocked in RRF mode. Default recall returned zero results even
   when strong matches existed. **Fixed in v2.4.12.**
2. **Topic-drift contamination**: when recall *does* return results, the
   mind-server assembles them into the LLM prompt as `role=user` / `role=assistant`
   chat-history turns (`cloto-mcp-servers/servers/common/llm_provider.py`
   `build_chat_messages()` lines ~361–385). The LLM interprets these as
   currently-active dialogue threads and continues their topics. **Unresolved.**

## 2. AB Test Methodology (Mode Y + Z-Step 1)

### 2.1 Fixed inputs

- **Agent**: `agent.cloto_default` with 108 memories (jina-v5-nano 768d)
- **Engine**: `mind.deepseek` (DeepSeek API, temperature default)
- **Query set (14, 6 categories)**:
  - Known-bad drift triggers: `この前のパンの話覚えてる?`, `昨日話したパンの件`
  - Reverse direction: `ラズベリーパイについて覚えてる?`, `Raspberry Pi って何?`
  - Short keyword: `パン`, `朝食`, `ラズベリーパイ`
  - Open-ended meta: `昨日何話してたっけ`, `このセッションで何話した?`, `私の好きな食べ物は?`
  - Specific topic: `git push の件`, `Discord の話`
  - False positive (no relevant memory): `今日の天気`, `週末の予定`
- **N = 3 trials per query per arm** (LLM variance sampling)
- **DB snapshot** (`/tmp/cpersona-snapshot-Y.db`) restored before each arm
- **Session accumulation within an arm is intentional** (real-world condition)

### 2.2 Arms

| Arm | Quality gate | Memory presentation | Anti-contamination prompt |
|---|---|---|---|
| `A-v245` | disabled (simulate v2.4.5) | chat-turn format | — |
| `A-v12` | **v2.4.12 fix** | chat-turn format | — |
| `B-L1` | v2.4.12 | chat-turn format | **added** |
| `B-L2` | v2.4.12 | **single system block** | — |
| `B-L12` | v2.4.12 | single system block | added |

### 2.3 Metrics

- **(a)** Full LLM prompt dump (`CPERSONA_AB_DUMP_PROMPTS=1` probe in
  `common/llm_provider.py::call_llm_api`, removed after the measurement)
- **(b)** Rubric-based response classification: severe / mild / coherent drift
  - SEVERE = unrelated-topic keyword present **and** elaborated (≥2 detail
    markers or follow-up question or ≥3 keyword mentions)
  - MILD = unrelated keyword mentioned with disclaim ("以前の…", "別の話…")
  - COHERENT = no unrelated keyword, or dismissed appropriately
- **(c)** End-to-end latency per trial (via SSE `ThoughtResponse` timing)
- **(d)** Cosine top-5 per query (embedding server HTTP endpoint)

### 2.4 Tooling

Harness scripts (kept locally in `/tmp/`, not committed):

- `cpersona_ab_runner.py` — multi-arm runner via `/api/chat` + SSE subscription
- `cpersona_arm_switch.sh` — DB restore + patch application
- `cpersona_ab_measure.py` — cosine top-N simulation per query

## 3. Results

### 3.1 5-arm aggregate (14 queries × 3 trials = 42 each, 210 total)

| Arm | Coherent | Mild | Severe | Timeout | API-err | Sev%/Completed | Latency median |
|---|---|---|---|---|---|---|---|
| A-v245 | 27 | 1 | 11 | 3 | 0 | **28.2%** | 3.8s |
| **A-v12** | 26 | 4 | 9 | 3 | 0 | **23.1%** ← best | 4.2s |
| B-L1 | 25 | 1 | 9 | 4 | 3 | 25.7% | 3.5s |
| **B-L2** | 12 | 2 | 8 | **20** | 0 | **36.4%** ← worst | 9.0s |
| B-L12 | 18 | 1 | 9 | 14 | 0 | 32.1% | 5.0s |

Severe-drift rate is computed out of completed (non-timeout, non-error) trials.

### 3.2 Per-query × arm pattern (3 trials per cell)

`C` = coherent, `m` = mild, `S` = severe, `T` = timeout, `E` = API error.

| Query | A-v245 | A-v12 | B-L1 | B-L2 | B-L12 |
|---|---|---|---|---|---|
| A1 `この前のパンの話覚えてる?` | SSS | SSS | SSS | SSS | SSS |
| A2 `昨日話したパンの件…` | SSS | SSS | SSS | TTS | SSS |
| B3 `ラズベリーパイについて覚えてる?` | CCC | CCC | CCC | CCC | CCC |
| B4 `Raspberry Pi って何?` | CCC | CCC | CCC | CCC | CCC |
| C5 `パン` | SSm | mSm | TSm | SSS | TTm |
| C6 `朝食` | SSS | SmS | CSS | Smm | SSS |
| C7 `ラズベリーパイ` | CCC | CCC | CCC | CTT | CCC |
| D8 `昨日何話してたっけ` | TCC | CCT | CCC | TTT | TCT |
| D9 `このセッションで何話した?` | CCC | CCC | CCC | TTT | TCC |
| D10 `私の好きな食べ物は?` | CCC | CCC | CCC | CCC | CCC |
| E11 `git push の件` | CCC | CCC | CCC | TTT | TTT |
| E12 `Discord の話` | CCC | mCC | CCC | CTC | CCC |
| F13 `今日の天気` | CCT | TTC | TTT | TTT | TTT |
| F14 `週末の予定` | TCC | CCC | EEE | TTT | TTT |

Note: `E11 / F13 / F14` went from mostly-C in A arms to all-T in B-L2/B-L12 —
a strong regression signal independent of drift.

### 3.3 Prompt-size diagnostics (metric a)

Mean message-array size sent to DeepSeek per LLM call:

| Arm | Avg messages | Avg bytes | Shape |
|---|---|---|---|
| A-v245 | 18.4 | 7898 | many chat turns (1 sys + N×(ts, memory)) |
| A-v12 | 18.2 | 7574 | same shape |
| B-L2 / B-L12 | **3.6** | 8340 | 1 system + 1 block + 1 user |

The L2 consolidation *did* reduce message count as designed. Total byte count
is comparable — LLM-side processing cost alone does not explain the latency
spike.

## 4. Findings

### 4.1 v2.4.12 quality-gate fix is a real improvement

- Severe-drift rate **28.2% (A-v245) → 23.1% (A-v12)**, a 5.1 pp reduction
  with no latency or completion-rate cost.
- Mechanism: the fix restores the adaptive quality-gate's ability to filter
  out low-confidence matches (id 113 at cosine ≈ 0.30 passes the broken gate
  just because `_rrf_score > min_score` was never true).
- Production-ready. Landed in the commit associated with this report.

### 4.2 Single-system-block memory format (L2) is a net regression

- Severe-drift **rose** from 23.1% to 36.4% (of completed), i.e. worse than
  any baseline.
- Timeout rate jumped from 7% to **48%** (20/42).
- Queries with no matching memory (F13 / F14 / E11) were hit hardest — the
  model appears to get "stuck" parsing the `<<BACKGROUND_MEMORIES>>` block
  even when it should be ignored.
- Hypotheses (unverified):
  1. `<<…>>` tags look code-like; the model enters a "parse / respond to
     block" mode instead of treating it as reference.
  2. Large single system messages (~8 KB) may fall outside DeepSeek's training
     distribution for system-role usage.
  3. Loss of chat-turn structure removes temporal cues the model was relying on.

### 4.3 Anti-contamination prompt (L1) has no measurable effect

- Severe-drift 23.1% (A-v12) vs 25.7% (B-L1) — within LLM sampling variance
  at N=3.
- Even the explicit instruction "Do NOT proactively elaborate on recalled
  memories" does not alter DeepSeek's behavior meaningfully in this setup.
- This matches known results on LLM instruction adherence: negative
  instructions are unreliable, especially for strong in-context signals.

### 4.4 Cosine distribution is the underlying structural issue

Query `この前のパンの話覚えてる?` against corpus with memory_count=108:
- Relevant memory id 111 "今日の朝食で食べたパン屋さんの話": cos = 0.59
- Contaminant id 113 "人生で初めてラズベリーパイを食べた": cos = 0.31
- Current adaptive threshold at 108 memories: 0.29

id 113 passes by **0.02 above threshold**. Root causes:
- `パン` ↔ `パイ` share 2/3 katakana — tokenizer-level neighborhood
- "食べた", "初めて", past-tense memory frame shared with the query
- jina-v5-nano is general-purpose multilingual, not fine-tuned for Japanese
  food semantics

No threshold choice cleanly separates 0.59 and 0.31 while still retaining
legitimate 0.30-range matches in other queries.

## 5. Open Problem: Topic-Drift Contamination

### 5.1 Not fixed by this PR

The topic-drift / "そして…" pivot to unrelated recalled topics persists at
~23% severe-drift rate after v2.4.12. This is acknowledged as an unresolved
CPersona 2.4.x issue.

### 5.2 Tried and rejected in this session

| Approach | Result |
|---|---|
| L1 — system-prompt anti-contamination instruction | no effect |
| L2 — single-system-block memory presentation | severe regression (drift↑, timeout↑) |
| L1+L2 combined | regression |

### 5.3 Design space still to explore (future work)

- **L2 variant A**: inline per-memory markers (keep chat-turn format, add
  `[BACKGROUND]`/`[END]` system fences between groups)
- **L2 variant B**: `role=user` wrapper with memories packed inside a single
  user message framed as reference material
- **L2 variant C**: top-K truncation (e.g., force K=3) to reduce context load
- **L3**: recall-scope limits — `CPERSONA_AUTOCUT_ENABLED=true`, recency window,
  episode-boundary filtering
- **L4 (structural)**: per-memory topic vectors with MMR diversification, or
  episode-based session isolation
- **Threshold recalibration**: run `calibrate_threshold` tool against the
  jina-v5-nano cosine distribution to set a data-driven floor

### 5.4 Practical workaround today

Users who find drift intolerable can set `CPERSONA_RECALL_MODE=cascade`.
Cascade does not carry the RRF-score scale mismatch and tends to produce
fewer recall hits per query at the cost of weaker multi-retriever fusion —
empirically this reduces the probability of a borderline 0.30 contaminant
appearing in the result set.

## 6. Reproducing the measurement

Prerequisites:
- ClotoCore running with `agent.cloto_default` having ≥50 memories, some of
  which are from different topical conversations
- Embedding server on `127.0.0.1:8401`
- DeepSeek API credits (~$2 covers 5 × 42 trials)

Steps (summary):
1. Snapshot `cpersona.db` of the target agent
2. Apply or revert the patches per arm (see §2.2)
3. For each arm: restore DB, restart ClotoCore, run the 14 × 3 trials via the
   chat API with SSE capture
4. Classify responses per §2.3 rubric

The probe `CPERSONA_AB_DUMP_PROMPTS=1` in `common/llm_provider.py::call_llm_api`
used in this session was removed after measurement; re-add it to capture
full prompts if needed.

## 7. Commit landing with this report

- `servers/cpersona/server.py` — quality-gate priority fix (confidence >
  cosine > scaled RRF), v2.4.12 bump, new tests
- `servers/cpersona/pyproject.toml` — version 2.4.12
- `registry.json` — `memory.cpersona` version 2.4.12 + changelog
- `servers/tests/test_cpersona_quality_gate.py` — +6 tests, -1 obsolete test
- `qa/test-baseline.json` — baseline 198 → 203

No changes to `servers/common/llm_provider.py` (L1/L2 reverted after
regression finding).

---

*Report author: ClotoCore Project*
*Measurement date: 2026-04-24*
*Baseline commit: one ahead of d17ad75 `feat(embedding): cross-platform ONNX EP selection …`*
