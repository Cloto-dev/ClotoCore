import { useState, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Users, Activity, Zap, Plus, Lock, Trash2, MessageSquare, Settings, X, Route, Download, Upload } from 'lucide-react';
import { AgentMetadata, AccessControlEntry } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { displayServerId } from '../lib/format';

import { useAgentCreation } from '../hooks/useAgentCreation';
import { PowerToggleModal } from './PowerToggleModal';
import { AgentConsole } from './AgentConsole';
import { AgentPowerButton } from './AgentPowerButton';

import { EVENTS_URL } from '../services/api';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';

export interface AgentTerminalProps {
  agents: AgentMetadata[];
  selectedAgent: AgentMetadata | null;
  onSelectAgent: (agent: AgentMetadata | null) => void;
  onRefresh: () => void;
  onBack?: () => void;
}

export function AgentTerminal({
  agents,
  selectedAgent,
  onSelectAgent,
  onRefresh,
  onBack,
}: AgentTerminalProps) {
  const api = useApi();
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  // Power toggle modal
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<AgentMetadata | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deletePassword, setDeletePassword] = useState('');

  // MCP-based engine/memory discovery (mind.* = reasoning engines, memory.* = memory backends)
  // Must be called before any conditional returns to satisfy React's Rules of Hooks
  const { servers: mcpServers } = useMcpServers();
  const mcpEngines = mcpServers.filter(s => s.id.startsWith('mind.') && s.status === 'Connected');
  const mcpMemories = mcpServers.filter(s => s.id.startsWith('memory.') && s.status === 'Connected');

  const DEFAULT_AGENT_ID = 'agent.cloto_default';

  // Import file input
  const importRef = useRef<HTMLInputElement>(null);
  const [importWarnings, setImportWarnings] = useState<string[]>([]);

  const handleExport = async (agent: AgentMetadata) => {
    try {
      const accessData = await api.getAgentAccess(agent.id);
      const mcpAccess = (accessData.entries || [])
        .filter((e: AccessControlEntry) => e.entry_type === 'server_grant')
        .map((e: AccessControlEntry) => ({ server_id: e.server_id, permission: e.permission }));

      const { has_avatar, avatar_description, has_power_password, has_password, ...cleanMeta } = agent.metadata || {};
      const exportData = {
        cloto_agent_export: 1,
        exported_at: new Date().toISOString(),
        agent: {
          name: agent.name,
          description: agent.description,
          default_engine_id: agent.default_engine_id || null,
          metadata: cleanMeta,
          required_capabilities: agent.required_capabilities,
        },
        mcp_access: mcpAccess,
        avatar_path: has_avatar === 'true' ? `avatars/${agent.id}.png` : null,
      };

      const blob = new Blob([JSON.stringify(exportData, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${agent.name}.cloto-agent.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      console.error('Export failed:', e);
    }
  };

  const handleImport = async (file: File) => {
    const warnings: string[] = [];
    try {
      const text = await file.text();
      const data = JSON.parse(text);

      if (!data.cloto_agent_export || !data.agent?.name) {
        alert(t('import_invalid'));
        return;
      }

      const agentData = data.agent;
      const meta: Record<string, string> = { ...(agentData.metadata || {}), agent_type: agentData.metadata?.agent_type || 'ai' };

      // Check if engine exists
      let engineId = agentData.default_engine_id || '';
      if (engineId && !mcpEngines.some(s => s.id === engineId)) {
        warnings.push(t('import_engine_missing', { engine: engineId }));
        engineId = '';
      }

      // Create agent
      await api.createAgent({
        name: agentData.name,
        description: agentData.description || '',
        default_engine: engineId,
        metadata: meta,
      });

      // Find newly created agent to get its ID
      const allAgents = await api.getAgents();
      const created = allAgents.find((a: AgentMetadata) => a.name === agentData.name);

      // Restore MCP access
      if (created && Array.isArray(data.mcp_access) && data.mcp_access.length > 0) {
        const serverIds = new Set(mcpServers.map(s => s.id));
        for (const access of data.mcp_access) {
          if (!serverIds.has(access.server_id)) {
            warnings.push(t('import_server_skipped', { server: access.server_id }));
            continue;
          }
          try {
            const tree = await api.getMcpServerAccess(access.server_id);
            const newEntry: AccessControlEntry = {
              entry_type: 'server_grant',
              agent_id: created.id,
              server_id: access.server_id,
              permission: access.permission || 'allow',
              granted_by: 'import',
              granted_at: new Date().toISOString(),
            };
            await api.putMcpServerAccess(access.server_id, [...tree.entries, newEntry]);
          } catch {
            warnings.push(t('import_server_skipped', { server: access.server_id }));
          }
        }
      }

      onRefresh();
      setImportWarnings(warnings);
      if (warnings.length === 0) {
        alert(t('import_success', { name: agentData.name }));
      }
    } catch (e) {
      alert(t('import_error', { error: e instanceof Error ? e.message : 'Unknown error' }));
    }
  };

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    setDeleteError(null);
    try {
      const hasPassword = deleteTarget.metadata?.has_password === 'true';
      await api.deleteAgent(deleteTarget.id, hasPassword ? deletePassword : undefined);
      setDeleteTarget(null);
      setDeletePassword('');
      onRefresh();
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setIsDeleting(false);
    }
  };

  // Creation form
  const {
    form: newAgent, updateField, handleCreate, isCreating, createError,
    addRoutingRule, updateRoutingRule, removeRoutingRule,
  } = useAgentCreation(onRefresh);

  // Listen for AgentPowerChanged events to auto-refresh
  useEventStream(EVENTS_URL, (event) => {
    if (event.type === 'AgentPowerChanged') {
      onRefresh();
    }
  }, api.apiKey);

  const handlePowerToggle = (agent: AgentMetadata) => {
    setPowerTarget(agent);
  };

  if (configuringAgent) {
    return (
      <AgentPluginWorkspace
        agent={configuringAgent}
        onBack={() => { setConfiguringAgent(null); onRefresh(); }}
      />
    );
  }

  if (selectedAgent) {
    return <AgentConsole key={selectedAgent.id} agent={selectedAgent} onBack={() => onSelectAgent(null)} />;
  }

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
                <h3 className="font-bold text-content-primary text-sm">{t('delete.title')}</h3>
                <p className="text-[10px] text-content-tertiary font-mono mt-0.5">{t('delete.irreversible')}</p>
              </div>
            </div>
            <div className="bg-surface-secondary rounded-xl p-3 space-y-1">
              <p className="text-xs font-bold text-content-primary">{deleteTarget.name}</p>
              <p className="text-[10px] text-content-tertiary font-mono">{deleteTarget.id}</p>
            </div>
            <p className="text-xs text-content-secondary">
              {t('delete.warning')}
            </p>
            {deleteTarget.metadata?.has_password === 'true' && (
              <input
                type="password"
                value={deletePassword}
                onChange={e => setDeletePassword(e.target.value)}
                placeholder={t('delete.password_placeholder')}
                className="w-full bg-surface-base border border-edge rounded-xl px-3 py-2 text-xs font-mono text-content-primary placeholder:text-content-tertiary"
              />
            )}
            {deleteError && (
              <p className="text-xs text-red-400">{deleteError}</p>
            )}
            <div className="flex gap-2 pt-1">
              <button
                onClick={() => { setDeleteTarget(null); setDeleteError(null); setDeletePassword(''); }}
                disabled={isDeleting}
                className="flex-1 py-2 rounded-xl border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all disabled:opacity-50"
              >
                {tc('cancel')}
              </button>
              <button
                onClick={handleDeleteConfirm}
                disabled={isDeleting || (deleteTarget.metadata?.has_password === 'true' && !deletePassword)}
                className="flex-1 py-2 rounded-xl bg-red-500 text-white text-xs font-bold hover:bg-red-600 transition-all disabled:opacity-50 flex items-center justify-center gap-1"
              >
                {isDeleting ? <Activity size={12} className="animate-spin" /> : <Trash2 size={12} />}
                {tc('delete')}
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
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest flex-1">{t('title')}</h2>
            <input ref={importRef} type="file" accept=".json" className="hidden" onChange={e => { const f = e.target.files?.[0]; if (f) handleImport(f); e.target.value = ''; }} />
            <button
              onClick={() => importRef.current?.click()}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-content-tertiary hover:text-brand hover:bg-brand/10 transition-all"
            >
              <Upload size={14} /> {t('import_config')}
            </button>
          </div>

          {/* Import warnings */}
          {importWarnings.length > 0 && (
            <div className="mb-4 p-3 rounded-lg bg-amber-500/10 border border-amber-500/30 space-y-1">
              {importWarnings.map((w) => (
                <p key={w} className="text-[10px] text-amber-400 font-mono">{w}</p>
              ))}
              <button onClick={() => setImportWarnings([])} className="text-[10px] text-content-tertiary hover:text-brand mt-1">&times; {tc('close')}</button>
            </div>
          )}

          {/* Agent Cards Grid */}
          {agents.length === 0 ? (
            <div className="py-12 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
              {t('no_agents')}
            </div>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {agents.map((agent) => {
                const color = agentColor(agent);
                return (
                  <div
                    key={agent.id}
                    className="relative bg-glass-strong backdrop-blur-sm p-4 rounded-lg shadow-sm hover:shadow-md transition-all duration-300 border border-edge hover:border-brand group cursor-pointer overflow-hidden"
                    onClick={() => onSelectAgent(agent)}
                  >
                    {agent.metadata?.has_avatar === 'true' && (
                      <img
                        src={api.getAvatarUrl(agent.id)}
                        alt=""
                        className="absolute inset-0 w-full h-full object-cover opacity-10 blur-sm group-hover:opacity-15 transition-opacity duration-300 pointer-events-none"
                      />
                    )}
                    {/* Row 1: Status + Name + Power */}
                    <div className="flex items-center gap-3 mb-2">
                      <div className={`w-3 h-3 rounded-full flex-shrink-0 ${agent.enabled ? 'bg-emerald-500' : 'bg-content-muted'}`} />
                      <h3 className="font-bold text-content-primary text-base flex-1 truncate">{agent.name}</h3>
                      <AgentPowerButton agent={agent} onPowerToggle={handlePowerToggle} />
                    </div>

                    {/* Row 2: Engine · Memory */}
                    <div className="text-xs font-mono text-content-tertiary mb-2">
                      {t('ai_agent')} · {agent.default_engine_id ? displayServerId(agent.default_engine_id) : t('no_engine')}
                      {agent.metadata?.preferred_memory && ` · ${displayServerId(agent.metadata.preferred_memory)}`}
                    </div>

                    {/* Divider + Actions */}
                    <div className="mt-2 pt-2 border-t border-edge-subtle flex items-center justify-between">
                      <span className="text-[9px] text-content-tertiary font-mono">
                        {agent.metadata?.has_power_password === 'true' && <Lock size={8} className="inline mr-1" />}
                        {agent.id}
                      </span>
                      <div className="flex items-center gap-2">
                        <button
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-brand hover:bg-brand/10 transition-all"
                          onClick={(e) => { e.stopPropagation(); onSelectAgent(agent); }}
                        >
                          <MessageSquare size={14} /> {t('chat')}
                        </button>
                        {agent.id !== DEFAULT_AGENT_ID && (
                          <button
                            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-content-tertiary hover:text-brand hover:bg-brand/10 transition-all"
                            onClick={(e) => { e.stopPropagation(); handleExport(agent); }}
                          >
                            <Download size={14} /> {t('export_config')}
                          </button>
                        )}
                        <button
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-content-tertiary hover:text-brand hover:bg-brand/10 transition-all"
                          onClick={(e) => { e.stopPropagation(); setConfiguringAgent(agent); }}
                        >
                          <Settings size={14} /> {t('config')}
                        </button>
                        {agent.id !== DEFAULT_AGENT_ID && (
                          <button
                            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all"
                            onClick={(e) => { e.stopPropagation(); setDeleteTarget(agent); setDeleteError(null); }}
                          >
                            <Trash2 size={14} />
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
            <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">{t('create_agent')}</h2>
          </div>

          <div className="space-y-4">
            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">{t('form.name')}</label>
              <input
                type="text"
                value={newAgent.name}
                onChange={e => updateField('name', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                placeholder={t('form.name_placeholder')}
              />
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">{t('form.description')}</label>
              <textarea
                value={newAgent.desc}
                onChange={e => updateField('desc', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary h-16 resize-none"
                placeholder={t('form.desc_placeholder')}
              />
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
                {t('form.llm_engine')}
              </label>
              {mcpEngines.length > 0 ? (
                <select
                  value={newAgent.engine}
                  onChange={e => updateField('engine', e.target.value)}
                  className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                >
                  <option value="">{t('form.select')}</option>
                  {mcpEngines.map(s => (
                    <option key={s.id} value={s.id}>{displayServerId(s.id)}</option>
                  ))}
                </select>
              ) : (
                <div className="w-full px-2 py-1.5 rounded-lg border border-dashed border-content-muted text-[10px] text-content-tertiary font-mono text-center">
                  {t('form.no_engines')}
                </div>
              )}
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">{t('form.memory')}</label>
              <select
                value={newAgent.memory}
                onChange={e => updateField('memory', e.target.value)}
                className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
              >
                <option value="">{t('form.memory_none')}</option>
                {mcpMemories.map(s => (
                  <option key={s.id} value={s.id}>{displayServerId(s.id)}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-1">
                {t('form.password')} <span className="text-content-tertiary font-normal normal-case">({t('form.password_optional')})</span>
              </label>
              <div className="relative">
                <Lock size={12} className="absolute left-3 top-1/2 -translate-y-1/2 text-content-muted" />
                <input
                  type="password"
                  value={newAgent.password}
                  onChange={e => updateField('password', e.target.value)}
                  className="w-full pl-8 pr-3 py-2 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                  placeholder={t('form.password_placeholder')}
                />
              </div>
            </div>

            {/* Engine Routing Rules */}
            {mcpEngines.length > 1 && (
              <div>
                <label className="block text-[10px] font-bold text-content-tertiary uppercase tracking-wider mb-2">
                  <Route size={10} className="inline mr-1" />
                  {t('routing.title')}
                  <span className="text-content-tertiary font-normal normal-case ml-1">({t('routing.optional')})</span>
                </label>
                <div className="space-y-2">
                  {newAgent.routingRules.map((rule, i) => (
                    <div key={i} className="space-y-1 bg-glass rounded-lg p-2 border border-edge">
                      <div className="flex items-center gap-1.5">
                        <input
                          type="text"
                          value={rule.match}
                          onChange={e => updateRoutingRule(i, 'match', e.target.value)}
                          placeholder="contains:keyword"
                          className="flex-1 px-2 py-1 rounded border border-edge text-[10px] font-mono bg-surface-primary focus:outline-none focus:border-brand min-w-0"
                        />
                        <span className="text-[10px] text-content-muted shrink-0">&rarr;</span>
                        <select
                          value={rule.engine}
                          onChange={e => updateRoutingRule(i, 'engine', e.target.value)}
                          className="w-28 px-1 py-1 rounded border border-edge text-[10px] font-mono bg-surface-primary focus:outline-none focus:border-brand"
                        >
                          <option value="">{t('routing.select_engine')}</option>
                          {mcpEngines.map(s => (
                            <option key={s.id} value={s.id}>{displayServerId(s.id)}</option>
                          ))}
                        </select>
                        <button
                          type="button"
                          onClick={() => removeRoutingRule(i)}
                          className="p-0.5 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all shrink-0"
                        >
                          <X size={12} />
                        </button>
                      </div>
                      {/* CFR + Fallback options */}
                      <div className="flex items-center gap-2 pl-1">
                        <label className="flex items-center gap-1 text-[9px] text-content-tertiary cursor-pointer">
                          <input
                            type="checkbox"
                            checked={rule.cfr || false}
                            onChange={e => updateRoutingRule(i, 'cfr', e.target.checked)}
                            className="w-3 h-3 rounded"
                          />
                          CFR
                        </label>
                        {rule.cfr && (
                          <>
                            <span className="text-[9px] text-content-muted">&rarr;</span>
                            <select
                              value={rule.escalate_to || ''}
                              onChange={e => updateRoutingRule(i, 'escalate_to', e.target.value || undefined)}
                              className="w-24 px-1 py-0.5 rounded border border-edge text-[9px] font-mono bg-surface-primary focus:outline-none focus:border-brand"
                            >
                              <option value="">{t('routing.escalate_to')}</option>
                              {mcpEngines.filter(s => s.id !== rule.engine).map(s => (
                                <option key={s.id} value={s.id}>{displayServerId(s.id)}</option>
                              ))}
                            </select>
                          </>
                        )}
                        <span className="text-[9px] text-content-tertiary ml-1">{t('routing.fallback')}</span>
                        <select
                          value={rule.fallback || ''}
                          onChange={e => updateRoutingRule(i, 'fallback', e.target.value || undefined)}
                          className="w-24 px-1 py-0.5 rounded border border-edge text-[9px] font-mono bg-surface-primary focus:outline-none focus:border-brand"
                        >
                          <option value="">{t('routing.fallback_none')}</option>
                          {mcpEngines.filter(s => s.id !== rule.engine).map(s => (
                            <option key={s.id} value={s.id}>{displayServerId(s.id)}</option>
                          ))}
                        </select>
                      </div>
                    </div>
                  ))}
                  <button
                    type="button"
                    onClick={addRoutingRule}
                    className="w-full py-1 rounded border border-dashed border-edge text-[10px] font-bold text-content-tertiary hover:text-brand hover:border-brand transition-all flex items-center justify-center gap-1"
                  >
                    <Plus size={10} /> {t('routing.add_rule')}
                  </button>
                </div>
                <p className="text-[10px] text-content-tertiary mt-1 font-mono">
                  {t('routing.help')}
                </p>
              </div>
            )}

            {createError && (
              <p className="text-[10px] text-red-400 text-center">{createError}</p>
            )}
            <button
              onClick={handleCreate}
              disabled={!newAgent.name || !newAgent.desc || !newAgent.engine || isCreating}
              className="w-full text-white py-2 rounded-lg text-xs font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-1.5 bg-brand"
            >
              {isCreating ? <Activity size={14} className="animate-spin" /> : <Plus size={14} />}
              {t('create_agent')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
