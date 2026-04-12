-- Add LM Studio as a local LLM provider.
-- LM Studio exposes an OpenAI-compatible server on port 1234 by default.
-- model_id is left empty: users select the loaded model in LM Studio's GUI and
-- configure the model name via the Dashboard Settings page.
INSERT OR IGNORE INTO llm_providers (id, display_name, api_url, api_key, model_id, timeout_secs, enabled)
VALUES ('lmstudio', 'LM Studio', 'http://localhost:1234/v1/chat/completions', '', '', 120, 1);
