-- Fix: Cloto Assistant was incorrectly using mind.deepseek as default engine.
-- The agent_plugins table has mind.cerebras assigned, but agents.default_engine_id
-- was never updated due to a WHERE condition bug in 20260220000001.

UPDATE agents
SET default_engine_id = 'mind.cerebras'
WHERE id = 'agent.cloto_default'
  AND default_engine_id = 'mind.deepseek';
