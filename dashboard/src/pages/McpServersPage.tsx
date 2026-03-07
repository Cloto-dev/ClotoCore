import { useState, useCallback } from 'react';
import { useMcpServers } from '../hooks/useMcpServers';
import { useAsyncAction } from '../hooks/useAsyncAction';
import { extractError } from '../lib/errors';
import { McpServerDetail } from '../components/mcp/McpServerDetail';
import { Modal } from '../components/Modal';
import { StatusDot, type StatusDotStatus } from '../components/ui/StatusDot';
import { AlertCard } from '../components/ui/AlertCard';
import { McpServerInfo } from '../types';
import { Server, Plus, RefreshCw, AlertTriangle } from 'lucide-react';
import { useApi } from '../hooks/useApi';

function mcpStatusToDot(status: McpServerInfo['status']): StatusDotStatus {
  if (status === 'Connected') return 'connected';
  if (status === 'Error') return 'error';
  return 'offline';
}

function statusLabel(status: McpServerInfo['status']) {
  if (status === 'Connected') return 'Running';
  if (status === 'Error') return 'Error';
  return 'Stopped';
}

export function McpServersPage() {
  const api = useApi();
  const { servers, isLoading, error: fetchError, refetch } = useMcpServers();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addModalOpen, setAddModalOpen] = useState(false);

  // Add server form state
  const [newName, setNewName] = useState('');
  const [newCommand, setNewCommand] = useState('python3');
  const [newArgs, setNewArgs] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const action = useAsyncAction();

  const isValidServerName = (name: string) => /^[a-z][a-z0-9._-]{0,62}[a-z0-9]$/.test(name);

  const selectedServer = servers.find(s => s.id === selectedId);

  // Category sort
  const categoryOrder: Record<string, number> = {
    'mind.': 0, 'memory.': 1, 'tool.': 2, 'voice.': 3, 'vision.': 4,
  };
  const getOrder = (id: string) => {
    for (const [prefix, order] of Object.entries(categoryOrder)) {
      if (id.startsWith(prefix)) return order;
    }
    return 9;
  };
  const sortedServers = [...servers].sort((a, b) => {
    const oa = getOrder(a.id), ob = getOrder(b.id);
    return oa !== ob ? oa - ob : a.id.localeCompare(b.id);
  });

  const running = servers.filter(s => s.status === 'Connected').length;

  const handleDelete = useCallback((id: string) => action.run(async () => {
    await api.deleteMcpServer(id);
    if (selectedId === id) setSelectedId(null);
    refetch();
  }), [api, selectedId, refetch, action.run]);

  const handleStart = useCallback((id: string) => action.run(async () => {
    await api.startMcpServer(id);
    setTimeout(refetch, 500);
  }), [api, refetch, action.run]);

  const handleStop = useCallback((id: string) => action.run(async () => {
    await api.stopMcpServer(id);
    setTimeout(refetch, 500);
  }), [api, refetch, action.run]);

  const handleRestart = useCallback((id: string) => action.run(async () => {
    await api.restartMcpServer(id);
    setTimeout(refetch, 500);
  }), [api, refetch, action.run]);

  async function handleAdd() {
    if (!newName.trim()) return;
    setAdding(true);
    setAddError(null);
    try {
      const args = newArgs.trim() ? newArgs.split(/\s+/) : [];
      await api.createMcpServer({ name: newName.trim(), command: newCommand, args });
      setAddModalOpen(false);
      setNewName('');
      setNewArgs('');
      refetch();
    } catch (err) {
      setAddError(extractError(err, 'Failed to add server'));
    } finally {
      setAdding(false);
    }
  }

  return (
    <div className="h-full flex flex-col overflow-hidden">
      {/* Header bar */}
      <div className="flex items-center gap-3 px-5 py-3 border-b border-edge shrink-0">
        <Server size={14} className="text-brand" />
        <span className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary">MCP Servers</span>
        <span className="text-[10px] font-mono text-content-muted ml-1">
          {servers.length} servers &middot; {running} running
        </span>
        <div className="ml-auto flex items-center gap-1">
          <button
            onClick={refetch}
            className="p-1.5 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors"
            title="Refresh"
          >
            <RefreshCw size={14} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button
            onClick={() => setAddModalOpen(true)}
            className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-brand/10 hover:bg-brand/20 border border-brand/30 text-brand text-[10px] font-mono font-bold tracking-wide transition-colors"
          >
            <Plus size={12} />
            Add Server
          </button>
        </div>
      </div>

      {/* Action error banner */}
      {action.error && (
        <AlertCard className="mx-5 mt-1 shrink-0">{action.error}</AlertCard>
      )}

      {/* Connection error */}
      {fetchError && (
        <AlertCard className="mx-5 mt-3 flex items-center gap-2 shrink-0">
          <AlertTriangle size={12} className="text-red-500 shrink-0" />
          <span>Backend unreachable</span>
        </AlertCard>
      )}

      {/* Server cards */}
      <div className="flex-1 overflow-y-auto p-5">
        {sortedServers.length === 0 && !isLoading && !fetchError && (
          <div className="flex flex-col items-center justify-center h-full text-content-muted">
            <Server size={32} className="mb-3 opacity-30" />
            <p className="text-xs font-mono">No MCP servers configured</p>
            <button
              onClick={() => setAddModalOpen(true)}
              className="mt-3 flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-brand/10 hover:bg-brand/20 border border-brand/30 text-brand text-[10px] font-mono font-bold transition-colors"
            >
              <Plus size={12} />
              Add Server
            </button>
          </div>
        )}

        <div className="grid grid-cols-[repeat(auto-fill,minmax(240px,1fr))] gap-3">
          {sortedServers.map(server => (
            <button
              key={server.id}
              onClick={() => setSelectedId(server.id)}
              className="text-left p-4 rounded-xl border border-edge bg-surface-primary hover:bg-surface-secondary/80 hover:border-brand/30 transition-all duration-200 group"
            >
              <div className="flex items-center gap-2.5 mb-2">
                <Server size={14} className="text-content-tertiary group-hover:text-brand transition-colors shrink-0" />
                <span className="text-xs font-mono font-bold text-content-primary truncate">{server.id}</span>
                {server.source === 'config' && (
                  <span className="text-[8px] font-mono text-amber-500/70 shrink-0" title="Config-loaded">CONFIG</span>
                )}
              </div>
              <div className="flex items-center gap-3 text-[10px] font-mono text-content-tertiary">
                <span className="flex items-center gap-1.5">
                  <StatusDot status={mcpStatusToDot(server.status)} />
                  {statusLabel(server.status)}
                </span>
                <span>{server.tools.length} tools</span>
                {server.is_cloto_sdk && <span className="text-brand">SDK</span>}
              </div>
            </button>
          ))}
        </div>
      </div>

      {/* Server Detail Modal */}
      {selectedServer && (
        <Modal title={selectedServer.id} icon={Server} size="lg" onClose={() => setSelectedId(null)}>
          <McpServerDetail
            server={selectedServer}
            onRefresh={refetch}
            onDelete={handleDelete}
            onStart={handleStart}
            onStop={handleStop}
            onRestart={handleRestart}
          />
        </Modal>
      )}

      {/* Add Server Modal */}
      {addModalOpen && (
        <Modal title="Add MCP Server" icon={Plus} size="sm" onClose={() => { setAddModalOpen(false); setAddError(null); }}>
          <div className="px-5 py-4 space-y-3">
            {addError && <AlertCard>{addError}</AlertCard>}

            <div>
              <label className="block text-[10px] font-mono text-content-muted mb-1">Server Name</label>
              <input
                type="text"
                value={newName}
                onChange={e => setNewName(e.target.value)}
                placeholder="my-server"
                className="w-full text-xs font-mono bg-glass border border-edge rounded px-2 py-1.5 text-content-primary placeholder:text-content-muted"
              />
              <p className="mt-0.5 text-[9px] font-mono text-content-muted">Lowercase letters, digits, dots, hyphens (e.g. tool.terminal)</p>
            </div>
            <div>
              <label className="block text-[10px] font-mono text-content-muted mb-1">Command</label>
              <input
                type="text"
                value={newCommand}
                onChange={e => setNewCommand(e.target.value)}
                placeholder="python3"
                className="w-full text-xs font-mono bg-glass border border-edge rounded px-2 py-1.5 text-content-primary placeholder:text-content-muted"
              />
            </div>
            <div>
              <label className="block text-[10px] font-mono text-content-muted mb-1">Arguments (space-separated)</label>
              <input
                type="text"
                value={newArgs}
                onChange={e => setNewArgs(e.target.value)}
                placeholder="scripts/my_server.py"
                className="w-full text-xs font-mono bg-glass border border-edge rounded px-2 py-1.5 text-content-primary placeholder:text-content-muted"
              />
            </div>

            <div className="flex justify-end gap-2 pt-1">
              <button
                onClick={() => { setAddModalOpen(false); setAddError(null); }}
                className="px-3 py-1.5 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge"
              >
                Cancel
              </button>
              <button
                onClick={handleAdd}
                disabled={adding || !isValidServerName(newName.trim())}
                className="px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 transition-colors border border-brand/20"
              >
                {adding ? 'Adding...' : 'Add Server'}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
