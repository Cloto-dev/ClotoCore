import { useState, useEffect, useRef } from 'react';
import { Server, ArrowLeft, Plus, X, Save, Activity, Wifi, WifiOff, AlertTriangle } from 'lucide-react';
import { AgentMetadata, McpServerInfo, AccessControlEntry } from '../types';
import { api } from '../services/api';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useMcpServers } from '../hooks/useMcpServers';

interface Props {
  agent: AgentMetadata;
  onBack: () => void;
}

const StatusIcon = ({ status }: { status: McpServerInfo['status'] }) => {
  switch (status) {
    case 'Connected': return <Wifi size={12} className="text-emerald-500" />;
    case 'Disconnected': return <WifiOff size={12} className="text-content-muted" />;
    case 'Error': return <AlertTriangle size={12} className="text-red-500" />;
  }
};

export function AgentPluginWorkspace({ agent, onBack }: Props) {
  const { apiKey } = useApiKey();
  const { servers } = useMcpServers(apiKey);

  const [grantedIds, setGrantedIds] = useState<Set<string>>(new Set());
  const initialGrantedRef = useRef<Set<string>>(new Set());
  const [isSaving, setIsSaving] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [saveError, setSaveError] = useState('');

  // Load current access entries for this agent
  useEffect(() => {
    api.getAgentAccess(agent.id)
      .then(data => {
        const granted = new Set(
          data.entries
            .filter(e => e.entry_type === 'server_grant' && e.permission === 'allow')
            .map(e => e.server_id)
        );
        setGrantedIds(granted);
        initialGrantedRef.current = new Set(granted);
      })
      .catch(e => {
        console.error('Failed to load agent access:', e);
      })
      .finally(() => setIsLoading(false));
  }, [agent.id]);

  const grantServer = (serverId: string) => {
    setGrantedIds(prev => new Set([...prev, serverId]));
  };

  const revokeServer = (serverId: string) => {
    setGrantedIds(prev => {
      const next = new Set(prev);
      next.delete(serverId);
      return next;
    });
  };

  const handleSave = async () => {
    if (!apiKey) { setSaveError('API Key is not set.'); return; }
    setIsSaving(true);
    setSaveError('');

    try {
      const initial = initialGrantedRef.current;
      const added = [...grantedIds].filter(id => !initial.has(id));
      const removed = [...initial].filter(id => !grantedIds.has(id));

      const now = new Date().toISOString();

      // Process added servers
      for (const serverId of added) {
        const tree = await api.getMcpServerAccess(serverId, apiKey);
        const existing = tree.entries.filter(
          e => !(e.agent_id === agent.id && e.entry_type === 'server_grant')
        );
        const newEntry: AccessControlEntry = {
          entry_type: 'server_grant',
          agent_id: agent.id,
          server_id: serverId,
          permission: 'allow',
          granted_by: 'admin',
          granted_at: now,
        };
        await api.putMcpServerAccess(serverId, [...existing, newEntry], apiKey);
      }

      // Process removed servers
      for (const serverId of removed) {
        const tree = await api.getMcpServerAccess(serverId, apiKey);
        const filtered = tree.entries.filter(
          e => !(e.agent_id === agent.id && e.entry_type === 'server_grant')
        );
        await api.putMcpServerAccess(serverId, filtered, apiKey);
      }

      // Derive default_engine_id and preferred_memory from granted servers
      const grantedServers = servers.filter(s => grantedIds.has(s.id));
      const engineServer = grantedServers.find(s => s.id.startsWith('mind.'));
      const memoryServer = grantedServers.find(s => s.id.startsWith('memory.'));

      const metadata: Record<string, string> = { ...agent.metadata };
      if (memoryServer) {
        metadata.preferred_memory = memoryServer.id;
      } else {
        delete metadata.preferred_memory;
      }

      await api.updateAgent(
        agent.id,
        {
          default_engine_id: engineServer?.id,
          metadata,
        },
        apiKey,
      );

      onBack();
    } catch (err: any) {
      setSaveError(err?.message || 'Failed to save configuration');
    } finally {
      setIsSaving(false);
    }
  };

  const grantedServers = servers.filter(s => grantedIds.has(s.id));
  const availableServers = servers.filter(s => !grantedIds.has(s.id));

  return (
    <div className="flex flex-col h-full overflow-hidden animate-in fade-in duration-500">
      {/* Header */}
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
            <h1 className="text-xl font-black tracking-tighter text-content-primary uppercase">{agent.name} · MCP Access</h1>
            <p className="text-[10px] text-content-tertiary font-mono uppercase tracking-[0.2em]">Server Access Control</p>
          </div>
        </div>
        <div className="bg-glass-subtle backdrop-blur-sm px-4 py-2 rounded-md flex items-center gap-3 shadow-sm border border-edge">
          <span className="text-[9px] uppercase font-bold text-content-tertiary tracking-widest">{grantedIds.size} granted</span>
        </div>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6 md:p-8 space-y-6 no-scrollbar">
        {isLoading ? (
          <div className="py-12 text-center text-content-muted font-mono text-xs animate-pulse">Loading...</div>
        ) : (
          <>
            {/* Granted Servers */}
            <section>
              <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
                <Server className="text-brand" size={16} />
                <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Granted Servers</h2>
              </div>
              {grantedServers.length === 0 ? (
                <div className="py-8 text-center text-content-tertiary bg-glass rounded-lg border border-edge border-dashed font-mono text-xs">
                  No servers granted. Add from the list below.
                </div>
              ) : (
                <div className="space-y-2">
                  {grantedServers.map(server => (
                    <div key={server.id} className="bg-glass-strong backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-brand transition-all flex items-center gap-3 group">
                      <div className="p-1.5 rounded-md" style={{ backgroundColor: `${agentColor(agent)}15`, color: agentColor(agent) }}>
                        <Server size={16} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="text-xs font-bold text-content-primary">{server.id}</span>
                        <span className="text-[10px] text-content-muted ml-2 font-mono">{server.tools.length} tools</span>
                      </div>
                      <StatusIcon status={server.status} />
                      <span className={`text-[9px] font-bold uppercase tracking-wider px-2 py-0.5 rounded ${
                        server.status === 'Connected' ? 'bg-emerald-500/10 text-emerald-500' :
                        server.status === 'Error' ? 'bg-red-500/10 text-red-500' :
                        'bg-surface-secondary text-content-tertiary'
                      }`}>
                        {server.status}
                      </span>
                      <button
                        onClick={() => revokeServer(server.id)}
                        className="p-1.5 rounded text-content-muted hover:text-red-500 hover:bg-red-500/10 transition-all opacity-0 group-hover:opacity-100"
                        title="Revoke"
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </section>

            {/* Available Servers */}
            {availableServers.length > 0 && (
              <section>
                <div className="flex items-center gap-3 mb-3 border-b border-edge pb-2">
                  <Plus className="text-brand" size={16} />
                  <h2 className="font-bold text-xs text-content-secondary uppercase tracking-widest">Available</h2>
                </div>
                <div className="space-y-2">
                  {availableServers.map(server => (
                    <div key={server.id} className="bg-glass backdrop-blur-sm px-4 py-3 rounded-lg border border-edge hover:border-brand/50 transition-all flex items-center gap-3 group">
                      <div className="p-1.5 rounded-md text-content-muted">
                        <Server size={16} />
                      </div>
                      <div className="flex-1 min-w-0">
                        <span className="text-xs font-medium text-content-secondary">{server.id}</span>
                        <span className="text-[10px] text-content-muted ml-2 font-mono">{server.tools.length} tools</span>
                      </div>
                      <StatusIcon status={server.status} />
                      <span className={`text-[9px] font-bold uppercase tracking-wider px-2 py-0.5 rounded ${
                        server.status === 'Connected' ? 'bg-emerald-500/10 text-emerald-500' :
                        server.status === 'Error' ? 'bg-red-500/10 text-red-500' :
                        'bg-surface-secondary text-content-tertiary'
                      }`}>
                        {server.status}
                      </span>
                      <button
                        onClick={() => grantServer(server.id)}
                        className="inline-flex items-center gap-1 px-2 py-1 rounded text-[10px] font-bold text-brand hover:bg-brand/10 transition-all opacity-0 group-hover:opacity-100"
                      >
                        <Plus size={10} /> Grant
                      </button>
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
