import { McpServerInfo } from '../../types';
import { Server, Plus, RefreshCw } from 'lucide-react';

interface Props {
  servers: McpServerInfo[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onRefresh: () => void;
  isLoading: boolean;
}

function statusIndicator(status: McpServerInfo['status']) {
  switch (status) {
    case 'Connected': return <span className="text-green-500" title="Running">●</span>;
    case 'Disconnected': return <span className="text-content-muted" title="Stopped">○</span>;
    case 'Error': return <span className="text-red-500" title="Error">◉</span>;
  }
}

export function McpServerList({ servers, selectedId, onSelect, onAdd, onRefresh, isLoading }: Props) {
  const running = servers.filter(s => s.status === 'Connected').length;
  const stopped = servers.filter(s => s.status !== 'Connected').length;

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-edge">
        <span className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary">MCP Servers</span>
        <div className="flex gap-1">
          <button onClick={onRefresh} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors" title="Refresh">
            <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button onClick={onAdd} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors" title="Add Server">
            <Plus size={12} />
          </button>
        </div>
      </div>

      {/* Server list */}
      <div className="flex-1 overflow-y-auto py-1">
        {servers.length === 0 && !isLoading && (
          <div className="px-3 py-4 text-center text-[10px] text-content-muted font-mono">NO SERVERS</div>
        )}
        {servers.map(server => (
          <button
            key={server.id}
            onClick={() => onSelect(server.id)}
            className={`w-full text-left px-3 py-2 flex items-center gap-2 transition-colors text-xs font-mono
              ${selectedId === server.id
                ? 'bg-glass-strong text-content-primary'
                : 'hover:bg-glass text-content-secondary hover:text-content-primary'}`}
          >
            <span className="text-[10px]">{statusIndicator(server.status)}</span>
            <Server size={12} className="text-content-tertiary flex-shrink-0" />
            <span className="truncate">{server.id}</span>
            <span className="ml-auto text-[9px] text-content-muted">{server.tools.length}t</span>
          </button>
        ))}
      </div>

      {/* Status bar */}
      <div className="px-3 py-1.5 border-t border-edge text-[9px] font-mono text-content-muted">
        {servers.length} servers | {running} running | {stopped} stopped
      </div>
    </div>
  );
}
