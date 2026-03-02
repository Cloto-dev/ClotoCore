-- Update Cerebras default model: llama-3.3-70b was removed from Cerebras API.
-- gpt-oss-120b is a 120B parameter model with ultra-fast inference.
UPDATE llm_providers SET model_id = 'gpt-oss-120b' WHERE id = 'cerebras' AND model_id = 'llama-3.3-70b';
