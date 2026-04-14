-- Add an optional context-window column so the kernel can pre-flight-validate
-- outgoing LLM requests against the provider's actual limit, rather than
-- discovering overflow only when the provider returns HTTP 400.
--
-- Auto-populated by the dashboard "Detect" button (which reads LM Studio's
-- /api/v0/models) or set manually; NULL means "unknown — don't pre-flight".

ALTER TABLE llm_providers ADD COLUMN context_length INTEGER DEFAULT NULL;
