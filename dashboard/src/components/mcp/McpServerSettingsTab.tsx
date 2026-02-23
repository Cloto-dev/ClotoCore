import { useState, useEffect } from 'react';
import { McpServerInfo, McpServerSettings, DefaultPolicy } from '../../types';
import { api } from '../../services/api';
import { Save, RotateCcw } from 'lucide-react';

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

  useEffect(() => {
    loadSettings();
  }, [server.id]);

  async function loadSettings() {
    try {
      setError(null);
      const data = await api.getMcpServerSettings(server.id, apiKey);
      setSettings(data);
      setDefaultPolicy(data.default_policy);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load settings');
    }
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      await api.updateMcpServerSettings(server.id, { default_policy: defaultPolicy }, apiKey);
      await loadSettings();
      onRefresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save settings');
    } finally {
      setSaving(false);
    }
  }

  const hasChanges = settings && defaultPolicy !== settings.default_policy;

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
