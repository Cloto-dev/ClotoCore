-- LLM Provider registry: centralized API key management (MGP §13.4 llm_completion)
-- API keys are held by the kernel only; MCP servers call through the internal proxy.
CREATE TABLE IF NOT EXISTS llm_providers (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    api_url TEXT NOT NULL,
    api_key TEXT DEFAULT '',
    model_id TEXT NOT NULL,
    timeout_secs INTEGER NOT NULL DEFAULT 120,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO llm_providers (id, display_name, api_url, model_id)
VALUES
  ('deepseek', 'DeepSeek', 'https://api.deepseek.com/chat/completions', 'deepseek-chat'),
  ('cerebras', 'Cerebras', 'https://api.cerebras.ai/v1/chat/completions', 'llama-3.3-70b'),
  ('ollama', 'Ollama (Local)', 'http://localhost:11434/api/chat', 'glm-4.7-flash');
