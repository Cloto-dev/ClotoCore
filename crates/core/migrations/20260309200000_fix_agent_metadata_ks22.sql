-- Fix: migration 20260309000000 renamed memory.ks22 → memory.cpersona in
-- mcp_servers, mcp_access_control, and plugin_configs, but missed the
-- agents.metadata JSON column where preferred_memory is stored.

-- Update preferred_memory in agent metadata JSON
UPDATE agents
SET metadata = json_set(metadata, '$.preferred_memory', 'memory.cpersona')
WHERE json_extract(metadata, '$.preferred_memory') = 'memory.ks22';

-- Re-apply default engine fix (conditional, in case 20260304200002 did not
-- take effect on databases where migration ordering was disrupted)
UPDATE agents
SET default_engine_id = 'mind.cerebras'
WHERE id = 'agent.cloto_default'
  AND default_engine_id = 'mind.deepseek';
