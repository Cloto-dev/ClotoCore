import { Save } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { useAsyncAction } from '../../hooks/useAsyncAction';
import type { AccessControlEntry, AccessTreeResponse, AgentMetadata, McpServerInfo } from '../../types';
import { AlertCard } from '../ui/AlertCard';
import { McpAccessSummaryBar } from './McpAccessSummaryBar';
import { McpAccessTree } from './McpAccessTree';

interface Props {
  server: McpServerInfo;
}

export function McpAccessControlTab({ server }: Props) {
  const api = useApi();
  const { t } = useTranslation('mcp');
  const [accessData, setAccessData] = useState<AccessTreeResponse | null>(null);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>('');
  const [localEntries, setLocalEntries] = useState<AccessControlEntry[]>([]);
  const [dirty, setDirty] = useState(false);
  const loadAction = useAsyncAction('Failed to load access data');
  const saveAction = useAsyncAction('Failed to save access control');

  const loadData = useCallback(async () => {
    await loadAction.run(async () => {
      const [access, agentList] = await Promise.all([api.getMcpServerAccess(server.id), api.getAgents()]);
      setAccessData(access);
      setAgents(agentList);
      setLocalEntries(access.entries);
      if (agentList.length > 0) {
        setSelectedAgent((prev) => prev || agentList[0].id);
      }
      setDirty(false);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server.id]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  function handleEntriesChange(updated: AccessControlEntry[]) {
    setLocalEntries(updated);
    setDirty(true);
  }

  async function handleSave() {
    await saveAction.run(async () => {
      const toSave = localEntries.filter((e) => e.entry_type !== 'capability');
      await api.putMcpServerAccess(server.id, toSave);
      await loadData();
    });
  }

  const error = loadAction.error || saveAction.error;
  const saving = saveAction.isLoading;

  const serverGrantCount = localEntries.filter(
    (e) => e.entry_type === 'server_grant' && e.server_id === server.id,
  ).length;

  return (
    <div className="p-4 space-y-4">
      {error && <AlertCard>{error}</AlertCard>}

      {/* Default Policy Display */}
      {accessData && (
        <div className="text-[10px] font-mono text-content-tertiary">
          {t('access.default_policy')} <span className="text-content-secondary">{accessData.default_policy}</span>
          {accessData.default_policy === 'opt-in'
            ? ` ${t('access.deny_by_default')}`
            : ` ${t('access.allow_by_default')}`}
        </div>
      )}

      {/* Summary Bar */}
      <McpAccessSummaryBar
        tools={accessData?.tools ?? server.tools}
        entries={localEntries}
        serverGrantCount={serverGrantCount}
      />

      {/* Agent Selector */}
      <div className="flex items-center gap-2">
        <label className="text-[10px] font-mono text-content-tertiary">{t('access.agent')}</label>
        <select
          value={selectedAgent}
          onChange={(e) => setSelectedAgent(e.target.value)}
          className="text-xs font-mono bg-glass border border-edge rounded px-2 py-1 text-content-primary"
        >
          {agents.map((agent) => (
            <option key={agent.id} value={agent.id}>
              {agent.id} — {agent.name}
            </option>
          ))}
        </select>
      </div>

      {/* Access Tree */}
      {selectedAgent && (
        <div className="border border-edge rounded p-2 bg-glass">
          <McpAccessTree
            entries={localEntries}
            tools={accessData?.tools ?? server.tools}
            agentId={selectedAgent}
            serverId={server.id}
            onChange={handleEntriesChange}
          />
        </div>
      )}

      {/* Save button */}
      {dirty && (
        <div className="flex gap-2 pt-2 border-t border-edge">
          <button
            onClick={handleSave}
            disabled={saving}
            aria-label={t('access.save_changes')}
            className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 transition-colors border border-brand/20"
          >
            <Save size={10} /> {saving ? t('access.saving') : t('access.save_changes')}
          </button>
          <button
            onClick={loadData}
            aria-label={t('access.discard')}
            className="px-3 py-1.5 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge"
          >
            {t('access.discard')}
          </button>
        </div>
      )}
    </div>
  );
}
