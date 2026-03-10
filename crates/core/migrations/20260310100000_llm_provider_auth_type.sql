-- Phase 7: Add auth_type column to llm_providers for provider-agnostic auth header selection.
-- Eliminates ANTHROPIC_PROVIDER_ID hard-coding in the kernel (bug-272).

ALTER TABLE llm_providers ADD COLUMN auth_type TEXT NOT NULL DEFAULT 'bearer';

-- Claude uses x-api-key header instead of Bearer token
UPDATE llm_providers SET auth_type = 'x-api-key' WHERE id = 'claude';
