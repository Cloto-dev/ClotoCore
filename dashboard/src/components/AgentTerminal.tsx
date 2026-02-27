import { useState } from 'react';
import { Users, Puzzle, Activity, Zap, Plus, Lock, Trash2, MessageSquare, Settings } from 'lucide-react';
import { AgentMetadata, PluginManifest } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor, AgentTypeIcon, agentTypeColor, isAiAgent } from '../lib/agentIdentity';

import { useAgentCreation } from '../hooks/useAgentCreation';
import { PowerToggleModal } from './PowerToggleModal';
import { AgentConsole } from './AgentConsole';
import { ContainerDashboard } from './ContainerDashboard';
import { AgentPowerButton } from './AgentPowerButton';

import { api, EVENTS_URL } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useMcpServers } from '../hooks/useMcpServers';

export interface AgentTerminalProps {
  agents: AgentMetadata[];
  plugins: PluginManifest[];
  selectedAgent: AgentMetadata | null;
  onSelectAgent: (agent: AgentMetadata | null) => void;
  onRefresh: () => void;
  onBack?: () => void;
}

export function AgentTerminal({
  agents,
  plugins,
  selectedAgent,
  onSelectAgent,
  onRefresh,
  onBack,
}: AgentTerminalProps) {
  const { apiKey } = useApiKey();
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  // Power toggle modal
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<AgentMetadata | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const DEFAULT_AGENT_ID = 'agent.cloto_default';

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

  const handlePowerToggle = (agent: AgentMetadata) => {
    setPowerTarget(agent);
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

  // MCP-based engine/memory discovery (mind.* = reasoning engines, memory.* = memory backends)
  const { servers: mcpServers } = useMcpServers(apiKey);
  const mcpEngines = mcpServers.filter(s => s.id.startsWith('mind.') && s.status === 'Connected');
  const mcpMemories = mcpServers.filter(s => s.id.startsWith('memory.') && s.status === 'Connected');


  return (
    <div className="relative flex h-full overflow-hidden">
      {/* Power Toggle Modal */}
      {powerTarget && (
        <PowerToggleModal
          agent={powerTarget}
          onClose={() => setPowerTarget(null)}
          onSuccess={onRefresh}
        />
      )}

      {/* Delete Confirmation Modal */}
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

      {/* Main Content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        <div className="flex-1 overflow-y-auto no-scrollbar p-6 md:p-8">

          {/* Section: Agents */}
          <div className="flex items-center gap-3 mb-4 border-b border-edge pb-2">
            <Users className="text-brand" size={16} />
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Agents</h2>
          </div>

          {/* Agent Cards Grid */}
          {agents.length === 0 ? (
            <div className="py-12 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
              No agents registered. Create one to get started.
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {agents.map((agent) => {
                const color = agentColor(agent);
                const isAi = isAiAgent(agent);
                return (
                  <div
                    key={agent.id}
                    className="bg-glass-strong backdrop-blur-sm p-4 rounded-lg shadow-sm hover:shadow-md transition-all duration-300 border border-edge hover:border-brand group cursor-pointer"
                    onClick={() => onSelectAgent(agent)}
                  >
                    {/* Row 1: Status + Name + Power */}
                    <div className="flex items-center gap-3 mb-2">
                      <div className={`w-3 h-3 rounded-full flex-shrink-0 ${agent.enabled ? 'bg-emerald-500' : 'bg-content-muted'}`} />
                      <h3 className="font-bold text-content-primary text-sm flex-1 truncate">{agent.name}</h3>
                      <AgentPowerButton agent={agent} onPowerToggle={handlePowerToggle} />
                    </div>

                    {/* Row 2: Type + Engine */}
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-[10px] font-mono text-content-tertiary">
                        {isAi ? 'AI Agent' : 'Container'} · {agent.default_engine_id || 'No engine'}
                      </span>
                    </div>

                    {/* Row 3: Memory */}
                    {agent.metadata?.preferred_memory && (
                      <div className="text-[10px] font-mono text-content-muted mb-2">
                        {agent.metadata.preferred_memory}
                      </div>
                    )}

                    {/* Divider + Actions */}
                    <div className="mt-2 pt-2 border-t border-edge-subtle flex items-center justify-between">
                      <span className="text-[9px] text-content-tertiary font-mono">
                        {agent.metadata?.has_power_password === 'true' && <Lock size={8} className="inline mr-1" />}
                        {agent.id}
                      </span>
                      <div className="flex items-center gap-1.5">
                        {isAi && (
                          <button
                            className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-brand hover:bg-brand/10 transition-all"
                            onClick={(e) => { e.stopPropagation(); onSelectAgent(agent); }}
                          >
                            <MessageSquare size={10} /> Chat
                          </button>
                        )}
                        <button
                          className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-content-tertiary hover:text-brand hover:bg-brand/10 transition-all"
                          onClick={(e) => { e.stopPropagation(); setConfiguringAgent(agent); }}
                        >
                          <Settings size={10} /> Config
                        </button>
                        {agent.id !== DEFAULT_AGENT_ID && (
                          <button
                            className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all"
                            onClick={(e) => { e.stopPropagation(); setDeleteTarget(agent); setDeleteError(null); }}
                          >
                            <Trash2 size={10} />
                          </button>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      {/* Right Sidebar: Create Form */}
      <div className="w-[340px] shrink-0 border-l border-[var(--border-strong)] bg-surface-base/30 overflow-y-auto no-scrollbar hidden lg:flex flex-col">
        <div className="p-6">
          {/* Section header */}
          <div className="flex items-center gap-3 mb-6 border-b border-edge pb-2">
            <Zap className="text-brand" size={16} />
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Create Agent</h2>
          </div>

          <div className="space-y-4">
            {/* Agent Type Selector */}
            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-2">Type</label>
              <div className="grid grid-cols-2 gap-2">
                {([['ai', 'AI Agent', 'LLM-powered'], ['container', 'Container', 'Bridge / Script']] as const).map(([type, label, desc]) => {
                  const selected = newAgent.type === type;
                  const color = agentTypeColor(type);
                  return (
                    <button
                      key={type}
                      type="button"
                      onClick={() => handleTypeChange(type)}
                      className={`flex items-center gap-2 p-2.5 rounded-lg border transition-all text-left ${
                        selected ? 'bg-glass-strong shadow-sm border-brand' : 'bg-glass border-edge hover:border-edge'
                      }`}
                    >
                      <div className="p-1 rounded shrink-0" style={{ backgroundColor: selected ? `${color}20` : undefined, color: selected ? color : '#94a3b8' }}>
                        <AgentTypeIcon type={type} size={14} />
                      </div>
                      <div>
                        <div className="text-[10px] font-bold text-content-primary">{label}</div>
                        <div className="text-[8px] text-content-muted">{desc}</div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">Name</label>
              <input
                type="text"
                value={newAgent.name}
                onChange={e => updateField('name', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                placeholder="e.g. Mike"
              />
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">Description</label>
              <textarea
                value={newAgent.desc}
                onChange={e => updateField('desc', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary h-16 resize-none"
                placeholder="Describe the agent's role"
              />
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
                {newAgent.type === 'ai' ? 'LLM Engine' : 'Bridge Engine'}
              </label>
              {mcpEngines.length > 0 ? (
                <select
                  value={newAgent.engine}
                  onChange={e => updateField('engine', e.target.value)}
                  className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                >
                  <option value="">Select...</option>
                  {mcpEngines.map(s => (
                    <option key={s.id} value={s.id}>{s.id.replace('mind.', '')}</option>
                  ))}
                </select>
              ) : (
                <div className="w-full px-2 py-1.5 rounded-lg border border-dashed border-content-muted text-[10px] text-content-tertiary font-mono text-center">
                  No engines available
                </div>
              )}
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">Memory</label>
              <select
                value={newAgent.memory}
                onChange={e => updateField('memory', e.target.value)}
                className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
              >
                <option value="">None</option>
                {mcpMemories.map(s => (
                  <option key={s.id} value={s.id}>{s.id.replace('memory.', '')}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
                Password <span className="text-content-muted font-normal normal-case">(optional)</span>
              </label>
              <div className="relative">
                <Lock size={12} className="absolute left-3 top-1/2 -translate-y-1/2 text-content-muted" />
                <input
                  type="password"
                  value={newAgent.password}
                  onChange={e => updateField('password', e.target.value)}
                  className="w-full pl-8 pr-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                  placeholder="Power toggle password"
                />
              </div>
            </div>

            {createError && (
              <p className="text-[10px] text-red-400 text-center">{createError}</p>
            )}
            <button
              onClick={handleCreate}
              disabled={!newAgent.name || !newAgent.desc || !newAgent.engine || isCreating}
              className="w-full text-white py-2 rounded-lg text-xs font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-1.5"
              style={{ backgroundColor: agentTypeColor(newAgent.type) }}
            >
              {isCreating ? <Activity size={14} className="animate-spin" /> : <Plus size={14} />}
              Create {newAgent.type === 'ai' ? 'AI Agent' : 'Container'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
