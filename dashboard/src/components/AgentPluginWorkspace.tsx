import React, { useState, useEffect } from 'react';
import { Puzzle, X, CheckCircle2, Save, Lock, Shield, Wrench } from 'lucide-react';
import { PluginManifest, AgentMetadata, InstalledConfig } from '../types';
import { api } from '../services/api';
import { AgentIcon, agentColor, isAiAgent } from '../lib/agentIdentity';
import { isLlmPlugin, ServiceTypeIcon } from '../lib/pluginUtils';
import { useApiKey } from '../contexts/ApiKeyContext';
import { Spinner } from '../lib/Spinner';

interface Props {
  agent: AgentMetadata;
  availablePlugins: PluginManifest[];
  onBack: () => void;
}

// System-level plugins that are always active for all agents (event dispatch level).
// Shown as read-only in the UI — cannot be added/removed per-agent.
const SYSTEM_ALWAYS_PLUGINS = ['core.moderator'];

const GRID_SIZE = 64;

export function AgentPluginWorkspace({ agent, availablePlugins, onBack }: Props) {
  const { apiKey } = useApiKey();
  const [configs, setConfigs] = useState<InstalledConfig[]>([]);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [isDraggingFromLibrary, setIsDraggingFromLibrary] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [saveError, setSaveError] = useState('');

  // Load from agent_plugins table
  useEffect(() => {
    if (!apiKey) { setIsLoading(false); return; }
    api.getAgentPlugins(agent.id, apiKey)
      .then(rows => {
        setConfigs(rows.map(r => ({ pluginId: r.plugin_id, x: r.pos_x, y: r.pos_y })));
      })
      .catch(e => {
        console.error('Failed to load agent plugins:', e);
        // Fallback: try legacy plugin_layout metadata
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
      setSaveError('API Key が設定されていません。右上の 🔒 から設定してください。');
      return;
    }
    setIsSaving(true);
    setSaveError('');
    try {
      await api.setAgentPlugins(
        agent.id,
        configs.map(c => ({ plugin_id: c.pluginId, pos_x: c.x, pos_y: c.y })),
        apiKey
      );
      onBack();
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save plugin configuration');
      console.error('Failed to save agent plugins:', err);
    } finally {
      setIsSaving(false);
    }
  };

  const ai = isAiAgent(agent);
  const libraryPlugins = availablePlugins.filter(p => {
    if (SYSTEM_ALWAYS_PLUGINS.includes(p.id)) return false;
    if (configs.find(c => c.pluginId === p.id)) return false;
    if (p.category === 'Memory') return ai ? true : !isLlmPlugin(p);
    if (p.category === 'Agent') return ai ? isLlmPlugin(p) : !isLlmPlugin(p);
    return false;
  });

  const systemPlugins = availablePlugins.filter(p => SYSTEM_ALWAYS_PLUGINS.includes(p.id));

  const handleDragStartFromLibrary = (id: string) => {
    setDraggingId(id);
    setIsDraggingFromLibrary(true);
  };

  const handleDragStartFromCore = (id: string) => {
    setDraggingId(id);
    setIsDraggingFromLibrary(false);
  };

  const handleDropToLibrary = (e: React.DragEvent) => {
    e.preventDefault();
    if (draggingId && !isDraggingFromLibrary) {
      setConfigs(prev => prev.filter(c => c.pluginId !== draggingId));
    }
    setDraggingId(null);
  };

  const handleDropToCore = (e: React.DragEvent) => {
    e.preventDefault();
    const rect = e.currentTarget.getBoundingClientRect();
    const x = Math.floor((e.clientX - rect.left) / GRID_SIZE);
    const y = Math.floor((e.clientY - rect.top) / GRID_SIZE);

    if (draggingId) {
      const isOccupied = configs.some(c => c.x === x && c.y === y && c.pluginId !== draggingId);
      if (isOccupied) return;

      if (isDraggingFromLibrary) {
        setConfigs(prev => [...prev, { pluginId: draggingId, x, y }]);
      } else {
        setConfigs(prev => prev.map(c =>
          c.pluginId === draggingId ? { ...c, x, y } : c
        ));
      }
    }
    setDraggingId(null);
  };

  const getPluginById = (id: string) => availablePlugins.find(p => p.id === id);

  const MANDATORY_TAGS = ['#CORE', '#MIND', '#MEMORY', '#LLM', '#TOOL', '#ADAPTER', '#HAL'];

  return (
    <div className="flex h-full bg-glass backdrop-blur-3xl overflow-hidden animate-in fade-in duration-500">
      {/* Sidebar: Plugin Library */}
      <div
        className="w-80 border-r border-edge-subtle bg-surface-base/50 flex flex-col"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDropToLibrary}
      >
        <div className="p-4 border-b border-edge-subtle bg-glass flex justify-between items-center">
          <div>
            <h3 className="text-[10px] font-black tracking-[0.2em] text-content-tertiary uppercase">Library</h3>
            <p className="text-[8px] text-content-muted mt-0.5 uppercase font-mono">Drag to Core Matrix</p>
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-3 no-scrollbar">
          {/* Assignable plugins */}
          {libraryPlugins.map(plugin => {
            const isVerified = MANDATORY_TAGS.some(tag => plugin.tags.includes(tag));
            return (
              <div
                key={plugin.id}
                draggable
                onDragStart={() => handleDragStartFromLibrary(plugin.id)}
                className="bg-surface-primary border border-edge-subtle p-3 rounded-2xl flex flex-col cursor-grab active:cursor-grabbing hover:shadow-md transition-all group"
                style={{ ['--accent' as string]: agentColor(agent) }}
                onMouseEnter={e => (e.currentTarget.style.borderColor = `${agentColor(agent)}4D`)}
                onMouseLeave={e => (e.currentTarget.style.borderColor = '')}
              >
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-xl shrink-0" style={{ backgroundColor: `${agentColor(agent)}0D`, color: agentColor(agent) }}>
                    <ServiceTypeIcon type={plugin.service_type} size={24} />
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5">
                      <h4 className="font-bold text-content-primary text-[11px] truncate">{plugin.name}</h4>
                      {isVerified && <CheckCircle2 size={10} className="text-emerald-500" />}
                    </div>
                    <p className="text-[8px] text-content-tertiary line-clamp-1">{plugin.description}</p>
                  </div>
                </div>
              </div>
            );
          })}

          {/* System capabilities — always active, read-only */}
          {systemPlugins.length > 0 && (
            <div className="pt-2">
              <p className="text-[8px] font-black tracking-[0.2em] text-content-muted uppercase mb-2 flex items-center gap-1">
                <Shield size={8} /> System Capabilities
              </p>
              {systemPlugins.map(plugin => (
                <div
                  key={plugin.id}
                  className="bg-surface-secondary border border-edge-subtle/50 p-3 rounded-2xl flex items-center gap-3 opacity-70"
                  title="Always active — cannot be removed"
                >
                  <div className="p-2 rounded-xl shrink-0 text-content-muted">
                    <Shield size={16} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <h4 className="font-bold text-content-secondary text-[11px] truncate">{plugin.name}</h4>
                      <Lock size={8} className="text-content-muted shrink-0" />
                    </div>
                    <p className="text-[8px] text-content-muted">Always active for all agents</p>
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* skill_manager info chip (always available tool) */}
          <div className="pt-1">
            <p className="text-[8px] font-black tracking-[0.2em] text-content-muted uppercase mb-2 flex items-center gap-1">
              <Wrench size={8} /> Built-in Tools
            </p>
            <div
              className="bg-surface-secondary border border-edge-subtle/50 p-3 rounded-2xl flex items-center gap-3 opacity-70"
              title="Available when core.skill_manager is assigned"
            >
              <div className="p-2 rounded-xl shrink-0 text-content-muted">
                <Puzzle size={16} />
              </div>
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <h4 className="font-bold text-content-secondary text-[11px]">Skill Manager</h4>
                </div>
                <p className="text-[8px] text-content-muted">Active when core.skill_manager is assigned</p>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Main Workspace: Grid Board */}
      <div className="flex-1 flex flex-col relative overflow-hidden">
        <div className="p-4 border-b border-edge-subtle bg-glass flex items-center justify-between z-10">
          <div className="flex items-center gap-3">
            <div className="p-1.5 text-white rounded-md shadow-lg" style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}>
              <AgentIcon agent={agent} size={14} />
            </div>
            <h2 className="text-sm font-black text-content-primary tracking-tight uppercase">{agent.name} Core Matrix</h2>
          </div>
          <button onClick={onBack} className="p-1 text-content-muted hover:text-content-primary transition-colors"><X size={20} /></button>
        </div>

        <div
          className="flex-1 relative bg-surface-base/20 overflow-auto"
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDropToCore}
          style={{
            backgroundImage: `radial-gradient(circle, var(--canvas-grid) 1px, transparent 1px)`,
            backgroundSize: `${GRID_SIZE}px ${GRID_SIZE}px`
          }}
        >
          <div className="absolute inset-0 pointer-events-none opacity-[0.1]"
            style={{
              backgroundImage: `linear-gradient(to right, ${agentColor(agent)} 1px, transparent 1px), linear-gradient(to bottom, ${agentColor(agent)} 1px, transparent 1px)`,
              backgroundSize: `${GRID_SIZE}px ${GRID_SIZE}px`
            }}
          />

          {isLoading ? (
            <div className="absolute inset-0 flex items-center justify-center text-content-muted text-[10px] font-mono tracking-widest uppercase animate-pulse">
              Loading configuration...
            </div>
          ) : (
            configs.map(config => {
              const plugin = getPluginById(config.pluginId);
              if (!plugin) return null;
              return (
                <div
                  key={config.pluginId}
                  draggable
                  onDragStart={() => handleDragStartFromCore(config.pluginId)}
                  className="absolute w-12 h-12 bg-surface-primary border-2 rounded-xl shadow-lg flex items-center justify-center cursor-grab active:cursor-grabbing hover:scale-110 transition-transform animate-in zoom-in-90"
                  style={{
                    left: config.x * GRID_SIZE + (GRID_SIZE - 48) / 2,
                    top: config.y * GRID_SIZE + (GRID_SIZE - 48) / 2,
                    borderColor: agentColor(agent),
                  }}
                  title={plugin.name}
                >
                  <div className="scale-75" style={{ color: agentColor(agent) }}>
                    <ServiceTypeIcon type={plugin.service_type} size={24} />
                  </div>
                  <div className="absolute -inset-1 border rounded-xl animate-pulse" style={{ borderColor: `${agentColor(agent)}33` }} />
                </div>
              );
            })
          )}
        </div>

        <div className="p-4 bg-glass-subtle border-t border-edge-subtle flex items-center justify-between px-8 text-[9px] font-mono text-content-tertiary">
           <div className="flex flex-col gap-1">
             <div className="flex gap-4">
               <span>COORDINATE_SYSTEM: ACTIVE</span>
               <span>MATRIX_STABILITY: 100%</span>
             </div>
             {saveError && (
               <span className="text-red-400 text-[10px]">{saveError}</span>
             )}
           </div>
           <button
             onClick={handleSave}
             disabled={isSaving || isLoading}
             className="flex items-center gap-2 px-8 py-2 text-white rounded-xl font-bold tracking-widest shadow-lg transition-all active:scale-95 disabled:opacity-50"
             style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}
           >
             {isSaving ? (
               <Spinner size={3} />
             ) : <Save size={14} />}
             Save and exit
           </button>
        </div>
      </div>
    </div>
  );
}
