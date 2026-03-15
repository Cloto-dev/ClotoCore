import { Play, RotateCcw, Square, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { McpServerInfo } from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { McpAccessControlTab } from './McpAccessControlTab';
import { McpServerLogsTab } from './McpServerLogsTab';
import { McpServerSettingsTab } from './McpServerSettingsTab';

type Tab = 'settings' | 'access' | 'logs';

interface Props {
  server: McpServerInfo;
  onRefresh: () => void;
  onDelete: (id: string) => Promise<void>;
  onStart: (id: string) => Promise<void>;
  onStop: (id: string) => Promise<void>;
  onRestart: (id: string) => Promise<void>;
}

export function McpServerDetail({ server, onRefresh, onDelete, onStart, onStop, onRestart }: Props) {
  const { t } = useTranslation('mcp');
  const [activeTab, setActiveTab] = useState<Tab>('settings');
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

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
    { id: 'settings', label: t('detail.tab_settings') },
    { id: 'access', label: t('detail.tab_access') },
    { id: 'logs', label: t('detail.tab_logs') },
  ];

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="px-4 py-3 border-b border-edge">
        <div className="flex items-center gap-4 text-[10px] font-mono text-content-tertiary">
          <span className="flex items-center gap-1">
            {t('detail.status')}
            <span className={isRunning ? 'text-green-500' : isError ? 'text-red-500' : 'text-content-tertiary'}>
              {isRunning ? `● ${t('status_running')}` : isError ? `◉ ${t('status_error')}` : `○ ${t('status_stopped')}`}
            </span>
          </span>
          <span>{t('detail.tools_registered', { count: server.tools.length })}</span>
          {server.is_cloto_sdk && <span className="text-brand">{t('detail.cloto_sdk')}</span>}
          <span className={server.source === 'config' ? 'text-amber-500' : 'text-blue-400'}>
            {server.source === 'config' ? t('detail.source_config') : t('detail.source_dynamic')}
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
              <Play size={10} /> {t('detail.start')}
            </button>
          )}
          {isRunning && (
            <button
              onClick={() => handleAction('stop', () => onStop(server.id))}
              disabled={actionLoading !== null}
              className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-secondary hover:text-red-500 transition-colors border border-edge"
            >
              <Square size={10} /> {t('detail.stop')}
            </button>
          )}
          <button
            onClick={() => handleAction('restart', () => onRestart(server.id))}
            disabled={actionLoading !== null}
            className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-secondary hover:text-brand transition-colors border border-edge"
          >
            <RotateCcw size={10} /> {t('detail.restart')}
          </button>
          {server.source === 'dynamic' && (
            <>
              <div className="w-px h-4 bg-edge self-center mx-0.5" />
              <button
                onClick={() => setConfirmDelete(true)}
                disabled={actionLoading !== null}
                className="flex items-center gap-1 px-2 py-1 text-[10px] font-mono rounded bg-glass hover:bg-red-500/10 text-content-secondary hover:text-red-500 transition-colors border border-edge"
              >
                <Trash2 size={10} /> {t('detail.delete')}
              </button>
              <ConfirmDialog
                open={confirmDelete}
                title={t('detail.delete')}
                message={t('detail.delete_confirm', { id: server.id })}
                confirmLabel={t('detail.delete')}
                variant="danger"
                onConfirm={() => {
                  setConfirmDelete(false);
                  handleAction('delete', () => onDelete(server.id));
                }}
                onCancel={() => setConfirmDelete(false)}
              />
            </>
          )}
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-edge">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`px-4 py-2 text-[10px] font-mono uppercase tracking-wider transition-colors
              ${
                activeTab === tab.id
                  ? 'text-content-primary border-b-2 border-brand'
                  : 'text-content-tertiary hover:text-content-secondary'
              }`}
          >
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-y-auto">
        {activeTab === 'settings' && <McpServerSettingsTab server={server} onRefresh={onRefresh} />}
        {activeTab === 'access' && <McpAccessControlTab server={server} />}
        {activeTab === 'logs' && <McpServerLogsTab server={server} />}
      </div>
    </div>
  );
}
