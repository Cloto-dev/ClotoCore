import { useState, useEffect } from 'react';
import { Settings, FolderOpen, CheckCircle2, Lock, Unlock, Plus } from 'lucide-react';
import { PluginManifest } from '../types';
import { api } from '../services/api';
import { isTauri, openFileDialog } from '../lib/tauri';
import { useApiKey } from '../contexts/ApiKeyContext';

export const ALL_PERMISSIONS = [
  { name: 'NetworkAccess',    label: 'Network Access',     enforced: true,  desc: 'HTTP requests to whitelisted hosts' },
  { name: 'InputControl',     label: 'Input Control',      enforced: true,  desc: 'Keyboard / mouse control' },
  { name: 'FileRead',         label: 'File Read',          enforced: false, desc: 'Read files from disk (declared only)' },
  { name: 'FileWrite',        label: 'File Write',         enforced: false, desc: 'Write files to disk (declared only)' },
  { name: 'ProcessExecution', label: 'Process Execution',  enforced: false, desc: 'Execute system processes (declared only)' },
  { name: 'VisionRead',       label: 'Vision Read',        enforced: false, desc: 'Screen / camera capture (declared only)' },
  { name: 'MemoryRead',       label: 'Memory Read',        enforced: false, desc: 'Read agent memory (declared only)' },
  { name: 'MemoryWrite',      label: 'Memory Write',       enforced: false, desc: 'Write agent memory (declared only)' },
  { name: 'AdminAccess',      label: 'Admin Access',       enforced: false, desc: 'Administrative operations (declared only)' },
];

