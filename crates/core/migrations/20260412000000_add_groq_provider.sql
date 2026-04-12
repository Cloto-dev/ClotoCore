-- Add Groq as a first-class LLM provider.
-- Groq offers ultra-fast inference (500+ tok/s) for gpt-oss-120b via an
-- OpenAI-compatible endpoint, so auth_type defaults to 'bearer'.
-- API key must be configured via the dashboard Settings page.
INSERT OR IGNORE INTO llm_providers (id, display_name, api_url, api_key, model_id, timeout_secs, enabled)
VALUES ('groq', 'Groq', 'https://api.groq.com/openai/v1/chat/completions', '', 'openai/gpt-oss-120b', 30, 1);
