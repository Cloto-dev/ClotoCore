import React, { useState, useEffect } from 'react';
import { Puzzle, Shield, CheckCircle2, AlertTriangle, Save, Filter, Brain, Database, Zap, Globe, Settings, MousePointer2, ExternalLink, Terminal, ChevronDown, ChevronRight, Hash, Box, FolderOpen } from 'lucide-react';
import { PluginManifest, PluginCategory } from '../types';

import { api } from '../services/api';
import { isTauri, openFileDialog } from '../lib/tauri';

function ConfigModal({ plugin, onClose }: { plugin: PluginManifest, onClose: () => void }) {
  const [config, setConfig] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  // H-19: Use React state instead of direct DOM manipulation for save button
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'error'>('idle');

  useEffect(() => {
    api.getPluginConfig(plugin.id)
      .then(setConfig)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [plugin.id]);

  const save = async (key: string, value: string) => {
    const newConfig = { ...config, [key]: value };
    await api.updatePluginConfig(plugin.id, { key, value });
    setConfig(newConfig);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-4" onClick={onClose}>
      <div className="bg-white rounded-2xl w-full max-w-lg p-6 shadow-2xl animate-in zoom-in-95 duration-200" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-bold mb-4 flex items-center gap-2 text-slate-800">
          <Settings size={18} className="text-[#2e4de6]" />
          Configure {plugin.name}
        </h3>
        
        {loading ? (
          <div className="p-8 text-center text-slate-400 font-mono text-xs">Loading config...</div>
        ) : (
          <div className="space-y-4">
            {plugin.required_config_keys.length > 0 ? plugin.required_config_keys.map(key => {
              const isPathKey = /path|script|file|dir/i.test(key);
              const isSecretKey = key.includes('key') || key.includes('password');
              return (
                <div key={key}>
                  <label className="block text-xs font-bold text-slate-500 mb-1 uppercase tracking-wider">{key}</label>
                  <div className="flex gap-2">
                    <input
                      type={isSecretKey ? 'password' : 'text'}
                      value={config[key] || ''}
                      onChange={e => setConfig(prev => ({ ...prev, [key]: e.target.value }))}
                      onBlur={e => save(key, e.target.value)}
                      className="flex-1 px-3 py-2 rounded-lg border border-slate-200 text-sm font-mono focus:outline-none focus:border-[#2e4de6] focus:ring-1 focus:ring-[#2e4de6]"
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
                        className="px-3 py-2 rounded-lg border border-slate-200 bg-slate-50 hover:bg-[#2e4de6]/10 hover:border-[#2e4de6]/50 text-slate-500 hover:text-[#2e4de6] transition-colors"
                        title="Browse files"
                      >
                        <FolderOpen size={16} />
                      </button>
                    )}
                  </div>
                </div>
              );
            }) : (
              <div className="p-4 bg-slate-50 text-slate-400 text-xs rounded-lg text-center font-mono border border-slate-100 border-dashed">
                No configuration required for this plugin.
              </div>
            )}
          </div>
        )}

        <div className="mt-6 flex justify-end gap-3">
          <button onClick={onClose} className="px-6 py-2 bg-slate-100 text-slate-600 rounded-lg text-xs font-bold hover:bg-slate-200 transition-colors tracking-wide">
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
                    api.updatePluginConfig(plugin.id, { key, value })
                  )
                );
                onClose();
              } catch (err) {
                console.error("Failed to save config:", err);
                setSaveState('error');
              }
            }}
            disabled={saveState === 'saving'}
            className="px-6 py-2 bg-[#2e4de6] text-white rounded-lg text-xs font-bold hover:bg-[#1e3bb3] transition-all shadow-md shadow-[#2e4de6]/20 tracking-wide disabled:opacity-50"
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
    setIsSaving(true);
    try {
      const changes = editingPlugins
        .filter(p => {
          const original = plugins.find(orig => orig.id === p.id);
          return original && original.is_active !== p.is_active;
        })
        .map(p => ({ id: p.id, is_active: p.is_active }));

      if (changes.length > 0) {
        await api.applyPluginSettings(changes);
      }
      setPlugins(JSON.parse(JSON.stringify(editingPlugins)));
    } catch (err) {
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

  const getIcon = (type: string) => {
    switch(type) {
      case 'Reasoning': return <Brain size={20} />;
      case 'Memory': return <Database size={20} />;
      case 'Skill': return <Zap size={20} />;
      case 'Communication': return <Globe size={20} />;
      case 'HAL': return <MousePointer2 size={20} />;
      default: return <Puzzle size={20} />;
    }
  };

  // Order of categories
  const categoryOrder: PluginCategory[] = ['Agent', 'Tool', 'Memory', 'System', 'Other'];

  return (
    <div className="flex flex-col h-full bg-white/40 backdrop-blur-3xl overflow-hidden">
      {/* Header */}
      <div className="p-6 border-b border-slate-100 flex items-center justify-between bg-white/40">
        <div>
          <h2 className="text-xl font-black tracking-tight text-slate-800 uppercase">System Plugins</h2>
          <p className="text-[10px] text-slate-400 font-mono tracking-widest uppercase mt-1">
            EXIV-SYSTEM Kernel v{__APP_VERSION__} / Configuration Panel
          </p>
        </div>
        <div className="flex items-center gap-3">
           <div className="px-3 py-1 rounded-full bg-slate-100 text-[10px] font-bold text-slate-500">
             {plugins.filter(p => p.is_active).length} / {plugins.length} ACTIVE
           </div>
        </div>
      </div>

      <div className="flex-1 flex overflow-hidden">
        {/* Sidebar - Tags */}
        <div className="w-64 border-r border-slate-100 bg-slate-50/30 p-6 flex flex-col gap-6 hidden md:flex">
          <div>
            <div className="flex items-center gap-2 text-slate-400 mb-4">
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
                      ? 'bg-[#2e4de6] text-white shadow-md'
                      : 'bg-white text-slate-500 border border-slate-200 hover:border-[#2e4de6]/50'
                  }`}
                >
                  {tag}
                </button>
              ))}
            </div>
          </div>

          <div className="mt-auto">
             <div className="p-4 rounded-2xl bg-[#2e4de6]/5 border border-[#2e4de6]/10 text-[#2e4de6]">
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
                  className="flex items-center gap-2 text-slate-400 hover:text-slate-600 transition-colors w-full text-left"
                >
                  {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <span className="text-[11px] font-black uppercase tracking-widest">{category}s</span>
                  <div className="h-px bg-slate-100 flex-1 ml-2" />
                  <span className="text-[10px] font-mono text-slate-300">{categoryPlugins.length}</span>
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
                              ? 'bg-white border-slate-200 shadow-sm hover:shadow-md' 
                              : 'bg-slate-50/50 border-slate-100 opacity-60'
                          }`}
                        >
                          <div className="flex items-start justify-between mb-4">
                            <div className={`p-2.5 rounded-xl ${plugin.is_active ? 'bg-[#2e4de6]/10 text-[#2e4de6]' : 'bg-slate-200 text-slate-400'}`}>
                              {getIcon(plugin.service_type)}
                            </div>
                            <button
                              onClick={() => togglePlugin(plugin.id)}
                              className={`w-12 h-6 rounded-full relative transition-colors duration-300 ${
                                plugin.is_active ? 'bg-[#2e4de6]' : 'bg-slate-300'
                              }`}
                            >
                              <div className={`absolute top-1 w-4 h-4 bg-white rounded-full transition-all duration-300 ${
                                plugin.is_active ? 'left-7' : 'left-1'
                              }`} />
                            </button>
                          </div>

                          <div className="mb-4">
                            <div className="flex items-center gap-2">
                              <h3 className="font-bold text-slate-800 text-sm">{plugin.name}</h3>
                              {isVerified ? (
                                <CheckCircle2 size={14} className="text-emerald-500" title={`Verified (SDK v${plugin.sdk_version})`} />
                              ) : (
                                <AlertTriangle size={14} className="text-amber-500" title="Unverified Plugin" />
                              )}
                            </div>
                            <p className="text-[11px] text-slate-500 mt-1 line-clamp-2 leading-relaxed h-8">
                              {plugin.description}
                            </p>
                          </div>

                          <div className="flex flex-wrap gap-1.5 mt-auto">
                            {plugin.tags.map(tag => (
                              <span key={tag} className="flex items-center gap-0.5 px-2 py-0.5 bg-slate-100 rounded text-[9px] font-mono text-slate-400">
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
                                  ? 'bg-[#2e4de6]/10 text-[#2e4de6] hover:bg-[#2e4de6] hover:text-white shadow-sm'
                                  : 'bg-slate-100 text-slate-300 cursor-not-allowed opacity-50'
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
        <div className="p-4 bg-white border-t border-slate-100 flex items-center justify-between animate-in slide-in-from-bottom-full duration-500 z-50">
          <div className="flex items-center gap-2 text-amber-600 px-4">
            <AlertTriangle size={16} />
            <span className="text-[10px] font-bold uppercase tracking-widest">Pending changes exist</span>
          </div>
          <button
            onClick={applyChanges}
            disabled={isSaving}
            className="flex items-center gap-2 px-6 py-2.5 bg-[#2e4de6] text-white rounded-xl text-xs font-bold shadow-lg shadow-[#2e4de6]/20 hover:scale-105 active:scale-95 transition-all disabled:opacity-50"
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