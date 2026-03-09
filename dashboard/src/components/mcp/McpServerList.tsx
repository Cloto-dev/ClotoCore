import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { McpServerInfo } from '../../types';
import { Server, Plus, RefreshCw, AlertTriangle, X } from 'lucide-react';
import { AlertCard } from '../ui/AlertCard';
import { displayServerId } from '../../lib/format';

interface Props {
  servers: McpServerInfo[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAdd: () => void;
  onRefresh: () => void;
  isLoading: boolean;
  error?: string | null;
}

function statusIndicator(status: McpServerInfo['status']) {
  switch (status) {
    case 'Connected': return <span className="text-green-500" title="Running">●</span>;
    case 'Disconnected': return <span className="text-content-muted" title="Stopped">○</span>;
    case 'Error': return <span className="text-red-500" title="Error">◉</span>;
  }
}

export function McpServerList({ servers, selectedId, onSelect, onAdd, onRefresh, isLoading, error }: Props) {
  const { t } = useTranslation('mcp');
  const [filterText, setFilterText] = useState('');
  const [isFiltering, setIsFiltering] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Auto-focus input when entering filter mode
  useEffect(() => {
    if (isFiltering && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isFiltering]);

  // Category sort: mind > memory > tool > voice > vision > other
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

  const filteredServers = filterText
    ? sortedServers.filter(s =>
        s.id.toLowerCase().includes(filterText.toLowerCase()) ||
        s.tools.some(t => t.toLowerCase().includes(filterText.toLowerCase())) ||
        s.status.toLowerCase().includes(filterText.toLowerCase())
      )
    : sortedServers;

  const running = servers.filter(s => s.status === 'Connected').length;
  const stopped = servers.filter(s => s.status !== 'Connected').length;

  const handleExitFilter = () => {
    setIsFiltering(false);
    setFilterText('');
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header — click title to enter filter mode */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-edge">
        {isFiltering ? (
          <div className="flex items-center gap-1 flex-1 mr-1">
            <input
              ref={inputRef}
              type="text"
              value={filterText}
              onChange={e => setFilterText(e.target.value)}
              onKeyDown={e => { if (e.key === 'Escape') handleExitFilter(); }}
              placeholder={t('list.filter')}
              className="w-full bg-transparent text-[10px] font-mono uppercase tracking-widest text-content-primary placeholder:text-content-tertiary outline-none"
            />
            <button onClick={handleExitFilter} className="p-0.5 rounded hover:bg-glass text-content-muted hover:text-content-primary transition-colors" title={t('list.clear_filter')}>
              <X size={10} />
            </button>
          </div>
        ) : (
          <span
            onClick={() => setIsFiltering(true)}
            className="text-[10px] font-mono uppercase tracking-widest text-content-tertiary hover:text-content-secondary cursor-text transition-colors"
            title="Click to filter"
          >
            {t('title')}
          </span>
        )}
        <div className="flex gap-1">
          <button onClick={onRefresh} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors" title={t('refresh')}>
            <RefreshCw size={12} className={isLoading ? 'animate-spin' : ''} />
          </button>
          <button onClick={onAdd} className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-content-primary transition-colors" title={t('add_server')}>
            <Plus size={12} />
          </button>
        </div>
      </div>

      {/* Connection error */}
      {error && (
        <AlertCard className="mx-2 mt-1 flex items-center gap-1.5">
          <AlertTriangle size={10} className="text-red-500 shrink-0" />
          <span className="text-[9px] leading-tight">{t('backend_unreachable')}</span>
        </AlertCard>
      )}

      {/* Server list */}
      <div className="flex-1 overflow-y-auto py-1">
        {filteredServers.length === 0 && !isLoading && !error && (
          <div className="px-3 py-4 text-center text-[10px] text-content-tertiary font-mono">
            {filterText ? t('list.no_match') : t('list.no_servers')}
          </div>
        )}
        {filteredServers.map(server => (
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
            <span className="truncate">{displayServerId(server.id)}</span>
            {server.source === 'config' && (
              <span className="text-[9px] text-amber-500/70 flex-shrink-0" title="Config-loaded">C</span>
            )}
            <span className="ml-auto text-[9px] text-content-tertiary">{server.tools.length}t</span>
          </button>
        ))}
      </div>

      {/* Status bar */}
      <div className="px-3 py-1.5 border-t border-edge text-[9px] font-mono text-content-tertiary">
        {filterText
          ? t('list.status_bar_filtered', { filtered: filteredServers.length, total: servers.length })
          : t('list.status_bar', { count: servers.length, running, stopped })}
      </div>
    </div>
  );
}
