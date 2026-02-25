-- Fix default_engine_id for agents that have a mind.* plugin assigned
-- but whose default_engine_id is not yet pointing to it.
-- Also inject agent_type='ai' into metadata for explicit detection.
UPDATE agents
SET default_engine_id = (
    SELECT plugin_id FROM agent_plugins
    WHERE agent_id = agents.id AND plugin_id LIKE 'mind.%'
    ORDER BY pos_y, pos_x LIMIT 1
),
metadata = json_set(metadata, '$.agent_type', 'ai')
WHERE id IN (
    SELECT DISTINCT agent_id FROM agent_plugins WHERE plugin_id LIKE 'mind.%'
)
AND (default_engine_id NOT LIKE 'mind.%' OR default_engine_id IS NULL);
