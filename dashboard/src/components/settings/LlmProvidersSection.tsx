import { RefreshCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { SectionCard } from './common';

const MODEL_ID_MAX_LEN = 200;

// Provider-specific model-ID format examples. Each LLM backend has its own
// naming convention and showing a wrong example (e.g. Ollama's `name:tag`
// format for LM Studio, which expects `org/name`) silently misleads users
// into saving an invalid model ID.
const MODEL_PLACEHOLDER_BY_PROVIDER: Record<string, string> = {
  local: 'qwen/qwen3.5-9b',
  ollama: 'qwen3.5:9b',
  claude: 'claude-sonnet-4-6',
  cerebras: 'gpt-oss-120b',
  deepseek: 'deepseek-chat',
  groq: 'openai/gpt-oss-20b',
};

type Provider = {
  id: string;
  display_name: string;
  has_key: boolean;
  model_id: string;
};

type ModelOption = { id: string; name?: string };
type ModelListState = {
  status: 'loading' | 'ready' | 'fallback';
  models: ModelOption[];
  errorCode?: string;
};

export function LlmProvidersSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const [providers, setProviders] = useState<Provider[]>([]);
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);

  // Model edit state
  const [editingModelId, setEditingModelId] = useState<string | null>(null);
  const [modelInput, setModelInput] = useState('');
  const [modelSaving, setModelSaving] = useState(false);
  const [modelError, setModelError] = useState<string | null>(null);
  const [modelList, setModelList] = useState<ModelListState | null>(null);
  const modelInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api
      .listLlmProviders()
      .then((d) => setProviders(d.providers))
      .catch((e) => {
        if (import.meta.env.DEV) console.warn('Failed to load LLM providers:', e);
      });
  }, [api]);

  const handleSave = async (providerId: string) => {
    if (!keyInputs[providerId]?.trim()) return;
    setSaving(providerId);
    try {
      await api.setLlmProviderKey(providerId, keyInputs[providerId].trim());
      setKeyInputs((prev) => ({ ...prev, [providerId]: '' }));
      const d = await api.listLlmProviders();
      setProviders(d.providers);
    } catch {
      /* ignore */
    }
    setSaving(null);
  };

  const handleDelete = async (providerId: string) => {
    await api.deleteLlmProviderKey(providerId);
    const d = await api.listLlmProviders();
    setProviders(d.providers);
  };

  const fetchModels = async (providerId: string): Promise<ModelListState> => {
    try {
      const res = await api.listProviderModels(providerId);
      if (res.error_code && res.error_code !== 'static_fallback') {
        return { status: 'fallback', models: res.models ?? [], errorCode: res.error_code };
      }
      return { status: 'ready', models: res.models ?? [], errorCode: res.error_code };
    } catch {
      return { status: 'fallback', models: [], errorCode: 'request_failed' };
    }
  };

  const startModelEdit = async (p: Provider) => {
    setEditingModelId(p.id);
    setModelInput(p.model_id);
    setModelError(null);
    setModelList({ status: 'loading', models: [] });
    // Focus the input immediately (pre-fetch) so user can type even before models load
    setTimeout(() => modelInputRef.current?.focus(), 0);
    const result = await fetchModels(p.id);
    setModelList(result);
  };

  const refreshModels = async (providerId: string) => {
    setModelList({ status: 'loading', models: [] });
    const result = await fetchModels(providerId);
    setModelList(result);
  };

  const cancelModelEdit = () => {
    setEditingModelId(null);
    setModelInput('');
    setModelError(null);
    setModelList(null);
  };

  const commitModelEdit = async (providerId: string) => {
    const trimmed = modelInput.trim();
    if (!trimmed) {
      setModelError(t('llm_providers.model_validation_empty'));
      return;
    }
    if (trimmed.length > MODEL_ID_MAX_LEN) {
      setModelError(t('llm_providers.model_validation_too_long'));
      return;
    }
    setModelSaving(true);
    setModelError(null);
    try {
      await api.setLlmProviderModel(providerId, trimmed);
      const d = await api.listLlmProviders();
      setProviders(d.providers);
      cancelModelEdit();
    } catch (e) {
      setModelError(e instanceof Error ? e.message : String(e));
    } finally {
      setModelSaving(false);
    }
  };

  const handleModelKeyDown = (
    e: React.KeyboardEvent<HTMLInputElement | HTMLSelectElement>,
    providerId: string,
  ) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitModelEdit(providerId);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelModelEdit();
    }
  };

  return (
    <SectionCard title={t('llm_providers.title')}>
      <p className="text-xs text-content-tertiary mb-4">{t('llm_providers.desc')}</p>
      <div className="space-y-3">
        {providers.map((p) => (
          <div
            key={p.id}
            className="flex items-center gap-3 p-3 bg-surface-secondary rounded-lg border border-edge-subtle"
          >
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span
                  role="img"
                  className={`w-2 h-2 rounded-full ${p.has_key ? 'bg-green-500' : 'bg-amber-500'}`}
                  aria-label={p.has_key ? 'API key configured' : 'API key not configured'}
                />
                <span className="text-xs font-bold text-content-primary">{p.display_name}</span>
                {editingModelId === p.id ? (
                  <div className="flex items-center gap-1">
                    {modelList?.status === 'ready' && modelList.models.length > 0 ? (
                      <select
                        aria-label={`${p.display_name} model ID`}
                        value={modelInput}
                        onChange={(e) => setModelInput(e.target.value)}
                        onKeyDown={(e) => handleModelKeyDown(e, p.id)}
                        className="bg-surface-base border border-brand/50 rounded px-2 py-0.5 text-[11px] font-mono text-content-primary w-48"
                      >
                        {/* Preserve a currently-saved model that isn't in the list (e.g. unloaded) */}
                        {modelInput && !modelList.models.some((m) => m.id === modelInput) && (
                          <option value={modelInput}>{modelInput}</option>
                        )}
                        {modelList.models.map((m) => (
                          <option key={m.id} value={m.id}>
                            {m.name ? `${m.id} — ${m.name}` : m.id}
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        ref={modelInputRef}
                        type="text"
                        value={modelInput}
                        maxLength={MODEL_ID_MAX_LEN}
                        onChange={(e) => setModelInput(e.target.value)}
                        onKeyDown={(e) => handleModelKeyDown(e, p.id)}
                        aria-label={`${p.display_name} model ID`}
                        placeholder={
                          modelList?.status === 'loading'
                            ? t('llm_providers.model_dropdown_loading')
                            : MODEL_PLACEHOLDER_BY_PROVIDER[p.id]
                              ? t('llm_providers.model_placeholder_ex', { example: MODEL_PLACEHOLDER_BY_PROVIDER[p.id] })
                              : t('llm_providers.model_placeholder')
                        }
                        className="bg-surface-base border border-brand/50 rounded px-2 py-0.5 text-[11px] font-mono text-content-primary placeholder:text-content-tertiary w-48"
                      />
                    )}
                    <button
                      type="button"
                      onClick={() => refreshModels(p.id)}
                      disabled={modelList?.status === 'loading'}
                      aria-label={t('llm_providers.model_refresh')}
                      title={t('llm_providers.model_refresh')}
                      className="p-0.5 text-content-tertiary hover:text-brand rounded disabled:opacity-40"
                    >
                      <RefreshCw
                        className={`w-3 h-3 ${modelList?.status === 'loading' ? 'animate-spin' : ''}`}
                      />
                    </button>
                    <button
                      onClick={() => commitModelEdit(p.id)}
                      disabled={modelSaving || !modelInput.trim()}
                      aria-label={t('llm_providers.model_save')}
                      className="px-2 py-0.5 bg-brand text-white text-[10px] font-bold rounded disabled:opacity-40"
                    >
                      {modelSaving ? '...' : t('llm_providers.model_save')}
                    </button>
                    <button
                      onClick={cancelModelEdit}
                      disabled={modelSaving}
                      aria-label={t('llm_providers.model_cancel')}
                      className="px-2 py-0.5 text-content-tertiary text-[10px] hover:text-content-primary rounded"
                    >
                      {t('llm_providers.model_cancel')}
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => startModelEdit(p)}
                    title={t('llm_providers.model_edit_hint')}
                    className="text-[11px] font-mono text-content-tertiary hover:text-brand hover:underline cursor-pointer bg-transparent border-0 p-0"
                  >
                    {p.model_id || <span className="italic">{t('llm_providers.model_unset')}</span>}
                  </button>
                )}
              </div>
              {editingModelId === p.id && modelError && (
                <p className="text-[10px] text-red-400 mt-1 ml-4">{modelError}</p>
              )}
              {editingModelId === p.id && modelList?.status === 'fallback' && !modelError && (
                <p className="text-[10px] text-content-tertiary mt-1 ml-4">
                  {t('llm_providers.model_dropdown_error', { code: modelList.errorCode ?? 'unknown' })}
                </p>
              )}
              <div className="flex gap-2 mt-2">
                <input
                  type="password"
                  value={keyInputs[p.id] || ''}
                  onChange={(e) => setKeyInputs((prev) => ({ ...prev, [p.id]: e.target.value }))}
                  placeholder={p.has_key ? t('llm_providers.placeholder_saved') : t('llm_providers.placeholder_new')}
                  className="flex-1 bg-surface-base border border-edge rounded px-2 py-1 text-xs font-mono text-content-primary placeholder:text-content-tertiary"
                />
                <button
                  onClick={() => handleSave(p.id)}
                  disabled={!keyInputs[p.id]?.trim() || saving === p.id}
                  aria-label={`${tc('save')} ${p.display_name}`}
                  className="px-3 py-1 bg-brand text-white text-xs font-bold rounded disabled:opacity-40"
                >
                  {saving === p.id ? '...' : tc('save')}
                </button>
                {p.has_key && (
                  <button
                    onClick={() => handleDelete(p.id)}
                    aria-label={`${t('llm_providers.clear')} ${p.display_name}`}
                    className="px-2 py-1 text-red-400 text-xs hover:bg-red-500/10 rounded"
                  >
                    {t('llm_providers.clear')}
                  </button>
                )}
              </div>
            </div>
          </div>
        ))}
        {providers.length === 0 && <p className="text-xs text-content-tertiary italic">{t('llm_providers.empty')}</p>}
      </div>
    </SectionCard>
  );
}