export function PluginConfigModal({ plugin, onClose }: { plugin: PluginManifest, onClose: () => void }) {
  const { apiKey } = useApiKey();
  const [config, setConfig] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState('');
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'error'>('idle');
  const [permissions, setPermissions] = useState<string[]>([]);
  const [permLoading, setPermLoading] = useState(true);
  const [permError, setPermError] = useState('');
  const [activeTab, setActiveTab] = useState<'config' | 'permissions'>('config');

  useEffect(() => {
    if (!apiKey) return;
    setLoading(true);
    setFetchError('');
    api.getPluginConfig(plugin.id, apiKey)
      .then(setConfig)
      .catch((e: any) => setFetchError(e?.message || 'Failed to load config'))
      .finally(() => setLoading(false));
    setPermLoading(true);
    api.getPluginPermissions(plugin.id, apiKey)
      .then(setPermissions)
      .catch((e: any) => setPermError(e?.message || 'Failed to load permissions'))
      .finally(() => setPermLoading(false));
  }, [plugin.id, apiKey]);

  const save = async (key: string, value: string) => {
    const newConfig = { ...config, [key]: value };
    await api.updatePluginConfig(plugin.id, { key, value }, apiKey);
    setConfig(newConfig);
  };

  const handleGrant = async (permission: string) => {
    try {
      await api.grantPermission(plugin.id, permission, apiKey);
      setPermissions(prev => [...prev, permission]);
    } catch (e: any) {
      setPermError(e?.message || 'Failed to grant permission');
    }
  };

  const handleRevoke = async (permission: string) => {
    try {
      await api.revokePermission(plugin.id, permission, apiKey);
      setPermissions(prev => prev.filter(p => p !== permission));
    } catch (e: any) {
      setPermError(e?.message || 'Failed to revoke permission');
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm p-4" onClick={onClose}>
      <div className="bg-surface-primary rounded-2xl w-full max-w-lg p-6 shadow-2xl animate-in zoom-in-95 duration-200" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-bold mb-4 flex items-center gap-2 text-content-primary">
          <Settings size={18} className="text-brand" />
          {plugin.name}
        </h3>

        {/* Tabs */}
        <div className="flex gap-1 mb-4 bg-surface-secondary rounded-lg p-1">
          {(['config', 'permissions'] as const).map(tab => (
            <button
              key={tab}
              onClick={() => setActiveTab(tab)}
              className={`flex-1 py-1.5 text-[11px] font-bold rounded-md transition-all ${
                activeTab === tab ? 'bg-surface-primary text-content-primary shadow-sm' : 'text-content-tertiary hover:text-content-secondary'
              }`}
            >
              {tab === 'config' ? '⚙ Config' : '🔐 Permissions'}
            </button>
          ))}
        </div>

        {!apiKey ? (
          <div className="p-4 bg-amber-500/10 text-amber-400 text-xs rounded-lg font-mono border border-amber-500/20">
            API Key が設定されていません。画面右上の 🔒 から設定してください。
          </div>
        ) : activeTab === 'config' ? (
          fetchError ? (
            <div className="mb-3 p-3 bg-red-500/10 text-red-400 text-xs rounded-lg font-mono">{fetchError}</div>
          ) : loading ? (
            <div className="p-8 text-center text-content-tertiary font-mono text-xs">Loading config...</div>
          ) : (
            <div className="space-y-4">
              {plugin.required_config_keys.length > 0 ? plugin.required_config_keys.map(key => {
                const isPathKey = /path|script|file|dir/i.test(key);
                const isSecretKey = key.includes('key') || key.includes('password');
                return (
                  <div key={key}>
                    <label className="block text-xs font-bold text-content-secondary mb-1 uppercase tracking-wider">{key}</label>
                    <div className="flex gap-2">
                      <input
                        type={isSecretKey ? 'password' : 'text'}
                        value={config[key] || ''}
                        onChange={e => setConfig(prev => ({ ...prev, [key]: e.target.value }))}
                        onBlur={e => save(key, e.target.value)}
                        className="flex-1 px-3 py-2 rounded-lg border border-edge text-sm font-mono text-content-primary bg-surface-base placeholder:text-content-muted focus:outline-none focus:border-brand focus:ring-1 focus:ring-brand"
                        placeholder={`Enter ${key}...`}
                      />
                      {isPathKey && isTauri && (
                        <button
                          type="button"
                          onClick={async () => {
                            const filters = key.includes('script') || key.includes('python')
                              ? [{ name: 'Python Scripts', extensions: ['py'] }]
                              : undefined;
                            const selected = await openFileDialog({ title: `Select ${key}`, filters });
                            if (selected) { setConfig(prev => ({ ...prev, [key]: selected })); save(key, selected); }
                          }}
                          className="px-3 py-2 rounded-lg border border-edge bg-surface-base hover:bg-brand/10 hover:border-brand/50 text-content-secondary hover:text-brand transition-colors"
                          title="Browse files"
                        >
                          <FolderOpen size={16} />
                        </button>
                      )}
                    </div>
                  </div>
                );
              }) : (
                <div className="p-4 bg-surface-base text-content-tertiary text-xs rounded-lg text-center font-mono border border-edge-subtle border-dashed">
                  No configuration required for this plugin.
                </div>
              )}
            </div>
          )
        ) : (
          /* Permissions tab */
          permLoading ? (
            <div className="p-8 text-center text-content-tertiary font-mono text-xs">Loading permissions...</div>
          ) : (
            <div className="space-y-1">
              {permError && <div className="mb-2 p-2 bg-red-500/10 text-red-400 text-xs rounded-lg">{permError}</div>}
              <p className="text-[9px] text-content-muted mb-3 font-mono">
                ✅ enforced = system actively blocks access without this permission<br/>
                ⚠ declared = metadata only, system enforcement not yet implemented
              </p>
              {ALL_PERMISSIONS.map(p => {
                const granted = permissions.includes(p.name);
                return (
                  <div key={p.name} className={`flex items-center gap-3 p-2.5 rounded-xl border transition-all ${
                    granted ? 'bg-emerald-500/10 border-emerald-500/20' : 'bg-surface-secondary border-edge-subtle'
                  }`}>
                    <div className={`w-4 h-4 rounded-full flex items-center justify-center shrink-0 ${granted ? 'bg-emerald-500' : 'bg-content-muted/20'}`}>
                      {granted ? <CheckCircle2 size={10} className="text-white" /> : null}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className={`text-[11px] font-bold ${granted ? 'text-emerald-500' : 'text-content-secondary'}`}>{p.label}</span>
                        <span className={`text-[8px] px-1 rounded font-mono ${p.enforced ? 'bg-emerald-500/20 text-emerald-500' : 'bg-amber-500/20 text-amber-500'}`}>
                          {p.enforced ? '✅' : '⚠'}
                        </span>
                      </div>
                      <p className="text-[9px] text-content-muted">{p.desc}</p>
                    </div>
                    <button
                      onClick={() => granted ? handleRevoke(p.name) : handleGrant(p.name)}
                      className={`p-1.5 rounded-lg transition-all text-[10px] font-bold flex items-center gap-1 ${
                        granted
                          ? 'bg-red-500/10 text-red-500 hover:bg-red-500/20'
                          : 'bg-emerald-500/10 text-emerald-500 hover:bg-emerald-500/20'
                      }`}
                      title={granted ? `Revoke ${p.name}` : `Grant ${p.name}`}
                    >
                      {granted ? <><Unlock size={10} /> Revoke</> : <><Plus size={10} /> Grant</>}
                    </button>
                  </div>
                );
              })}
            </div>
          )
        )}

        <div className="mt-6 flex justify-end gap-3">
          <button onClick={onClose} className="px-6 py-2 bg-surface-secondary text-content-secondary rounded-lg text-xs font-bold hover:bg-surface-secondary transition-colors tracking-wide">
            CANCEL
          </button>
          <button
            onClick={async (e) => {
              e.preventDefault();
              setSaveState('saving');

              try {
                await Promise.all(
                  Object.entries(config).map(([key, value]) =>
                    api.updatePluginConfig(plugin.id, { key, value }, apiKey)
                  )
                );
                onClose();
              } catch (err) {
                console.error("Failed to save config:", err);
                setSaveState('error');
              }
            }}
            disabled={saveState === 'saving'}
            className="px-6 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:bg-[var(--brand-hover)] transition-all shadow-md shadow-brand/20 tracking-wide disabled:opacity-50"
          >
            {saveState === 'saving' ? 'SAVING...' : saveState === 'error' ? 'SAVE ERROR (TRY AGAIN)' : 'SAVE & CLOSE'}
          </button>
        </div>
      </div>
    </div>
  );
}
