import { useState } from 'react';
import { Eye, EyeOff, AlertTriangle } from 'lucide-react';
import { SectionCard, Toggle } from './common';
import { LlmProvidersSection } from './LlmProvidersSection';
import { useApiKey } from '../../contexts/ApiKeyContext';
import { api } from '../../services/api';

export function SecuritySection() {
  const { apiKey, isPersisted, setApiKey, setPersist, forgetApiKey } = useApiKey();
  const [newKey, setNewKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const [confirmInvalidate, setConfirmInvalidate] = useState(false);

  const handleSave = async () => {
    if (!newKey.trim()) return;
    setSaving(true);
    setError('');
    try {
      await api.listCronJobs(newKey.trim());
      setApiKey(newKey.trim());
      setNewKey('');
    } catch {
      setError('Invalid API key');
    } finally {
      setSaving(false);
    }
  };

  const handleInvalidate = async () => {
    if (!apiKey) return;
    try {
      await api.invalidateApiKey(apiKey);
      forgetApiKey();
      setConfirmInvalidate(false);
    } catch {
      setError('Failed to invalidate key');
    }
  };

  return (
    <>
      <SectionCard title="API Key">
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${apiKey ? 'bg-green-500' : 'bg-amber-500'}`} />
            <span className="text-xs text-content-secondary">{apiKey ? 'Configured' : 'Not configured'}</span>
          </div>

          <div className="flex gap-2">
            <div className="relative flex-1">
              <input
                type={showKey ? 'text' : 'password'}
                value={newKey}
                onChange={e => { setNewKey(e.target.value); setError(''); }}
                placeholder={apiKey ? 'Enter new key to replace' : 'Enter API key'}
                className="w-full bg-surface-secondary border border-edge rounded-lg px-3 py-2 text-xs font-mono text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand transition-colors"
              />
              <button
                onClick={() => setShowKey(v => !v)}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-content-muted hover:text-content-secondary"
              >
                {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
            </div>
            <button
              onClick={handleSave}
              disabled={!newKey.trim() || saving}
              className="px-4 py-2 bg-brand text-white text-xs font-bold rounded-lg disabled:opacity-40 hover:bg-brand/90 transition-colors"
            >
              {saving ? '...' : 'Save'}
            </button>
          </div>

          {error && (
            <div className="flex items-center gap-2 text-red-400 text-[10px]">
              <AlertTriangle size={12} />
              {error}
            </div>
          )}

          <Toggle
            enabled={isPersisted}
            onToggle={() => setPersist(!isPersisted)}
            label="Save to this device"
          />

          {apiKey && (
            <div className="pt-3 border-t border-edge">
              {!confirmInvalidate ? (
                <button
                  onClick={() => setConfirmInvalidate(true)}
                  className="text-[10px] text-red-400 hover:text-red-300 font-bold uppercase tracking-widest transition-colors"
                >
                  Invalidate current key (system-wide)
                </button>
              ) : (
                <div className="flex items-center gap-3">
                  <span className="text-[10px] text-red-400">This will revoke the key for all clients.</span>
                  <button onClick={handleInvalidate} className="px-3 py-1 bg-red-500 text-white text-[10px] font-bold rounded-lg">Confirm</button>
                  <button onClick={() => setConfirmInvalidate(false)} className="px-3 py-1 bg-surface-secondary text-content-secondary text-[10px] font-bold rounded-lg border border-edge">Cancel</button>
                </div>
              )}
            </div>
          )}
        </div>
      </SectionCard>

      <LlmProvidersSection />
    </>
  );
}
