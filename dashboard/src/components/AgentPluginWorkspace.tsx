import React, { useState, useEffect } from 'react';
import { Puzzle, ArrowLeft, Plus, X, Save, Lock, Shield, Wrench, Activity } from 'lucide-react';
import { PluginManifest, AgentMetadata, InstalledConfig } from '../types';
import { api } from '../services/api';
import { AgentIcon, agentColor, isAiAgent } from '../lib/agentIdentity';
import { isLlmPlugin, ServiceTypeIcon } from '../lib/pluginUtils';
import { useApiKey } from '../contexts/ApiKeyContext';

interface Props {
  agent: AgentMetadata;
  availablePlugins: PluginManifest[];
  onBack: () => void;
}

const SYSTEM_ALWAYS_PLUGINS = ['core.moderator'];

export function AgentPluginWorkspace({ agent, availablePlugins, onBack }: Props) {
  const { apiKey } = useApiKey();
  const [configs, setConfigs] = useState<InstalledConfig[]>([]);
  const [isSaving, setIsSaving] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [saveError, setSaveError] = useState('');

  useEffect(() => {
    if (!apiKey) { setIsLoading(false); return; }
    api.getAgentPlugins(agent.id, apiKey)
      .then(rows => {
        setConfigs(rows.map(r => ({ pluginId: r.plugin_id, x: r.pos_x, y: r.pos_y })));
      })
      .catch(e => {
        console.error('Failed to load agent plugins:', e);
        if (agent.metadata.plugin_layout) {
          try {
            const layout = JSON.parse(agent.metadata.plugin_layout);
            setConfigs(layout);
          } catch {}
        }
      })
      .finally(() => setIsLoading(false));
  }, [agent.id, apiKey]);

  const handleSave = async () => {
    if (!apiKey) {
      setSaveError('API Key is not set.');
      return;
    }
    setIsSaving(true);
    setSaveError('');
    try {
      await api.setAgentPlugins(
        agent.id,
        configs.map(c => ({ plugin_id: c.pluginId, pos_x: 0, pos_y: 0 })),
        apiKey
      );
      onBack();
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save plugin configuration');
    } finally {
      setIsSaving(false);
    }
  };

  const ai = isAiAgent(agent);
  const assignedPluginIds = new Set(configs.map(c => c.pluginId));
  const libraryPlugins = availablePlugins.filter(p => {
    if (SYSTEM_ALWAYS_PLUGINS.includes(p.id)) return false;
    if (assignedPluginIds.has(p.id)) return false;
    if (p.category === 'Memory') return ai ? true : !isLlmPlugin(p);
    if (p.category === 'Agent') return ai ? isLlmPlugin(p) : !isLlmPlugin(p);
    return false;
  });
  const systemPlugins = availablePlugins.filter(p => SYSTEM_ALWAYS_PLUGINS.includes(p.id));

  const addPlugin = (pluginId: string) => {
    setConfigs(prev => [...prev, { pluginId, x: 0, y: 0 }]);
  };

  const removePlugin = (pluginId: string) => {
    setConfigs(prev => prev.filter(c => c.pluginId !== pluginId));
  };

  const getPluginById = (id: string) => availablePlugins.find(p => p.id === id);

  return (
    <div className="flex flex-col h-full overflow-hidden animate-in fade-in duration-500">
      {/* Header — MemoryCore pattern */}
      <header className="p-6 flex items-center justify-between border-b border-edge">
        <div className="flex items-center gap-4">
          <button
            onClick={onBack}
            className="p-2.5 rounded-full bg-glass-subtle backdrop-blur-sm border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={18} />
          </button>
          <div className="w-10 h-10 rounded-md flex items-center justify-center shadow-sm text-white" style={{ backgroundColor: agentColor(agent) }}>
            <AgentIcon agent={agent} size={20} />
          </div>
          <div>
            <h1 className="text-xl font-black tracking-tighter text-content-primary uppercase">{agent.name} · Plugins</h1>
            <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em]">Plugin Configuration</p>
          </div>
        </div>
        <div className="bg-glass-subtle backdrop-blur-sm px-4 py-2 rounded-md flex items-center gap-3 shadow-sm border border-edge">
          <span className="text-[9px] uppercase font-bold text-content-tertiary tracking-widest">{configs.length} assigned</span>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6 md:p-8 space-y-6 no-scrollbar">
        {isLoading ? (
          <div className="py-12 text-center text-content-muted font-mono text-xs animate-pulse">Loading...</div>
        ) : (
          <>
            {/* Assigned Plugins */}
            <section>
              <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
                <Puzzle className="text-brand" size={16} />
                <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Assigned Plugins</h2>
              </div>
              {configs.length === 0 ? (
                <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                  No plugins assigned. Add from the list below.
                </div>
              ) : (
                <div className="space-y-2">
                  {configs.map(config => {
                    const plugin = getPluginById(config.pluginId);
                    if (!plugin) return null;
                    return (
                      <div key={config.pluginId} className="bg-glass-strong backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-brand transition-all flex items-center gap-3 group">
                        <div className="p-1.5 rounded-md" style={{ backgroundColor: `${agentColor(agent)}15`, color: agentColor(agent) }}>
                          <ServiceTypeIcon type={plugin.service_type} size={16} />
                        </div>
                        <div className="flex-1 min-w-0">
                          <span className="text-xs font-bold text-content-primary">{plugin.name}</span>
                          <span className="text-[10px] text-content-muted ml-2 font-mono">{plugin.id}</span>
                        </div>
                        <span className="text-[9px] font-bold text-content-tertiary uppercase tracking-wider px-2 py-0.5 bg-surface-secondary rounded">
                          {plugin.service_type}
                        </span>
                        <button
                          onClick={() => removePlugin(config.pluginId)}
                          className="p-1.5 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                          title="Remove"
                        >
                          <X size={14} />
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>

            {/* Available Plugins */}
            {libraryPlugins.length > 0 && (
              <section>
                <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
                  <Plus className="text-brand" size={16} />
                  <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Available</h2>
                </div>
                <div className="space-y-2">
                  {libraryPlugins.map(plugin => (
                    <div key={plugin.id} className="bg-glass backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-brand/50 transition-all flex items-center gap-3 group">
                      <div className="p-1.5 rounded-md text-content-muted">
                        <ServiceTypeIcon type={plugin.service_type} size={16} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="text-xs font-medium text-content-secondary">{plugin.name}</span>
                        <span className="text-[10px] text-content-muted ml-2 font-mono">{plugin.id}</span>
                      </div>
                      <span className="text-[9px] font-bold text-content-muted uppercase tracking-wider px-2 py-0.5 bg-surface-secondary/50 rounded">
                        {plugin.service_type}
                      </span>
                      <button
                        onClick={() => addPlugin(plugin.id)}
                        className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-brand hover:bg-brand/10 transition-all opacity-0 group-hover:opacity-100"
                      >
                        <Plus size={10} /> Add
                      </button>
                    </div>
                  ))}
                </div>
              </section>
            )}

            {/* System Plugins */}
            {systemPlugins.length > 0 && (
              <section>
                <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
                  <Shield className="text-content-muted" size={16} />
                  <h2 className="font-bold text-xs text-content-muted uppercase tracking-widest">System (always active)</h2>
                </div>
                <div className="space-y-2">
                  {systemPlugins.map(plugin => (
                    <div key={plugin.id} className="bg-glass backdrop-blur-sm px-4 py-3 rounded-lg border border-edge/50 flex items-center gap-3 opacity-60">
                      <div className="p-1.5 rounded-md text-content-muted">
                        <Shield size={16} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="text-xs text-content-secondary">{plugin.name}</span>
                      </div>
                      <Lock size={10} className="text-content-muted" />
                    </div>
                  ))}
                </div>
              </section>
            )}
          </>
        )}
      </div>

      {/* Footer */}
      <div className="p-4 border-t border-edge flex items-center justify-between">
        {saveError && <span className="text-[10px] text-red-400">{saveError}</span>}
        <div className="flex-1" />
        <div className="flex gap-2">
          <button
            onClick={onBack}
            className="px-4 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={isSaving || isLoading}
            className="flex items-center gap-1.5 px-6 py-2 rounded-lg bg-brand text-white text-xs font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50"
          >
            {isSaving ? <Activity size={14} className="animate-spin" /> : <Save size={14} />}
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
