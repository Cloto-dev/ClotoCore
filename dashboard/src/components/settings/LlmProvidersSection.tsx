import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { SectionCard } from './common';

export function LlmProvidersSection() {
  const api = useApi();
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  const [providers, setProviders] = useState<
    Array<{ id: string; display_name: string; has_key: boolean; model_id: string }>
  >([]);
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    api
      .listLlmProviders()
      .then((d) => setProviders(d.providers))
      .catch((e) => { if (import.meta.env.DEV) console.warn('Failed to load LLM providers:', e); });
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
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${p.has_key ? 'bg-green-500' : 'bg-amber-500'}`} />
                <span className="text-xs font-bold text-content-primary">{p.display_name}</span>
                <span className="text-[11px] font-mono text-content-tertiary">{p.model_id}</span>
              </div>
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
                  className="px-3 py-1 bg-brand text-white text-xs font-bold rounded disabled:opacity-40"
                >
                  {saving === p.id ? '...' : tc('save')}
                </button>
                {p.has_key && (
                  <button
                    onClick={() => handleDelete(p.id)}
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
