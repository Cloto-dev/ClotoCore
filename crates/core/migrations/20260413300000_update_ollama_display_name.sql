-- Align Ollama's display_name with the Dashboard positioning established
-- in 20260413200000 (mind.local = recommended, mind.ollama = alternative).
-- Gated on the original seed value so user-customized entries are preserved.
UPDATE llm_providers
SET display_name = 'Ollama (代替)'
WHERE id = 'ollama' AND display_name = 'Ollama (Local)';
