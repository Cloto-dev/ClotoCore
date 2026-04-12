-- Clear the hardcoded Ollama default model.
-- mind.ollama now requires users to select a model explicitly (via Dashboard
-- Settings, the switch_model tool, or the OLLAMA_MODEL env var) rather than
-- silently defaulting to a potentially uninstalled model.
--
-- The UPDATE is gated on the original seed value so user-customized entries
-- (e.g. already switched to qwen3.5:27b) are preserved.
UPDATE llm_providers
SET model_id = ''
WHERE id = 'ollama' AND model_id = 'glm-4.7-flash';
