import { RefreshCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { Segmented, Select, SettingsGroup } from './common';

const MODEL_ID_MAX_LEN = 200;

type ThinkingMode = 'auto' | 'on' | 'off';

// Backing-engine state per provider, computed by the kernel by joining the
// provider row against the registered MCP engine servers:
//   connected    — an engine server is registered and operational.
//   disconnected — an engine server is registered but down/stopped.
//   uninstalled  — no engine server, but the user configured this provider →
//                  keep + warn (settings are never dropped when an engine goes away).
//   catalog_only — pristine seeded provider with no engine and no user config →
//                  hidden from the UI (not a "real" engine).
type EngineStatus = 'connected' | 'disconnected' | 'uninstalled' | 'catalog_only';

type Provider = {
  id: string;
  display_name: string;
  has_key: boolean;
  model_id: string;
  context_length: number | null;
  thinking_mode: ThinkingMode;
  engine_status: EngineStatus;
  configured: boolean;
  // Provider-specific model-ID example (e.g. LM Studio's `org/name` vs Ollama's
  // `name:tag`). Supplied by the backend so the dashboard keeps no hardcoded
  // provider list of its own; null for providers without a baked-in example.
  model_placeholder: string | null;
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

  // Thinking mode 3-way toggle state — per-provider user-facing override.
  const [thinkingSavingId, setThinkingSavingId] = useState<string | null>(null);

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

  const handleCommitKey = async (providerId: string) => {
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

  const handleModelKeyDown = (e: React.KeyboardEvent<HTMLInputElement>, providerId: string) => {
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

  const commitThinkingMode = async (providerId: string, value: ThinkingMode) => {
    setThinkingSavingId(providerId);
    try {
      await api.setLlmProviderThinkingMode(providerId, value);
      const d = await api.listLlmProviders();
      setProviders(d.providers);
    } catch (e) {
      if (import.meta.env.DEV) console.warn('setLlmProviderThinkingMode failed:', e);
    } finally {
      setThinkingSavingId(null);
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

  /** The one line a model reads as in the list: the id, then what is known about it. */
  const modelLabel = (m: ModelOption): string => {
    const parts: string[] = [m.id];
    if (m.name) parts.push(`— ${m.name}`);
    // Prefer showing the actually loaded n_ctx (what LM Studio will accept
    // right now) alongside the model's native maximum so the user can see the
    // gap at a glance.
    if (
      m.loaded &&
      m.loaded_context_length &&
      m.max_context_length &&
      m.loaded_context_length !== m.max_context_length
    ) {
      parts.push(
        `· ${t('llm_providers.model_ctx_loaded_of_max', {
          loaded: m.loaded_context_length.toLocaleString(),
          max: m.max_context_length.toLocaleString(),
        })}`,
      );
    } else if (m.loaded && m.loaded_context_length) {
      parts.push(`· ${t('llm_providers.model_ctx_suffix', { tokens: m.loaded_context_length.toLocaleString() })}`);
    } else if (m.max_context_length) {
      parts.push(`· ${t('llm_providers.model_ctx_max_suffix', { tokens: m.max_context_length.toLocaleString() })}`);
    }
    if (m.loaded) parts.push(`· ${t('llm_providers.model_loaded')}`);
    return parts.join(' ');
  };

  // Only providers whose backing engine actually exists are shown. `uninstalled`
  // is shown (with a warning) so a user's saved settings remain visible and
  // editable after an engine is removed; `catalog_only` (pristine seed, no
  // engine) is hidden — the LLM Providers list is derived from real engines,
  // not from the seeded provider catalog.
  const visibleProviders = providers.filter((p) => p.engine_status !== 'catalog_only');

  return (
    <SettingsGroup title={t('llm_providers.title')}>
      <p className="gdesc">{t('llm_providers.desc')}</p>
      {visibleProviders.length === 0 ? (
        <p className="says">{t('llm_providers.empty_no_engines')}</p>
      ) : (
        visibleProviders.map((p) => {
          const ts = testStates[p.id];
          const done = ts?.phase === 'done' ? ts : null;
          const testLabel = !done
            ? null
            : done.status === 'ok'
              ? t('llm_providers.test_ok', { latency: done.latency_ms })
              : done.status === 'auth_failed'
                ? t('llm_providers.test_auth_failed')
                : done.status === 'unreachable'
                  ? t('llm_providers.test_unreachable')
                  : t('llm_providers.test_model_list_unavailable');
          const testTone = !done ? '' : done.status === 'ok' ? ' ok' : ' bad';
          return (
            <div className="prov" key={p.id}>
              <div className="n">
                <span className={p.has_key ? 'dot ok' : 'dot no'} aria-hidden="true" />
                <span className="nm">{p.display_name}</span>
                {p.engine_status === 'disconnected' && (
                  <span className="warn" title={t('llm_providers.engine_disconnected_hint')}>
                    {t('llm_providers.engine_disconnected')}
                  </span>
                )}
                {p.engine_status === 'uninstalled' && (
                  <span className="gone" title={t('llm_providers.engine_uninstalled_hint')}>
                    {t('llm_providers.engine_uninstalled')}
                  </span>
                )}
              </div>

              {/* The model this provider answers with. */}
              <div className="fields">
                <span className="lbl">{t('llm_providers.model_label')}</span>
                {editingModelId === p.id ? (
                  <>
                    {modelList?.status === 'ready' && modelList.models.length > 0 ? (
                      <Select
                        label={`${p.display_name} ${t('llm_providers.model_label')}`}
                        value={modelInput}
                        onChange={setModelInput}
                        placeholder={t('llm_providers.model_placeholder')}
                        options={[
                          // Preserve a currently-saved model that isn't in the list (e.g. unloaded)
                          ...(modelInput && !modelList.models.some((m) => m.id === modelInput)
                            ? [{ value: modelInput, label: modelInput }]
                            : []),
                          ...modelList.models.map((m) => ({ value: m.id, label: modelLabel(m) })),
                        ]}
                      />
                    ) : (
                      <input
                        ref={modelInputRef}
                        className="in mono"
                        type="text"
                        value={modelInput}
                        maxLength={MODEL_ID_MAX_LEN}
                        onChange={(e) => setModelInput(e.target.value)}
                        onKeyDown={(e) => handleModelKeyDown(e, p.id)}
                        aria-label={`${p.display_name} ${t('llm_providers.model_label')}`}
                        placeholder={
                          modelList?.status === 'loading'
                            ? t('llm_providers.model_dropdown_loading')
                            : p.model_placeholder
                              ? t('llm_providers.model_placeholder_ex', { example: p.model_placeholder })
                              : t('llm_providers.model_placeholder')
                        }
                      />
                    )}
                    <button
                      type="button"
                      className="icb"
                      onClick={() => refreshModels(p.id)}
                      disabled={modelList?.status === 'loading'}
                      aria-label={t('llm_providers.model_refresh')}
                      title={t('llm_providers.model_refresh')}
                    >
                      <RefreshCw size={13} className={modelList?.status === 'loading' ? 'animate-spin' : ''} />
                    </button>
                    <button
                      type="button"
                      className="btn pri"
                      onClick={() => commitModelEdit(p.id)}
                      disabled={modelSaving || !modelInput.trim()}
                      aria-label={t('llm_providers.model_save')}
                    >
                      {modelSaving ? '...' : t('llm_providers.model_save')}
                    </button>
                    <button
                      type="button"
                      className="btn"
                      onClick={cancelModelEdit}
                      disabled={modelSaving}
                      aria-label={t('llm_providers.model_cancel')}
                    >
                      {t('llm_providers.model_cancel')}
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    onClick={() => startModelEdit(p)}
                    title={t('llm_providers.model_edit_hint')}
                  >
                    {p.model_id || t('llm_providers.model_unset')}
                  </button>
                )}
              </div>
              {editingModelId === p.id && modelError && <p className="says bad">{modelError}</p>}
              {editingModelId === p.id && modelList?.status === 'fallback' && !modelError && (
                <p className="says">
                  {t('llm_providers.model_dropdown_error', { code: modelList.errorCode ?? 'unknown' })}
                </p>
              )}

              {/* How much of a conversation it will accept. */}
              <div className="fields">
                <span className="lbl">{t('llm_providers.context_length_label')}</span>
                {editingCtxId === p.id ? (
                  <>
                    <input
                      ref={ctxInputRef}
                      className="in ctx num"
                      type="number"
                      min={1}
                      step={1}
                      value={ctxInput}
                      onChange={(e) => setCtxInput(e.target.value)}
                      onKeyDown={(e) => handleCtxKeyDown(e, p.id)}
                      aria-label={`${p.display_name} ${t('llm_providers.context_length_label')}`}
                      placeholder={t('llm_providers.context_length_placeholder')}
                    />
                    <button
                      type="button"
                      className="btn"
                      onClick={() => detectCtxFromProbe(p.id)}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.context_length_detect')}
                      title={t('llm_providers.context_length_detect')}
                    >
                      {t('llm_providers.context_length_detect')}
                    </button>
                    <button
                      type="button"
                      className="btn pri"
                      onClick={() => commitCtxEdit(p.id)}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.model_save')}
                    >
                      {ctxSaving ? '...' : t('llm_providers.model_save')}
                    </button>
                    <button
                      type="button"
                      className="btn"
                      onClick={cancelCtxEdit}
                      disabled={ctxSaving}
                      aria-label={t('llm_providers.model_cancel')}
                    >
                      {t('llm_providers.model_cancel')}
                    </button>
                  </>
                ) : (
                  <button
                    type="button"
                    className="btn"
                    onClick={() => startCtxEdit(p)}
                    title={t('llm_providers.context_length_edit_hint')}
                  >
                    {p.context_length != null
                      ? t('llm_providers.model_ctx_suffix', { tokens: p.context_length.toLocaleString() })
                      : t('llm_providers.context_length_unset')}
                  </button>
                )}
              </div>
              {editingCtxId === p.id && ctxError && <p className="says bad">{ctxError}</p>}

              {/* Whether it is allowed to think first. */}
              <div className="fields">
                <span className="lbl" title={t('llm_providers.thinking_hint')}>
                  {t('llm_providers.thinking_label')}
                </span>
                <Segmented<ThinkingMode>
                  label={`${p.display_name} ${t('llm_providers.thinking_label')}`}
                  value={p.thinking_mode ?? 'auto'}
                  onChange={(mode) => {
                    if (thinkingSavingId === p.id) return;
                    commitThinkingMode(p.id, mode);
                  }}
                  options={[
                    { value: 'auto', label: t('llm_providers.thinking_auto') },
                    { value: 'on', label: t('llm_providers.thinking_on') },
                    { value: 'off', label: t('llm_providers.thinking_off') },
                  ]}
                />
              </div>

              {/* The key it is reached with, and whether it answers. */}
              <div className="fields">
                <span className="lbl">{t('llm_providers.key_label')}</span>
                <input
                  className="in mono"
                  type="password"
                  value={keyInputs[p.id] || ''}
                  onChange={(e) => setKeyInputs((prev) => ({ ...prev, [p.id]: e.target.value }))}
                  aria-label={`${p.display_name} ${t('llm_providers.key_label')}`}
                  placeholder={p.has_key ? t('llm_providers.placeholder_saved') : t('llm_providers.placeholder_new')}
                />
                <button
                  type="button"
                  className="btn pri"
                  onClick={() => handleCommitKey(p.id)}
                  disabled={!keyInputs[p.id]?.trim() || saving === p.id}
                  aria-label={`${tc('save')} ${p.display_name}`}
                >
                  {saving === p.id ? '...' : tc('save')}
                </button>
                {p.has_key && (
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() => handleDelete(p.id)}
                    aria-label={`${t('llm_providers.clear')} ${p.display_name}`}
                  >
                    {t('llm_providers.clear')}
                  </button>
                )}
                <button
                  type="button"
                  className="btn"
                  onClick={() => runConnectionTest(p.id)}
                  disabled={ts?.phase === 'running'}
                  aria-label={`${t('llm_providers.test')} ${p.display_name}`}
                >
                  {ts?.phase === 'running' ? '...' : t('llm_providers.test')}
                </button>
                {testLabel && <span className={`says${testTone}`}>{testLabel}</span>}
              </div>
            </div>
          );
        })
      )}
    </SettingsGroup>
  );
}
