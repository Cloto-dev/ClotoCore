-- Add dynamic capabilities to agents
ALTER TABLE agents ADD COLUMN required_capabilities TEXT DEFAULT '["Reasoning", "Memory"]';
