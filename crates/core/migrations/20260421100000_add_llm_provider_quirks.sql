-- Add per-provider quirks JSON column so provider-specific adapter logic
-- (native models-list path, live switch-model tool, no-api-key flag) can be
-- declared as data instead of hard-coded branches in handlers/llm.rs.
-- See docs/ARCHITECTURE.md §1.1 (Core Minimalism) and PROJECT_VISION.md §11.
--
-- Schema:
--   quirks: TEXT NULL — JSON object with optional fields:
--     {
--       "no_api_key": bool,            // provider does not require an API key
--       "models_endpoint_path": str,   // native models-list path (overrides OpenAI-compat derivation)
--       "switch_model_tool": str       // MCP tool name on `mind.<provider_id>` to relay model change
--     }
--   Rows with NULL quirks are treated as "standard OpenAI-compatible provider, key required".

ALTER TABLE llm_providers ADD COLUMN quirks TEXT;

-- Seed the two known non-standard providers. Other rows stay NULL.
UPDATE llm_providers
SET quirks = '{"no_api_key":true,"models_endpoint_path":"/api/tags","switch_model_tool":"switch_model"}'
WHERE id = 'ollama';

UPDATE llm_providers
SET quirks = '{"no_api_key":true}'
WHERE id = 'local';
