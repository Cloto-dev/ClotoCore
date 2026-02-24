import { useState } from 'react';
import { McpServerInfo } from '../../types';
import { McpServerSettingsTab } from './McpServerSettingsTab';
import { McpAccessControlTab } from './McpAccessControlTab';
import { McpServerLogsTab } from './McpServerLogsTab';
import { Play, Square, RotateCcw, Trash2 } from 'lucide-react';

type Tab = 'settings' | 'access' | 'logs';

interface Props {
  server: McpServerInfo;
  apiKey: string;
  onRefresh: () => void;
  onDelete: (id: string) => Promise<void>;
  onStart: (id: string) => Promise<void>;
  onStop: (id: string) => Promise<void>;
  onRestart: (id: string) => Promise<void>;
}

export function McpServerDetail({ server, apiKey, onRefresh, onDelete, onStart, onStop, onRestart }: Props) {
  const [activeTab, setActiveTab] = useState<Tab>('settings');
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const isRunning = server.status === 'Connected';
  const isError = server.status === 'Error';

  async function handleAction(action: string, fn: () => Promise<void>) {
    setActionLoading(action);
    try {
      await fn();
      setTimeout(onRefresh, 500);
    } finally {
      setActionLoading(null);
    }
  }

  const tabs: { id: Tab; label: string }[] = [
    { id: 'settings', label: 'Settings' },
    { id: 'access', label: 'Access' },
    { id: 'logs', label: 'Logs' },
  ];

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="px-4 py-3 border-b border-edge">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-mono font-medium text-content-primary">{server.id}</h2>
          {server.source === 'dynamic' && (
            <button
              onClick={() => {
                if (confirm(`Delete server '${server.id}'?`))
                  handleAction('delete', () => onDelete(server.id));
              }}
              disabled={actionLoading !== null}
              className="p-1 rounded hover:bg-red-500/10 text-content-muted hover:text-red-500 transition-colors disabled:opacity-40"
              title="Delete Server"
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
        <div className="flex items-center gap-4 mt-1.5 text-[10px] font-mono text-content-tertiary">
          <span className="flex items-center gap-1">
            Status:
            <span className={isRunning ? 'text-green-500' : isError ? 'text-red-500' : 'text-content-muted'}>
              {isRunning ? '● Running' : isError ? '◉ Error' : '○ Stopped'}
            </span>
          </span>
          <span>Tools: {server.tools.length} registered</span>
          {server.is_cloto_sdk && <span className="text-brand">CLOTO SDK</span>}
          <span className={server.source === 'config' ? 'text-amber-500' : 'text-blue-400'}>
            {server.source === 'config' ? 'CONFIG' : 'DYNAMIC'}
          </span>
        </div>

        {/* Lifecycle buttons */}
        <div className="flex gap-1.5 mt-2">
          {!isRunning && (
            <button
              onClick={() => handleAction('start', () => onStart(server.id))}
              disabled={actionLoading !== null}
              className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-secondary hover:text-green-500 transition-colors border border-edge"
            >
              <Play size={10} /> Start
            </button>
          )}
          {isRunning && (
            <button
              onClick={() => handleAction('stop', () => onStop(server.id))}
              disabled={actionLoading !== null}
              className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-secondary hover:text-red-500 transition-colors border border-edge"
            >
              <Square size={10} /> Stop
            </button>
          )}
          <button
            onClick={() => handleAction('restart', () => onRestart(server.id))}
            disabled={actionLoading !== null}
            className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-secondary hover:text-brand transition-colors border border-edge"
          >
            <RotateCcw size={10} /> Restart
          </button>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-edge">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-4 py-2 text-[10px] font-mono uppercase tracking-wider transition-colors
              ${activeTab === tab.id
                ? 'text-content-primary border-b-2 border-brand'
                : 'text-content-tertiary hover:text-content-secondary'}`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-y-auto">
        {activeTab === 'settings' && (
          <McpServerSettingsTab server={server} apiKey={apiKey} onRefresh={onRefresh} />
        )}
        {activeTab === 'access' && (
          <McpAccessControlTab server={server} apiKey={apiKey} />
        )}
        {activeTab === 'logs' && (
          <McpServerLogsTab server={server} />
        )}
      </div>
    </div>
  );
}
