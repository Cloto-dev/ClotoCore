import { useState, useEffect } from 'react';
import { SectionCard } from './common';
import { useApiKey } from '../../contexts/ApiKeyContext';
import { api } from '../../services/api';

export function LlmProvidersSection() {
  const { apiKey } = useApiKey();
  const [providers, setProviders] = useState<Array<{ id: string; display_name: string; has_key: boolean; model_id: string }>>([]);
  const [keyInputs, setKeyInputs] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<string | null>(null);

  useEffect(() => {
    api.listLlmProviders(apiKey || '').then(d => setProviders(d.providers)).catch(() => {});
  }, [apiKey]);

  const handleSave = async (providerId: string) => {
    if (!keyInputs[providerId]?.trim()) return;
    setSaving(providerId);
    try {
      await api.setLlmProviderKey(providerId, apiKey || '', keyInputs[providerId].trim());
      setKeyInputs(prev => ({ ...prev, [providerId]: '' }));
      const d = await api.listLlmProviders(apiKey);
      setProviders(d.providers);
    } catch { /* ignore */ }
    setSaving(null);
  };

  const handleDelete = async (providerId: string) => {
    await api.deleteLlmProviderKey(providerId, apiKey || '');
    const d = await api.listLlmProviders(apiKey || '');
    setProviders(d.providers);
  };

  return (
    <SectionCard title="LLM Providers">
      <p className="text-[10px] text-content-muted mb-4">API keys are held by the kernel and never exposed to MCP servers (MGP §13.4).</p>
      <div className="space-y-3">
        {providers.map(p => (
          <div key={p.id} className="flex items-center gap-3 p-3 bg-surface-secondary rounded-lg border border-edge-subtle">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full ${p.has_key ? 'bg-green-500' : 'bg-amber-500'}`} />
                <span className="text-xs font-bold text-content-primary">{p.display_name}</span>
                <span className="text-[9px] font-mono text-content-muted">{p.model_id}</span>
              </div>
              <div className="flex gap-2 mt-2">
                <input
                  type="password"
                  value={keyInputs[p.id] || ''}
                  onChange={e => setKeyInputs(prev => ({ ...prev, [p.id]: e.target.value }))}
                  placeholder={p.has_key ? '••••••• (saved)' : 'Enter API key'}
                  className="flex-1 bg-surface-base border border-edge rounded px-2 py-1 text-[10px] font-mono text-content-primary placeholder:text-content-muted"
                />
                <button
                  onClick={() => handleSave(p.id)}
                  disabled={!keyInputs[p.id]?.trim() || saving === p.id}
                  className="px-3 py-1 bg-brand text-white text-[10px] font-bold rounded disabled:opacity-40"
                >
                  {saving === p.id ? '...' : 'Save'}
                </button>
                {p.has_key && (
                  <button
                    onClick={() => handleDelete(p.id)}
                    className="px-2 py-1 text-red-400 text-[10px] hover:bg-red-500/10 rounded"
                  >
                    Clear
                  </button>
                )}
              </div>
            </div>
          </div>
        ))}
        {providers.length === 0 && (
          <p className="text-[10px] text-content-muted italic">No providers configured.</p>
        )}
      </div>
    </SectionCard>
  );
}
