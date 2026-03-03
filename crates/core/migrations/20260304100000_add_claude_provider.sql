-- Add Claude (Anthropic) as a first-class LLM provider.
-- API key must be configured via the dashboard Settings page.
INSERT OR IGNORE INTO llm_providers (id, display_name, api_url, api_key, model_id, timeout_secs, enabled)
VALUES ('claude', 'Claude', 'https://api.anthropic.com/v1/messages', '', 'claude-sonnet-4-6', 120, 1);
