-- Per-provider override for reasoning/thinking mode.
-- 'auto' (default): server-side heuristic on model name determines mode.
-- 'on': force reasoning on, regardless of model name.
-- 'off': force reasoning off, useful for hybrid models or speed-prioritized runs.
-- Injected by augment_mind_env as {PREFIX}_REASONING_PREFILL=true/false when
-- value is 'on'/'off'; skipped for 'auto' so server-side detection fires.

ALTER TABLE llm_providers
  ADD COLUMN reasoning_prefill TEXT NOT NULL DEFAULT 'auto'
  CHECK (reasoning_prefill IN ('auto', 'on', 'off'));
