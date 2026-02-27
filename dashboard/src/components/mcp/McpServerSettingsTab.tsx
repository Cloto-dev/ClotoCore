import { useState, useEffect, useCallback } from 'react';
import { McpServerInfo, McpServerSettings, DefaultPolicy } from '../../types';
import { api } from '../../services/api';
import { Save, RotateCcw, Plus, X, Eye, EyeOff } from 'lucide-react';

interface Props {
  server: McpServerInfo;
  apiKey: string;
  onRefresh: () => void;
}

export function McpServerSettingsTab({ server, apiKey, onRefresh }: Props) {
  const [settings, setSettings] = useState<McpServerSettings | null>(null);
  const [defaultPolicy, setDefaultPolicy] = useState<DefaultPolicy>('opt-in');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Env editor state
  const [envEntries, setEnvEntries] = useState<{ key: string; value: string }[]>([]);
  const [initialEnvKeys, setInitialEnvKeys] = useState<Set<string>>(new Set());
  const [newKey, setNewKey] = useState('');
  const [newValue, setNewValue] = useState('');
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set());

  const loadSettings = useCallback(async () => {
    try {
      setError(null);
      const data = await api.getMcpServerSettings(server.id, apiKey);
      setSettings(data);
      setDefaultPolicy(data.default_policy);

      // Load env entries
      const env = data.env ?? {};
      const entries = Object.entries(env).map(([key, value]) => ({ key, value }));
      setEnvEntries(entries);
      setInitialEnvKeys(new Set(Object.keys(env)));
      setVisibleKeys(new Set());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load settings');
    }
  }, [server.id, apiKey]);

  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      // Build env object from entries
      const envObj: Record<string, string> = {};
      for (const entry of envEntries) {
        if (entry.key.trim()) {
          envObj[entry.key.trim()] = entry.value;
        }
      }

      await api.updateMcpServerSettings(
        server.id,
        { default_policy: defaultPolicy, env: envObj },
        apiKey,
      );
      await loadSettings();
      onRefresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save settings');
    } finally {
      setSaving(false);
    }
  }

  const addEnvEntry = () => {
    const trimmedKey = newKey.trim();
    if (!trimmedKey) return;
    if (envEntries.some(e => e.key === trimmedKey)) return;
    setEnvEntries(prev => [...prev, { key: trimmedKey, value: newValue }]);
    setNewKey('');
    setNewValue('');
  };

  const removeEnvEntry = (key: string) => {
    setEnvEntries(prev => prev.filter(e => e.key !== key));
  };

  const updateEnvValue = (key: string, value: string) => {
    setEnvEntries(prev => prev.map(e => e.key === key ? { ...e, value } : e));
  };

  const toggleVisibility = (key: string) => {
    setVisibleKeys(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  // Detect changes
  const envChanged = (() => {
    if (!settings) return false;
    const currentKeys = new Set(envEntries.map(e => e.key));
    if (currentKeys.size !== initialEnvKeys.size) return true;
    for (const key of initialEnvKeys) {
      if (!currentKeys.has(key)) return true;
    }
    // Check if any values were changed (not "***")
    return envEntries.some(e => e.value !== '***');
  })();

  const hasChanges = (settings && defaultPolicy !== settings.default_policy) || envChanged;

  return (
    <div className="p-4 space-y-4">
      {error && (
        <div className="p-2 text-[10px] font-mono text-red-500 bg-red-500/10 rounded border border-red-500/20">
          {error}
        </div>
      )}

      {/* Server Configuration */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">Server Configuration</h3>
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-muted w-20">Command</label>
            <span className="text-xs font-mono text-content-secondary bg-glass rounded px-2 py-1 flex-1">
              {settings?.command ?? server.command ?? '—'}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-muted w-20">Args</label>
            <span className="text-xs font-mono text-content-secondary bg-glass rounded px-2 py-1 flex-1 truncate">
              {(settings?.args ?? server.args ?? []).join(' ') || '(none)'}
            </span>
          </div>
          <div className="flex items-center gap-2">
            <label className="text-[10px] font-mono text-content-muted w-20">Transport</label>
            <span className="text-xs font-mono text-content-secondary">stdio</span>
          </div>
        </div>
      </section>

      {/* Environment Variables */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">Environment Variables</h3>
        <div className="space-y-2">
          {envEntries.map(entry => (
            <div key={entry.key} className="flex items-center gap-2">
              <span className="text-[10px] font-mono text-content-secondary w-40 truncate shrink-0" title={entry.key}>
                {entry.key}
              </span>
              <div className="relative flex-1">
                <input
                  type={visibleKeys.has(entry.key) ? 'text' : 'password'}
                  value={entry.value}
                  onChange={e => updateEnvValue(entry.key, e.target.value)}
                  placeholder={initialEnvKeys.has(entry.key) ? 'Unchanged' : ''}
                  className="w-full text-xs font-mono bg-surface-secondary border border-edge rounded px-2 py-1 pr-7 text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand transition-colors"
                />
                <button
                  onClick={() => toggleVisibility(entry.key)}
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 text-content-muted hover:text-content-secondary"
                >
                  {visibleKeys.has(entry.key) ? <EyeOff size={12} /> : <Eye size={12} />}
                </button>
              </div>
              <button
                onClick={() => removeEnvEntry(entry.key)}
                className="p-1 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-colors shrink-0"
                title="Remove"
              >
                <X size={12} />
              </button>
            </div>
          ))}

          {/* Add new variable */}
          <div className="flex items-center gap-2 pt-1 border-t border-edge/50">
            <input
              type="text"
              value={newKey}
              onChange={e => setNewKey(e.target.value.toUpperCase())}
              placeholder="KEY"
              className="w-40 text-[10px] font-mono bg-surface-secondary border border-edge rounded px-2 py-1 text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand transition-colors shrink-0"
              onKeyDown={e => e.key === 'Enter' && addEnvEntry()}
            />
            <input
              type="password"
              value={newValue}
              onChange={e => setNewValue(e.target.value)}
              placeholder="value"
              className="flex-1 text-xs font-mono bg-surface-secondary border border-edge rounded px-2 py-1 text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand transition-colors"
              onKeyDown={e => e.key === 'Enter' && addEnvEntry()}
            />
            <button
              onClick={addEnvEntry}
              disabled={!newKey.trim()}
              className="p-1 rounded text-brand hover:bg-brand/10 transition-colors disabled:opacity-30 disabled:cursor-not-allowed shrink-0"
              title="Add"
            >
              <Plus size={14} />
            </button>
          </div>

          {envEntries.length === 0 && (
            <p className="text-[9px] font-mono text-content-muted py-2">
              No environment variables configured. Add API keys or other settings above.
            </p>
          )}
        </div>
      </section>

      {/* Default Policy */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">Default Policy</h3>
        <select
          value={defaultPolicy}
          onChange={e => setDefaultPolicy(e.target.value as DefaultPolicy)}
          className="text-xs font-mono bg-glass border border-edge rounded px-2 py-1 text-content-primary"
        >
          <option value="opt-in">opt-in (deny by default)</option>
          <option value="opt-out">opt-out (allow by default)</option>
        </select>
        <p className="mt-1 text-[9px] font-mono text-content-muted">
          {defaultPolicy === 'opt-in'
            ? 'Agents must be explicitly granted access to tools.'
            : 'All agents have access unless explicitly denied.'}
        </p>
      </section>

      {/* Manifest */}
      <section>
        <h3 className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary mb-2">Manifest</h3>
        <div className="space-y-1">
          <div className="flex gap-2 text-[10px] font-mono">
            <span className="text-content-muted w-16">ID</span>
            <span className="text-content-secondary">{server.id}</span>
          </div>
          <div className="flex gap-2 text-[10px] font-mono">
            <span className="text-content-muted w-16">Tools</span>
            <span className="text-content-secondary">{server.tools.join(', ') || '(none)'}</span>
          </div>
          {settings?.description && (
            <div className="flex gap-2 text-[10px] font-mono">
              <span className="text-content-muted w-16">Desc</span>
              <span className="text-content-secondary">{settings.description}</span>
            </div>
          )}
        </div>
      </section>

      {/* Actions */}
      <div className="flex gap-2 pt-2 border-t border-edge">
        <button
          onClick={handleSave}
          disabled={saving || !hasChanges}
          className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 disabled:cursor-not-allowed transition-colors border border-brand/20"
        >
          <Save size={10} /> {saving ? 'Saving...' : 'Save Changes'}
        </button>
        <button
          onClick={loadSettings}
          className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge"
        >
          <RotateCcw size={10} /> Reset
        </button>
      </div>
    </div>
  );
}
