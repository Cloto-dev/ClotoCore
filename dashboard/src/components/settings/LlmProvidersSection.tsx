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
  context_length: number | null;
};

type ModelOption = {
  id: string;
  name?: string;
  loaded?: boolean;
  max_context_length?: number;
  loaded_context_length?: number;
  architecture?: string;
};
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

  // Context length edit state (separate from model edit so both fields can be toggled independently)
  const [editingCtxId, setEditingCtxId] = useState<string | null>(null);
  const [ctxInput, setCtxInput] = useState('');
  const [ctxSaving, setCtxSaving] = useState(false);
  const [ctxError, setCtxError] = useState<string | null>(null);
  const ctxInputRef = useRef<HTMLInputElement>(null);

  // Per-provider connection test state — ephemeral UI feedback.
  type TestState =
    | { phase: 'idle' }
    | { phase: 'running' }
    | {
        phase: 'done';
        status: 'ok' | 'auth_failed' | 'unreachable' | 'model_list_unavailable';
        latency_ms: number;
        models_count: number | null;
      };
  const [testStates, setTestStates] = useState<Record<string, TestState>>({});

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

  const startCtxEdit = (p: Provider) => {
    setEditingCtxId(p.id);
    setCtxInput(p.context_length != null ? String(p.context_length) : '');
    setCtxError(null);
    setTimeout(() => ctxInputRef.current?.focus(), 0);
  };

  const cancelCtxEdit = () => {
    setEditingCtxId(null);
    setCtxInput('');
    setCtxError(null);
  };

  const commitCtxEdit = async (providerId: string) => {
    const trimmed = ctxInput.trim();
    const parsed: number | null = trimmed === '' ? null : Number(trimmed);
    if (parsed !== null && (!Number.isFinite(parsed) || !Number.isInteger(parsed) || parsed <= 0)) {
      setCtxError(t('llm_providers.context_length_validation'));
      return;
    }
    setCtxSaving(true);
    setCtxError(null);
    try {
      await api.setLlmProviderContextLength(providerId, parsed);
      const d = await api.listLlmProviders();
      setProviders(d.providers);
      cancelCtxEdit();
    } catch (e) {
      setCtxError(e instanceof Error ? e.message : String(e));
    } finally {
      setCtxSaving(false);
    }
  };

  const handleCtxKeyDown = (e: React.KeyboardEvent<HTMLInputElement>, providerId: string) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      commitCtxEdit(providerId);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelCtxEdit();
    }
  };

  const runConnectionTest = async (providerId: string) => {
    setTestStates((s) => ({ ...s, [providerId]: { phase: 'running' } }));
    try {
      const res = await api.testProviderConnection(providerId);
      setTestStates((s) => ({
        ...s,
        [providerId]: {
          phase: 'done',
          status: res.status,
          latency_ms: res.latency_ms,
          models_count: res.models_count,
        },
      }));
      // Auto-clear the pill after 10s so the next interaction starts clean.
      setTimeout(() => {
        setTestStates((s) => {
          const current = s[providerId];
          if (current && current.phase === 'done') {
            const { [providerId]: _, ...rest } = s;
            return rest;
          }
          return s;
        });
      }, 10_000);
    } catch (e) {
      setTestStates((s) => ({
        ...s,
        [providerId]: {
          phase: 'done',
          status: 'unreachable',
          latency_ms: 0,
          models_count: null,
        },
      }));
      if (import.meta.env.DEV) console.warn('test_provider_connection failed:', e);
    }
  };

  /// Auto-fill context_length from probe data for the provider's currently-set model.
  /// Prefers the actual `loaded_context_length` (what LM Studio will accept right now)
  /// over `max_context_length` (the model's native maximum), because the former is
  /// what pre-flight validation in the kernel actually cares about. Falls back to
  /// the native max when the model isn't loaded.
  const detectCtxFromProbe = async (providerId: string) => {
    const list = await fetchModels(providerId);
    const currentProvider = providers.find((p) => p.id === providerId);
    const modelId = currentProvider?.model_id;
    const target = modelId ? list.models.find((m) => m.id === modelId) : undefined;
    const detected = target?.loaded_context_length ?? target?.max_context_length;
    if (detected) {
      setCtxInput(String(detected));
    } else {
      setCtxError(t('llm_providers.context_length_detect_unavailable'));
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
                        {modelList.models.map((m) => {
                          const parts: string[] = [m.id];
                          if (m.name) parts.push(`— ${m.name}`);
                          // Prefer showing the actually loaded n_ctx (what LM Studio will
                          // accept right now) alongside the model's native maximum so the
                          // user can see the gap at a glance.
                          if (m.loaded && m.loaded_context_length && m.max_context_length &&
                              m.loaded_context_length !== m.max_context_length) {
                            parts.push(
                              `· ${t('llm_providers.model_ctx_loaded_of_max', {
                                loaded: m.loaded_context_length.toLocaleString(),
                                max: m.max_context_length.toLocaleString(),
                              })}`,
                            );
                          } else if (m.loaded && m.loaded_context_length) {
                            parts.push(
                              `· ${t('llm_providers.model_ctx_suffix', {
                                tokens: m.loaded_context_length.toLocaleString(),
                              })}`,
                            );
                          } else if (m.max_context_length) {
                            parts.push(
                              `· ${t('llm_providers.model_ctx_max_suffix', {
                                tokens: m.max_context_length.toLocaleString(),
                              })}`,
                            );
                          }
                          if (m.loaded) parts.push(`· ${t('llm_providers.model_loaded')}`);
                          return (
                            <option key={m.id} value={m.id}>
                              {parts.join(' ')}
                            </option>
                          );
                        })}
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
                <span className="text-content-tertiary text-[10px]">·</span>
                {editingCtxId === p.id ? (
                  <div className="flex items-center gap-1">
                    <input
                      ref={ctxInputRef}
                      type="number"
                      min={1}
                      step={1}
                      value={ctxInput}
                      onChange={(e) => setCtxInput(e.target.value)}
                      onKeyDown={(e) => handleCtxKeyDown(e, p.id)}
                      aria-label={`${p.display_name} context length`}
                      placeholder={t('llm_providers.context_length_placeholder')}
                      className="bg-surface-base border border-brand/50 rounded px-2 py-0.5 text-[11px] font-mono text-content-primary placeholder:text-content-tertiary w-24"
                    />
                    <button
                      type="button"
                      onClick={() => detectCtxFromProbe(p.id)}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.context_length_detect')}
                      title={t('llm_providers.context_length_detect')}
                      className="px-2 py-0.5 text-content-tertiary text-[10px] hover:text-brand rounded disabled:opacity-40"
                    >
                      {t('llm_providers.context_length_detect')}
                    </button>
                    <button
                      onClick={() => commitCtxEdit(p.id)}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.model_save')}
                      className="px-2 py-0.5 bg-brand text-white text-[10px] font-bold rounded disabled:opacity-40"
                    >
                      {ctxSaving ? '...' : t('llm_providers.model_save')}
                    </button>
                    <button
                      onClick={cancelCtxEdit}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.model_cancel')}
                      className="px-2 py-0.5 text-content-tertiary text-[10px] hover:text-content-primary rounded"
                    >
                      {t('llm_providers.model_cancel')}
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => startCtxEdit(p)}
                    title={t('llm_providers.context_length_edit_hint')}
                    className="text-[11px] font-mono text-content-tertiary hover:text-brand hover:underline cursor-pointer bg-transparent border-0 p-0"
                  >
                    {p.context_length != null ? (
                      t('llm_providers.model_ctx_suffix', {
                        tokens: p.context_length.toLocaleString(),
                      })
                    ) : (
                      <span className="italic">{t('llm_providers.context_length_unset')}</span>
                    )}
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
              {editingCtxId === p.id && ctxError && (
                <p className="text-[10px] text-red-400 mt-1 ml-4">{ctxError}</p>
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
                <button
                  type="button"
                  onClick={() => runConnectionTest(p.id)}
                  disabled={testStates[p.id]?.phase === 'running'}
                  aria-label={`${t('llm_providers.test')} ${p.display_name}`}
                  className="px-2 py-1 text-xs text-content-secondary border border-edge rounded hover:border-brand hover:text-brand disabled:opacity-40"
                >
                  {testStates[p.id]?.phase === 'running' ? '...' : t('llm_providers.test')}
                </button>
                {(() => {
                  const ts = testStates[p.id];
                  if (!ts || ts.phase !== 'done') return null;
                  const color =
                    ts.status === 'ok'
                      ? 'bg-green-500/15 text-green-400 border-green-500/40'
                      : ts.status === 'unreachable'
                        ? 'bg-red-500/15 text-red-400 border-red-500/40'
                        : 'bg-amber-500/15 text-amber-400 border-amber-500/40';
                  const label =
                    ts.status === 'ok'
                      ? t('llm_providers.test_ok', { latency: ts.latency_ms })
                      : ts.status === 'auth_failed'
                        ? t('llm_providers.test_auth_failed')
                        : ts.status === 'unreachable'
                          ? t('llm_providers.test_unreachable')
                          : t('llm_providers.test_model_list_unavailable');
                  return (
                    <span className={`px-2 py-0.5 text-[10px] font-bold rounded border ${color}`}>
                      {label}
                    </span>
                  );
                })()}
              </div>
            </div>
          </div>
        ))}
        {providers.length === 0 && <p className="text-xs text-content-tertiary italic">{t('llm_providers.empty')}</p>}
      </div>
    </SectionCard>
  );
}
