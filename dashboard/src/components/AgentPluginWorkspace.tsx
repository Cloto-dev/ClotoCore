import React, { useState, useEffect } from 'react';
import { Puzzle, X, Brain, Database, Zap, Globe, CheckCircle2, Save } from 'lucide-react';
import { PluginManifest, AgentMetadata } from '../types';
import { api } from '../services/api';
import { AgentIcon, agentColor, isAiAgent } from '../lib/agentIdentity';

interface Props {
  agent: AgentMetadata;
  availablePlugins: PluginManifest[];
  onBack: () => void;
}

interface InstalledConfig {
  pluginId: string;
  x: number;
  y: number;
}

const GRID_SIZE = 64; // マス目のサイズ

export function AgentPluginWorkspace({ agent, availablePlugins, onBack }: Props) {
  const [configs, setConfigs] = useState<InstalledConfig[]>([]);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [isDraggingFromLibrary, setIsDraggingFromLibrary] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    if (agent.metadata.plugin_layout) {
      try {
        const layout = JSON.parse(agent.metadata.plugin_layout);
        setConfigs(layout);
      } catch (e) {
        console.error("Failed to parse plugin layout:", e);
      }
    }
  }, [agent]);

  const handleSave = async () => {
    setIsSaving(true);
    try {
      // Find the first Reasoning engine and first Memory engine in the matrix
      const reasoningPlugin = configs.find(c => {
        const p = getPluginById(c.pluginId);
        return p?.service_type === 'Reasoning';
      });
      const memoryPlugin = configs.find(c => {
        const p = getPluginById(c.pluginId);
        return p?.service_type === 'Memory';
      });

      const metadata = { 
        ...agent.metadata, 
        plugin_layout: JSON.stringify(configs),
        preferred_memory: memoryPlugin?.pluginId || agent.metadata.preferred_memory
      };

      await api.updateAgent(agent.id, { 
        default_engine_id: reasoningPlugin?.pluginId,
        metadata 
      });
      onBack(); // Close the workspace after saving
    } catch (err) {
      console.error("Failed to save neural matrix:", err);
    } finally {
      setIsSaving(false);
    }
  };

  const isLlmPlugin = (p: PluginManifest) => p.tags.includes('#MIND') || p.tags.includes('#LLM');
  const ai = isAiAgent(agent);
  const libraryPlugins = availablePlugins.filter(p => {
    if (configs.find(c => c.pluginId === p.id)) return false;
    if (p.category === 'Memory') return ai ? true : !isLlmPlugin(p);
    if (p.category === 'Agent') return ai ? isLlmPlugin(p) : !isLlmPlugin(p);
    return false;
  });

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
      // 衝突チェック: 同じ座標に他のプラグインがあるか
      const isOccupied = configs.some(c => c.x === x && c.y === y && c.pluginId !== draggingId);
      
      if (isOccupied) {
        console.warn("Grid cell occupied at:", x, y);
        return; // 重なりを防止
      }

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

  const getIcon = (type: string) => {
    switch(type) {
      case 'Reasoning': return <Brain size={24} />;
      case 'Memory': return <Database size={24} />;
      case 'Skill': return <Zap size={24} />;
      case 'Communication': return <Globe size={24} />;
      default: return <Puzzle size={24} />;
    }
  };

  const MANDATORY_TAGS = ['#CORE', '#MIND', '#MEMORY', '#LLM', '#TOOL', '#ADAPTER', '#HAL'];

  return (
    <div className="flex h-full bg-white/20 backdrop-blur-3xl overflow-hidden animate-in fade-in duration-500">
      {/* Sidebar: Plugin Library */}
      <div 
        className="w-80 border-r border-slate-100 bg-slate-50/50 flex flex-col"
        onDragOver={(e) => e.preventDefault()}
        onDrop={handleDropToLibrary}
      >
        <div className="p-4 border-b border-slate-100 bg-white/40 flex justify-between items-center">
          <div>
            <h3 className="text-[10px] font-black tracking-[0.2em] text-slate-400 uppercase">Library</h3>
            <p className="text-[8px] text-slate-300 mt-0.5 uppercase font-mono">Drag to Core Matrix</p>
          </div>
        </div>
        
        <div className="flex-1 overflow-y-auto p-4 space-y-3 no-scrollbar">
          {libraryPlugins.map(plugin => {
            const isVerified = MANDATORY_TAGS.some(tag => plugin.tags.includes(tag));
            return (
              <div
                key={plugin.id}
                draggable
                onDragStart={() => handleDragStartFromLibrary(plugin.id)}
                className="bg-white border border-slate-100 p-3 rounded-2xl flex flex-col cursor-grab active:cursor-grabbing hover:shadow-md transition-all group"
                style={{ ['--accent' as string]: agentColor(agent) }}
                onMouseEnter={e => (e.currentTarget.style.borderColor = `${agentColor(agent)}4D`)}
                onMouseLeave={e => (e.currentTarget.style.borderColor = '')}
              >
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-xl shrink-0" style={{ backgroundColor: `${agentColor(agent)}0D`, color: agentColor(agent) }}>
                    {getIcon(plugin.service_type)}
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5">
                      <h4 className="font-bold text-slate-800 text-[11px] truncate">{plugin.name}</h4>
                      {isVerified && <CheckCircle2 size={10} className="text-emerald-500" />}
                    </div>
                    <p className="text-[8px] text-slate-400 line-clamp-1">{plugin.description}</p>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Main Workspace: Grid Board */}
      <div className="flex-1 flex flex-col relative overflow-hidden">
        <div className="p-4 border-b border-slate-100 bg-white/40 flex items-center justify-between z-10">
          <div className="flex items-center gap-3">
            <div className="p-1.5 text-white rounded-md shadow-lg" style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}>
              <AgentIcon agent={agent} size={14} />
            </div>
            <h2 className="text-sm font-black text-slate-800 tracking-tight uppercase">{agent.name} Core Matrix</h2>
          </div>
          <button onClick={onBack} className="p-1 text-slate-300 hover:text-slate-800 transition-colors"><X size={20} /></button>
        </div>

        <div 
          className="flex-1 relative bg-slate-50/20 overflow-auto"
          onDragOver={(e) => e.preventDefault()}
          onDrop={handleDropToCore}
          style={{
            backgroundImage: `radial-gradient(circle, #e2e8f0 1px, transparent 1px)`,
            backgroundSize: `${GRID_SIZE}px ${GRID_SIZE}px`
          }}
        >
          {/* Snap Grid Simulation */}
          <div className="absolute inset-0 pointer-events-none opacity-[0.1]"
            style={{
              backgroundImage: `linear-gradient(to right, ${agentColor(agent)} 1px, transparent 1px), linear-gradient(to bottom, ${agentColor(agent)} 1px, transparent 1px)`,
              backgroundSize: `${GRID_SIZE}px ${GRID_SIZE}px`
            }}
          />

          {/* Installed Chips */}
          {configs.map(config => {
            const plugin = getPluginById(config.pluginId);
            if (!plugin) return null;
            return (
              <div
                key={config.pluginId}
                draggable
                onDragStart={() => handleDragStartFromCore(config.pluginId)}
                className="absolute w-12 h-12 bg-white border-2 rounded-xl shadow-lg flex items-center justify-center cursor-grab active:cursor-grabbing hover:scale-110 transition-transform animate-in zoom-in-90"
                style={{
                  left: config.x * GRID_SIZE + (GRID_SIZE - 48) / 2,
                  top: config.y * GRID_SIZE + (GRID_SIZE - 48) / 2,
                  borderColor: agentColor(agent),
                }}
                title={plugin.name}
              >
                <div className="scale-75" style={{ color: agentColor(agent) }}>
                  {getIcon(plugin.service_type)}
                </div>
                <div className="absolute -inset-1 border rounded-xl animate-pulse" style={{ borderColor: `${agentColor(agent)}33` }} />
              </div>
            );
          })}
        </div>

        <div className="p-4 bg-white/80 border-t border-slate-100 flex items-center justify-between px-8 text-[9px] font-mono text-slate-400">
           <div className="flex gap-4">
             <span>COORDINATE_SYSTEM: ACTIVE</span>
             <span>MATRIX_STABILITY: 100%</span>
           </div>
           <button 
             onClick={handleSave}
             disabled={isSaving}
             className="flex items-center gap-2 px-8 py-2 text-white rounded-xl font-bold tracking-widest shadow-lg transition-all active:scale-95 disabled:opacity-50"
             style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}
           >
             {isSaving ? (
               <div className="w-3 h-3 border-2 border-white/20 border-t-white rounded-full animate-spin" />
             ) : <Save size={14} />}
             Save and exit
           </button>
        </div>
      </div>
    </div>
  );
}
