import React, { useState, useEffect } from 'react';
import { Puzzle, Shield, CheckCircle2, AlertTriangle, Save, Filter, Settings, ExternalLink, Terminal, ChevronDown, ChevronRight, Hash, FolderOpen, Database, MousePointer2, Lock } from 'lucide-react';
import { PluginManifest, PluginCategory } from '../types';
import { api } from '../services/api';
import { ServiceTypeIcon } from '../lib/pluginUtils';
import { isTauri, openFileDialog } from '../lib/tauri';

function ConfigModal({ plugin, onClose }: { plugin: PluginManifest, onClose: () => void }) {
  const [apiKey, setApiKey] = useState('');
  const [config, setConfig] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [fetchError, setFetchError] = useState('');
  // H-19: Use React state instead of direct DOM manipulation for save button
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'error'>('idle');

  const loadConfig = async (key: string) => {
    setLoading(true);
    setFetchError('');
    try {
      const data = await api.getPluginConfig(plugin.id, key);
      setConfig(data);
    } catch (e: any) {
      setFetchError(e?.message || 'Failed to load config');
    } finally {
      setLoading(false);
    }
  };

  const save = async (key: string, value: string) => {
    const newConfig = { ...config, [key]: value };
    await api.updatePluginConfig(plugin.id, { key, value }, apiKey);
    setConfig(newConfig);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm p-4" onClick={onClose}>
      <div className="bg-surface-primary rounded-2xl w-full max-w-lg p-6 shadow-2xl animate-in zoom-in-95 duration-200" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-bold mb-4 flex items-center gap-2 text-content-primary">
          <Settings size={18} className="text-brand" />
          Configure {plugin.name}
        </h3>
        
        {/* API Key input — always shown; load is triggered manually */}
        <div className="flex gap-2 mb-4">
          <div className="relative flex-1">
            <Lock size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-content-muted" />
            <input
              type="password"
              value={apiKey}
              onChange={e => setApiKey(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && apiKey && loadConfig(apiKey)}
              placeholder="API Key (required)"
              className="w-full pl-7 pr-3 py-1.5 rounded-lg border border-edge text-xs font-mono text-content-primary bg-surface-base placeholder:text-content-muted focus:outline-none focus:border-brand"
            />
          </div>
          <button
            onClick={() => loadConfig(apiKey)}
            disabled={!apiKey || loading}
            className="px-3 py-1.5 rounded-lg bg-brand text-white text-xs font-bold disabled:opacity-40 hover:bg-brand/90 transition-colors"
          >
            {loading ? '...' : 'Load'}
          </button>
        </div>

        {fetchError && (
          <div className="mb-3 p-3 bg-red-500/10 text-red-400 text-xs rounded-lg font-mono">{fetchError}</div>
        )}

        {loading ? (
          <div className="p-8 text-center text-content-tertiary font-mono text-xs">Loading config...</div>
        ) : Object.keys(config).length > 0 || plugin.required_config_keys.length > 0 ? (
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
                          const selected = await openFileDialog({
                            title: `Select ${key}`,
                            filters,
                          });
                          if (selected) {
                            setConfig(prev => ({ ...prev, [key]: selected }));
                            save(key, selected);
                          }
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
        ) : null}

        <div className="mt-6 flex justify-end gap-3">
          <button onClick={onClose} className="px-6 py-2 bg-surface-secondary text-content-secondary rounded-lg text-xs font-bold hover:bg-surface-secondary transition-colors tracking-wide">
            CANCEL
          </button>
          <button
            onClick={async (e) => {
              e.preventDefault();
              // H-19: Use React state instead of direct DOM manipulation
              setSaveState('saving');

              try {
                // Save all keys in parallel with correct format { key, value }
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
            className="px-6 py-2 bg-brand text-white rounded-lg text-xs font-bold hover:bg-[#1e3bb3] transition-all shadow-md shadow-brand/20 tracking-wide disabled:opacity-50"
          >
            {saveState === 'saving' ? 'SAVING...' : saveState === 'error' ? 'SAVE ERROR (TRY AGAIN)' : 'SAVE & CLOSE'}
          </button>
        </div>
      </div>
    </div>
  );
}

export function ExivPluginManager() {
  const [plugins, setPlugins] = useState<PluginManifest[]>([]);
  const [editingPlugins, setEditingPlugins] = useState<PluginManifest[]>([]);
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [applyError, setApplyError] = useState('');
  const [configTarget, setConfigTarget] = useState<PluginManifest | null>(null);
  
  // Category expanded state (Discord-style)
  const [expandedCategories, setExpandedCategories] = useState<Record<PluginCategory, boolean>>({
    'Agent': true,
    'Tool': true,
    'Memory': true,
    'System': false,
    'Other': true,
  });

  useEffect(() => {
    fetchPlugins();
  }, []);

  const fetchPlugins = async () => {
    setIsLoading(true);
    try {
      const data = await api.getPlugins();
      setPlugins(data);
      setEditingPlugins(JSON.parse(JSON.stringify(data)));
    } catch (err) {
      console.error('Failed to fetch plugins:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const togglePlugin = (id: string) => {
    setEditingPlugins(prev => prev.map(p => 
      p.id === id ? { ...p, is_active: !p.is_active } : p
    ));
  };

  const toggleCategory = (cat: PluginCategory) => {
    setExpandedCategories(prev => ({ ...prev, [cat]: !prev[cat] }));
  };

  const hasChanges = JSON.stringify(plugins) !== JSON.stringify(editingPlugins);

  const isPluginPending = (id: string) => {
    const original = plugins.find(p => p.id === id);
    const editing = editingPlugins.find(p => p.id === id);
    return JSON.stringify(original) !== JSON.stringify(editing);
  };

  const applyChanges = async () => {
    if (!apiKey) {
      setApplyError('API Key is required to apply changes.');
      return;
    }
    setIsSaving(true);
    setApplyError('');
    try {
      const changes = editingPlugins
        .filter(p => {
          const original = plugins.find(orig => orig.id === p.id);
          return original && original.is_active !== p.is_active;
        })
        .map(p => ({ id: p.id, is_active: p.is_active }));

      if (changes.length > 0) {
        await api.applyPluginSettings(changes, apiKey);
      }
      setPlugins(JSON.parse(JSON.stringify(editingPlugins)));
    } catch (err: any) {
      setApplyError(err?.message || 'Failed to apply plugin changes.');
      console.error('Failed to apply plugin changes:', err);
    } finally {
      setIsSaving(false);
    }
  };

  const allTags = Array.from(new Set(plugins.flatMap(p => p.tags)));
  
  const filteredPlugins = editingPlugins.filter(p => 
    selectedTags.length === 0 || selectedTags.some(tag => p.tags.includes(tag))
  );

  // Group by category
  const groupedPlugins: Record<PluginCategory, PluginManifest[]> = {
    'Agent': [],
    'Tool': [],
    'Memory': [],
    'System': [],
    'Other': [],
  };

  filteredPlugins.forEach(p => {
    // Backend should return category, but fallback to 'Other' if missing or unknown
    const cat = p.category || 'Other';
    if (groupedPlugins[cat]) {
      groupedPlugins[cat].push(p);
    } else {
      groupedPlugins['Other'].push(p);
    }
  });

  const getActionIcon = (iconName?: string) => {
    switch(iconName) {
      case 'Settings': return <Settings size={14} />;
      case 'Database': return <Database size={14} />;
      case 'MousePointer2': return <MousePointer2 size={14} />;
      case 'ExternalLink': return <ExternalLink size={14} />;
      case 'Terminal': return <Terminal size={14} />;
      default: return <Settings size={14} />;
    }
  };


  // Order of categories
  const categoryOrder: PluginCategory[] = ['Agent', 'Tool', 'Memory', 'System', 'Other'];

  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl overflow-hidden">
      {/* Header */}
      <div className="p-6 border-b border-edge-subtle flex items-center justify-between bg-glass">
        <div>
          <h2 className="text-xl font-black tracking-tight text-content-primary uppercase">System Plugins</h2>
          <p className="text-[10px] text-content-tertiary font-mono tracking-widest uppercase mt-1">
            EXIV-SYSTEM Kernel v{__APP_VERSION__} / Configuration Panel
          </p>
        </div>
        <div className="flex items-center gap-3">
           <div className="px-3 py-1 rounded-full bg-surface-secondary text-[10px] font-bold text-content-secondary">
             {plugins.filter(p => p.is_active).length} / {plugins.length} ACTIVE
           </div>
        </div>
      </div>

      <div className="flex-1 flex overflow-hidden">
        {/* Sidebar - Tags */}
        <div className="w-64 border-r border-edge-subtle bg-surface-base/30 p-6 flex flex-col gap-6 hidden md:flex">
          <div>
            <div className="flex items-center gap-2 text-content-tertiary mb-4">
              <Filter size={14} />
              <span className="text-[10px] font-black uppercase tracking-widest">Tag Filters</span>
            </div>
            <div className="flex flex-wrap gap-2">
              {allTags.map(tag => (
                <button
                  key={tag}
                  onClick={() => setSelectedTags(prev => 
                    prev.includes(tag) ? prev.filter(t => t !== tag) : [...prev, tag]
                  )}
                  className={`px-3 py-1.5 rounded-lg text-[10px] font-bold transition-all ${
                    selectedTags.includes(tag)
                      ? 'bg-brand text-white shadow-md'
                      : 'bg-surface-primary text-content-secondary border border-edge hover:border-brand/50'
                  }`}
                >
                  {tag}
                </button>
              ))}
            </div>
          </div>

          <div className="mt-auto">
             <div className="p-4 rounded-2xl bg-brand/5 border border-brand/10 text-brand">
               <div className="flex items-center gap-2 mb-2">
                 <Shield size={16} />
                 <span className="text-[10px] font-black uppercase tracking-widest">Security</span>
               </div>
               <p className="text-[9px] leading-relaxed opacity-80">
                 Only plugins signed with the official Exiv SDK (Magic Seal) are marked as VERIFIED.
               </p>
             </div>
          </div>
        </div>

        {/* Main Area - Plugin Cards (Grouped) */}
        <div className="flex-1 overflow-y-auto p-6 no-scrollbar space-y-6">
          {categoryOrder.map(category => {
            const categoryPlugins = groupedPlugins[category];
            if (categoryPlugins.length === 0) return null;

            const isExpanded = expandedCategories[category];

            return (
              <div key={category} className="space-y-3">
                {/* Category Header */}
                <button 
                  onClick={() => toggleCategory(category)}
                  className="flex items-center gap-2 text-content-tertiary hover:text-content-secondary transition-colors w-full text-left"
                >
                  {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <span className="text-[11px] font-black uppercase tracking-widest">{category}s</span>
                  <div className="h-px bg-surface-secondary flex-1 ml-2" />
                  <span className="text-[10px] font-mono text-content-muted">{categoryPlugins.length}</span>
                </button>

                {/* Plugin Grid */}
                {isExpanded && (
                  <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4 gap-4 pl-2 animate-in slide-in-from-top-2 duration-300">
                    {categoryPlugins.map(plugin => {
                      const isVerified = plugin.magic_seal === 0x56455253;
                      return (
                        <div 
                          key={plugin.id}
                          className={`group relative p-5 rounded-2xl border transition-all duration-300 ${
                            plugin.is_active
                              ? 'bg-surface-primary border-edge shadow-sm hover:shadow-md'
                              : 'bg-surface-base/50 border-edge-subtle opacity-60'
                          }`}
                        >
                          <div className="flex items-start justify-between mb-4">
                            <div className={`p-2.5 rounded-xl ${plugin.is_active ? 'bg-brand/10 text-brand' : 'bg-surface-secondary text-content-tertiary'}`}>
                              <ServiceTypeIcon type={plugin.service_type} size={20} />
                            </div>
                            <button
                              onClick={() => togglePlugin(plugin.id)}
                              className={`w-12 h-6 rounded-full relative transition-colors duration-300 ${
                                plugin.is_active ? 'bg-brand' : 'bg-content-muted'
                              }`}
                            >
                              <div className={`absolute top-1 w-4 h-4 bg-white rounded-full transition-all duration-300 ${
                                plugin.is_active ? 'left-7' : 'left-1'
                              }`} />
                            </button>
                          </div>

                          <div className="mb-4">
                            <div className="flex items-center gap-2">
                              <h3 className="font-bold text-content-primary text-sm">{plugin.name}</h3>
                              {isVerified ? (
                                <CheckCircle2 size={14} className="text-emerald-500" title={`Verified (SDK v${plugin.sdk_version})`} />
                              ) : (
                                <AlertTriangle size={14} className="text-amber-500" title="Unverified Plugin" />
                              )}
                            </div>
                            <p className="text-[11px] text-content-secondary mt-1 line-clamp-2 leading-relaxed h-8">
                              {plugin.description}
                            </p>
                          </div>

                          <div className="flex flex-wrap gap-1.5 mt-auto">
                            {plugin.tags.map(tag => (
                              <span key={tag} className="flex items-center gap-0.5 px-2 py-0.5 bg-surface-secondary rounded text-[9px] font-mono text-content-tertiary">
                                <Hash size={8} className="opacity-50" />
                                {tag.replace('#', '')}
                              </span>
                            ))}
                            {!isVerified && (
                              <span className="px-2 py-0.5 bg-amber-100 text-amber-600 rounded text-[9px] font-black uppercase tracking-tighter">
                                UNVERIFIED
                              </span>
                            )}
                          </div>

                          {/* Action Icon (Bottom Right) */}
                          {plugin.action_icon && (
                            <button
                              disabled={!plugin.is_active || isPluginPending(plugin.id)}
                              onClick={() => setConfigTarget(plugin)}
                              className={`absolute bottom-4 right-4 p-2 rounded-lg transition-all ${
                                plugin.is_active && !isPluginPending(plugin.id)
                                  ? 'bg-brand/10 text-brand hover:bg-brand hover:text-white shadow-sm'
                                  : 'bg-surface-secondary text-content-muted cursor-not-allowed opacity-50'
                              }`}
                              title={!plugin.is_active ? "Activate plugin to configure" : isPluginPending(plugin.id) ? "Apply changes to configure" : "Plugin Settings"}
                            >
                              {getActionIcon(plugin.action_icon)}
                            </button>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* Footer - Apply Bar */}
      {hasChanges && (
        <div className="p-4 bg-surface-primary border-t border-edge-subtle flex items-center gap-4 animate-in slide-in-from-bottom-full duration-500 z-50">
          <div className="flex items-center gap-2 text-amber-600 px-4 flex-shrink-0">
            <AlertTriangle size={16} />
            <span className="text-[10px] font-bold uppercase tracking-widest">Pending changes</span>
          </div>
          <div className="flex items-center gap-2 flex-1">
            <Lock size={12} className="text-content-muted flex-shrink-0" />
            <input
              type="password"
              value={apiKey}
              onChange={e => { setApiKey(e.target.value); setApplyError(''); }}
              onKeyDown={e => e.key === 'Enter' && apiKey && applyChanges()}
              placeholder="API Key"
              className="flex-1 min-w-0 px-3 py-1.5 rounded-lg border border-white/10 bg-surface-base text-xs font-mono text-content-primary placeholder:text-content-muted focus:outline-none focus:border-brand"
            />
          </div>
          {applyError && (
            <span className="text-[10px] text-red-400 font-medium flex-shrink-0">{applyError}</span>
          )}
          <button
            onClick={applyChanges}
            disabled={isSaving || !apiKey}
            className="flex items-center gap-2 px-6 py-2.5 bg-brand text-white rounded-xl text-xs font-bold shadow-lg shadow-brand/20 hover:scale-105 active:scale-95 transition-all disabled:opacity-50 flex-shrink-0"
          >
            {isSaving ? (
              <div className="w-4 h-4 border-2 border-white/20 border-t-white rounded-full animate-spin" />
            ) : <Save size={16} />}
            APPLY CONFIGURATION
          </button>
        </div>
      )}

      {/* Config Modal */}
      {configTarget && (
        <ConfigModal plugin={configTarget} onClose={() => setConfigTarget(null)} />
      )}
    </div>
  );
}