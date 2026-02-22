import { useState, useEffect } from 'react';
import { Settings } from 'lucide-react';
import { useApiKey } from '../contexts/ApiKeyContext';
import { api } from '../services/api';
import type { EvolutionParams } from '../types';

export function ParamEditModal({ onClose, onSuccess }: { onClose: () => void; onSuccess: () => void }) {
  const { apiKey } = useApiKey();
  const [error, setError] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [params, setParams] = useState<EvolutionParams | null>(null);
  const [form, setForm] = useState({
    alpha: '', beta: '', theta_min: '', gamma: '', min_interactions: '',
  });

  useEffect(() => {
    let cancelled = false;
    api.getEvolutionParams().then(p => {
      if (cancelled) return;
      setParams(p);
      setForm({
        alpha: String(p.alpha),
        beta: String(p.beta),
        theta_min: String(p.theta_min),
        gamma: String(p.gamma),
        min_interactions: String(p.min_interactions),
      });
    }).catch(e => { if (!cancelled) setError(e.message); });
    return () => { cancelled = true; };
  }, []);

  const handleSave = async () => {
    if (!apiKey) return;
    setIsLoading(true);
    setError('');
    try {
      const alpha = parseFloat(form.alpha);
      const beta = parseFloat(form.beta);
      const theta_min = parseFloat(form.theta_min);
      const gamma = parseFloat(form.gamma);
      const min_interactions = parseInt(form.min_interactions);
      const values = [alpha, beta, theta_min, gamma, min_interactions];
      if (values.some(v => isNaN(v) || !isFinite(v))) {
        setError('All fields must be valid numbers');
        setIsLoading(false);
        return;
      }
      const update: EvolutionParams = { alpha, beta, theta_min, gamma, min_interactions, weights: params!.weights };
      await api.updateEvolutionParams(update, apiKey);
      onSuccess();
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'Failed to update params');
    } finally {
      setIsLoading(false);
    }
  };

  if (!params) return null;

  const fields: { key: keyof typeof form; label: string }[] = [
    { key: 'alpha', label: 'Alpha (growth)' },
    { key: 'beta', label: 'Beta (regression)' },
    { key: 'theta_min', label: 'Theta Min' },
    { key: 'gamma', label: 'Gamma (rebalance)' },
    { key: 'min_interactions', label: 'Min Interactions' },
  ];

  return (
    <div
      className="absolute inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Edit Evolution Params"
      onKeyDown={e => e.key === 'Escape' && onClose()}
    >
      <div className="bg-surface-primary rounded-2xl shadow-2xl p-6 w-80 space-y-3 border border-edge">
        <div className="flex items-center gap-2">
          <Settings size={16} className="text-content-secondary" />
          <h3 className="text-sm font-bold text-content-primary">Edit Evolution Params</h3>
        </div>

        {fields.map(f => (
          <div key={f.key}>
            <label className="text-[9px] font-mono text-content-tertiary uppercase">{f.label}</label>
            <input
              type="number"
              step="any"
              value={form[f.key]}
              onChange={e => setForm(prev => ({ ...prev, [f.key]: e.target.value }))}
              className="w-full px-2 py-1.5 rounded-lg border border-edge bg-glass-subtle text-xs font-mono focus:outline-none focus:border-purple-400"
            />
          </div>
        ))}

        {!apiKey && (
          <p className="text-[10px] text-amber-400 font-mono pt-2 border-t border-edge">
            API Key が未設定です。画面上部の 🔒 から設定してください。
          </p>
        )}

        {error && <p className="text-[10px] text-red-400 font-medium">{error}</p>}

        <div className="flex gap-2 pt-1">
          <button
            onClick={onClose}
            className="flex-1 py-2 rounded-xl border border-edge text-xs font-bold text-content-secondary hover:bg-glass-subtle transition-all"
            disabled={isLoading}
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!apiKey || isLoading}
            className="flex-1 py-2 rounded-xl bg-purple-600 hover:bg-purple-700 text-white text-xs font-bold transition-all disabled:opacity-50"
          >
            {isLoading ? 'Saving...' : 'Save'}
          </button>
        </div>
      </div>
    </div>
  );
}
