import { useState, useCallback } from 'react';
import { Link } from 'react-router-dom';
import { ArrowLeft, Server } from 'lucide-react';
import { useMcpServers } from '../hooks/useMcpServers';
import { useApiKey } from '../contexts/ApiKeyContext';
import { McpServerList } from '../components/mcp/McpServerList';
import { McpServerDetail } from '../components/mcp/McpServerDetail';
import { api } from '../services/api';

export function McpServersPage() {
  const { apiKey } = useApiKey();
  // Allow empty apiKey — debug backend skips auth when EXIV_API_KEY is unset
  const effectiveKey = apiKey || '';
  const { servers, isLoading, refetch } = useMcpServers(effectiveKey);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [addModalOpen, setAddModalOpen] = useState(false);

  // Add server form state
  const [newName, setNewName] = useState('');
  const [newCommand, setNewCommand] = useState('python3');
  const [newArgs, setNewArgs] = useState('');
  const [addError, setAddError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const isValidServerName = (name: string) => /^[a-z][a-z0-9._-]{0,62}[a-z0-9]$/.test(name);

  const selectedServer = servers.find(s => s.id === selectedId);

  const handleDelete = useCallback(async (id: string) => {
    try {
      setActionError(null);
      await api.deleteMcpServer(id, effectiveKey);
      if (selectedId === id) setSelectedId(null);
      refetch();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to delete server');
    }
  }, [effectiveKey, selectedId, refetch]);

  const handleStart = useCallback(async (id: string) => {
    try {
      setActionError(null);
      await api.startMcpServer(id, effectiveKey);
      setTimeout(refetch, 500);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to start server');
    }
  }, [effectiveKey, refetch]);

  const handleStop = useCallback(async (id: string) => {
    try {
      setActionError(null);
      await api.stopMcpServer(id, effectiveKey);
      setTimeout(refetch, 500);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to stop server');
    }
  }, [effectiveKey, refetch]);

  const handleRestart = useCallback(async (id: string) => {
    try {
      setActionError(null);
      await api.restartMcpServer(id, effectiveKey);
      setTimeout(refetch, 500);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Failed to restart server');
    }
  }, [effectiveKey, refetch]);

  async function handleAdd() {
    if (!newName.trim()) return;
    setAdding(true);
    setAddError(null);
    try {
      const args = newArgs.trim() ? newArgs.split(/\s+/) : [];
      await api.createMcpServer({ name: newName.trim(), command: newCommand, args }, effectiveKey);
      setAddModalOpen(false);
      setNewName('');
      setNewArgs('');
      refetch();
    } catch (err) {
      setAddError(err instanceof Error ? err.message : 'Failed to add server');
    } finally {
      setAdding(false);
    }
  }

  return (
    <div className="min-h-screen bg-surface-base flex flex-col">
      {/* Top bar */}
      <header className="flex items-center gap-3 px-4 py-2 border-b border-edge bg-surface-primary">
        <Link to="/" className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors">
          <ArrowLeft size={16} />
        </Link>
        <Server size={14} className="text-brand" />
        <h1 className="text-xs font-mono uppercase tracking-widest text-content-primary">MCP Server Management</h1>
      </header>

      {/* Action error banner */}
      {actionError && (
        <div className="px-4 py-2 text-[10px] font-mono text-red-500 bg-red-500/10 border-b border-red-500/20">
          {actionError}
        </div>
      )}

      {/* Master-Detail */}
      <div className="flex-1 flex overflow-hidden">
        {/* Left pane: Server list */}
        <div className="w-56 flex-shrink-0 border-r border-edge bg-surface-primary">
          <McpServerList
            servers={servers}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onAdd={() => setAddModalOpen(true)}
            onRefresh={refetch}
            isLoading={isLoading}
          />
        </div>

        {/* Right pane: Detail */}
        <div className="flex-1 bg-surface-base">
          {selectedServer ? (
            <McpServerDetail
              server={selectedServer}
              apiKey={effectiveKey}
              onRefresh={refetch}
              onDelete={handleDelete}
              onStart={handleStart}
              onStop={handleStop}
              onRestart={handleRestart}
            />
          ) : (
            <div className="flex items-center justify-center h-full text-xs font-mono text-content-muted">
              Select a server from the list
            </div>
          )}
        </div>
      </div>

      {/* Add Server Modal */}
      {addModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm">
          <div className="bg-surface-primary border border-edge rounded-lg shadow-xl w-96 p-4">
            <h3 className="text-xs font-mono uppercase tracking-widest text-content-primary mb-3">Add MCP Server</h3>

            {addError && (
              <div className="p-2 mb-3 text-[10px] font-mono text-red-500 bg-red-500/10 rounded border border-red-500/20">
                {addError}
              </div>
            )}

            <div className="space-y-3">
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
            </div>

            <div className="flex justify-end gap-2 mt-4">
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
        </div>
      )}
    </div>
  );
}
