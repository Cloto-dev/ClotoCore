-- Phase 8: Add agent_type column for UI filtering (G1.5)
-- Default 'agent' for user-created agents, 'system' for system.* agents
ALTER TABLE agents ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'agent';
UPDATE agents SET agent_type = 'system' WHERE id LIKE 'system.%';
