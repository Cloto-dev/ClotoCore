import { useState } from 'react';
import { ChevronRight, Puzzle, Activity, Zap, Plus, Lock, Trash2 } from 'lucide-react';
import { AgentMetadata, PluginManifest } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor, AgentTypeIcon, agentTypeColor, isAiAgent, statusBadgeClass } from '../lib/agentIdentity';
import { isLlmPlugin } from '../lib/pluginUtils';
import { useAgentCreation } from '../hooks/useAgentCreation';
import { PowerToggleModal } from './PowerToggleModal';
import { AgentConsole } from './AgentConsole';
import { ContainerDashboard } from './ContainerDashboard';
import { AgentPowerButton } from './AgentPowerButton';

import { api, EVENTS_URL } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

export interface AgentTerminalProps {
  agents: AgentMetadata[];
  plugins: PluginManifest[];
  selectedAgent: AgentMetadata | null;
  onSelectAgent: (agent: AgentMetadata | null) => void;
  onRefresh: () => void;
}

export function AgentTerminal({
  agents,
  plugins,
  selectedAgent,
  onSelectAgent,
  onRefresh,
}: AgentTerminalProps) {
  const { apiKey } = useApiKey();
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  // Power toggle modal
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<AgentMetadata | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const DEFAULT_AGENT_ID = 'agent.exiv_default';

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    setDeleteError(null);
    try {
      await api.deleteAgent(deleteTarget.id, apiKey);
      setDeleteTarget(null);
      onRefresh();
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setIsDeleting(false);
    }
  };

  // Creation form
  const { form: newAgent, updateField, handleTypeChange, handleCreate, isCreating, createError } = useAgentCreation(onRefresh);

  // Listen for AgentPowerChanged events to auto-refresh
  useEventStream(EVENTS_URL, (event) => {
    if (event.type === 'AgentPowerChanged') {
      onRefresh();
    }
  });

  const handlePowerToggle = async (agent: AgentMetadata) => {
    if (agent.metadata?.has_power_password === 'true') {
      setPowerTarget(agent);
    } else {
      try {
        await api.toggleAgentPower(agent.id, !agent.enabled, apiKey);
        onRefresh();
      } catch (err) {
        console.error('Failed to toggle power:', err);
      }
    }
  };

  if (configuringAgent) {
    return (
      <AgentPluginWorkspace
        agent={configuringAgent}
        availablePlugins={plugins.filter(p => p.is_active)}
        onBack={() => { setConfiguringAgent(null); onRefresh(); }}
      />
    );
  }

  if (selectedAgent) {
    if (isAiAgent(selectedAgent)) {
      return <AgentConsole agent={selectedAgent} onBack={() => onSelectAgent(null)} />;
    }
    return (
      <ContainerDashboard
        agent={selectedAgent}
        plugins={plugins}
        onBack={() => onSelectAgent(null)}
        onConfigure={() => setConfiguringAgent(selectedAgent)}
        onPowerToggle={handlePowerToggle}
      />
    );
  }

  const allEngines = plugins.filter(p => p.service_type === 'Reasoning' && p.is_active && p.category === 'Agent');
  const filteredEngines = allEngines.filter(p => newAgent.type === 'ai' ? isLlmPlugin(p) : !isLlmPlugin(p));
  const allMemories = plugins.filter(p => (p.service_type === 'Memory' || p.category === 'Memory') && p.is_active);
  const memories = allMemories.filter(p => newAgent.type === 'ai' ? true : !isLlmPlugin(p));

  return (
    <div className="relative flex h-full bg-glass-subtle backdrop-blur-sm overflow-hidden">
      {/* Password Modal */}
      {powerTarget && (
        <PowerToggleModal
          agent={powerTarget}
          onClose={() => setPowerTarget(null)}
          onSuccess={onRefresh}
        />
      )}

      {/* Delete confirmation modal */}
      {deleteTarget && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-[var(--surface-overlay)] backdrop-blur-sm">
          <div className="bg-surface-primary border border-edge rounded-2xl shadow-xl p-6 w-80 space-y-4">
            <div className="flex items-center gap-3">
              <div className="p-2 rounded-xl bg-red-500/10 text-red-500"><Trash2 size={18} /></div>
              <div>
                <h3 className="font-bold text-content-primary text-sm">Delete Agent</h3>
                <p className="text-[10px] text-content-tertiary font-mono mt-0.5">Irreversible operation</p>
              </div>
            </div>
            <div className="bg-surface-secondary rounded-xl p-3 space-y-1">
              <p className="text-xs font-bold text-content-primary">{deleteTarget.name}</p>
              <p className="text-[10px] text-content-tertiary font-mono">{deleteTarget.id}</p>
            </div>
            <p className="text-xs text-content-secondary">
              All chat history for this agent will be permanently deleted. This cannot be undone.
            </p>
            {deleteError && (
              <p className="text-xs text-red-400">{deleteError}</p>
            )}
            <div className="flex gap-2 pt-1">
              <button
                onClick={() => { setDeleteTarget(null); setDeleteError(null); }}
                disabled={isDeleting}
                className="flex-1 py-2 rounded-xl border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                className="flex-1 py-2 rounded-xl bg-red-500 text-white text-xs font-bold hover:bg-red-600 transition-all disabled:opacity-50 flex items-center justify-center gap-1"
              >
                {isDeleting ? <Activity size={12} className="animate-spin" /> : <Trash2 size={12} />}
                Delete
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Main content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="p-6 flex items-center justify-between">
          <div>
            <h2 className="text-xl font-black tracking-tight text-content-primary uppercase">Agent Management</h2>
            <p className="text-[10px] text-content-tertiary font-mono tracking-widest uppercase mt-1">
              EXIV-SYSTEM / Registered Instances
            </p>
          </div>
          <div className="px-3 py-1 rounded-full bg-surface-secondary text-[10px] font-bold text-content-secondary">
            {agents.filter(a => a.enabled).length} / {agents.length} ACTIVE
          </div>
        </div>

        {/* Agent List */}
        <div className="flex-1 overflow-y-auto p-6 space-y-3 no-scrollbar bg-gradient-to-b from-surface-primary/40 from-25% via-surface-primary/20 via-65% to-brand/[0.05]">
          {agents.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
              <Zap size={32} strokeWidth={1} className="opacity-20" />
              <p className="text-[10px] font-mono tracking-[0.2em] uppercase">No agents registered</p>
            </div>
          ) : (
            agents.map((agent) => (
              <div
                key={agent.id}
                className="group p-4 bg-surface-primary border border-edge rounded-xl shadow-sm flex items-center gap-4 cursor-pointer"
                onClick={() => onSelectAgent(agent)}
              >
                <div className="p-2.5 rounded-xl shrink-0" style={{ backgroundColor: `${agentColor(agent)}12`, color: agentColor(agent) }}>
                  <AgentIcon agent={agent} size={22} />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="font-bold text-content-primary text-sm truncate">{agent.name}</h3>
                    <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[8px] font-bold ${statusBadgeClass(agent.status)}`}>
                      {agent.metadata?.has_power_password === 'true' && <Lock size={7} />}
                      {agent.status.toUpperCase()}
                    </span>
                  </div>
                  <p className="text-[11px] text-content-secondary mt-0.5 truncate">{agent.description}</p>
                  <div className="flex gap-2 mt-2">
                    <span className="text-[9px] bg-surface-secondary px-1.5 py-0.5 rounded text-content-tertiary font-mono">
                      ENGINE: {agent.default_engine_id || 'DEFAULT'}
                    </span>
                    <span className="text-[9px] bg-surface-secondary px-1.5 py-0.5 rounded text-content-tertiary font-mono">
                      MEM: {agent.metadata?.preferred_memory || 'DEFAULT'}
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <AgentPowerButton agent={agent} onPowerToggle={handlePowerToggle} />
                  <button
                    title="Manage Plugins"
                    className="p-2 rounded-lg border border-edge-subtle text-content-tertiary hover:text-brand hover:border-brand/30 hover:bg-brand/5 transition-all"
                    onClick={(e) => { e.stopPropagation(); setConfiguringAgent(agent); }}
                  >
                    <Puzzle size={16} />
                  </button>
                  {agent.id === DEFAULT_AGENT_ID ? (
                    <div title="Default agent is protected" className="p-2 text-content-muted opacity-30">
                      <Lock size={15} />
                    </div>
                  ) : (
                    <button
                      title="Delete agent"
                      className="p-2 rounded-lg border border-edge-subtle text-content-tertiary hover:text-red-500 hover:border-red-500/30 hover:bg-red-500/10 transition-all"
                      onClick={(e) => { e.stopPropagation(); setDeleteTarget(agent); setDeleteError(null); }}
                    >
                      <Trash2 size={15} />
                    </button>
                  )}
                  <ChevronRight size={18} className="text-content-muted group-hover:text-content-secondary transition-colors" />
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* Right Sidebar: Create Form */}
      <div className="w-[380px] shrink-0 border-l border-[var(--border-strong)] bg-surface-base/30 overflow-y-auto no-scrollbar hidden lg:flex flex-col">
        <div className="p-5">
          <h3 className="text-[11px] font-black text-content-secondary uppercase tracking-[0.15em]">Initialize New Agent</h3>
        </div>
        <div className="p-5 flex-1">
          <div className="space-y-4">
            {/* Agent Type Selector */}
            <div>
              <label className="block text-xs font-bold text-content-secondary mb-2">Agent Type</label>
              <div className="grid grid-cols-2 gap-3">
                {([['ai', 'AI Agent', 'LLM-powered reasoning'], ['container', 'Container', 'Script / bridge process']] as const).map(([type, label, desc]) => {
                  const selected = newAgent.type === type;
                  const color = agentTypeColor(type);
                  return (
                    <button
                      key={type}
                      type="button"
                      onClick={() => handleTypeChange(type)}
                      className={`flex items-center gap-2.5 p-3 rounded-xl border-2 transition-all text-left ${
                        selected ? 'bg-surface-primary shadow-md' : 'bg-surface-primary/50 border-edge hover:border-edge'
                      }`}
                      style={selected ? { borderColor: color } : undefined}
                    >
                      <div className="p-1.5 rounded-lg text-white shrink-0" style={{ backgroundColor: selected ? color : '#94a3b8' }}>
                        <AgentTypeIcon type={type} size={16} />
                      </div>
                      <div>
                        <div className="text-[11px] font-bold text-content-primary">{label}</div>
                        <div className="text-[8px] text-content-tertiary">{desc}</div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Agent Name</label>
              <input
                type="text"
                value={newAgent.name}
                onChange={e => updateField('name', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary"
                placeholder="e.g. Mike"
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Description / System Prompt</label>
              <textarea
                value={newAgent.desc}
                onChange={e => updateField('desc', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary h-16 resize-none"
                placeholder="Briefly describe the agent's role."
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">
                {newAgent.type === 'ai' ? 'LLM Engine' : 'Bridge Engine'}
              </label>
              {filteredEngines.length > 0 ? (
                <select
                  value={newAgent.engine}
                  onChange={e => updateField('engine', e.target.value)}
                  className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                >
                  <option value="">Select Engine...</option>
                  {filteredEngines.map(p => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              ) : (
                <div className="w-full px-2 py-1.5 rounded-lg border border-dashed border-content-muted text-[10px] text-content-tertiary font-mono text-center">
                  No {newAgent.type === 'ai' ? 'LLM' : 'bridge'} engines available
                </div>
              )}
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Memory Engine</label>
              <select
                value={newAgent.memory}
                onChange={e => updateField('memory', e.target.value)}
                className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
              >
                <option value="">Select Memory...</option>
                {memories.map(p => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">
                Power Password <span className="text-content-muted font-normal">(optional)</span>
              </label>
              <div className="relative">
                <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-content-muted" />
                <input
                  type="password"
                  value={newAgent.password}
                  onChange={e => updateField('password', e.target.value)}
                  className="w-full pl-9 pr-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary"
                  placeholder="Leave empty for no password"
                />
              </div>
              <p className="text-[9px] text-content-tertiary mt-1">Require password to toggle power on/off</p>
            </div>

            {createError && (
              <p className="text-xs text-red-400 text-center px-1">{createError}</p>
            )}
            <button
              onClick={handleCreate}
              disabled={!newAgent.name || !newAgent.desc || !newAgent.engine || isCreating}
              className="w-full mt-2 text-white py-2.5 rounded-xl text-sm font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
              style={{ backgroundColor: agentTypeColor(newAgent.type) }}
            >
              {isCreating ? <Activity size={16} className="animate-spin" /> : <Plus size={16} />}
              {newAgent.type === 'ai' ? 'CREATE AI AGENT' : 'CREATE CONTAINER'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
