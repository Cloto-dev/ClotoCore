import { useState, useEffect } from 'react';
import { McpServerInfo, AccessControlEntry, AccessTreeResponse, AgentMetadata } from '../../types';
import { api } from '../../services/api';
import { McpAccessTree } from './McpAccessTree';
import { McpAccessSummaryBar } from './McpAccessSummaryBar';
import { Save } from 'lucide-react';

interface Props {
  server: McpServerInfo;
  apiKey: string;
}

export function McpAccessControlTab({ server, apiKey }: Props) {
  const [accessData, setAccessData] = useState<AccessTreeResponse | null>(null);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>('');
  const [localEntries, setLocalEntries] = useState<AccessControlEntry[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    loadData();
  }, [server.id]);

  async function loadData() {
    try {
      setError(null);
      const [access, agentList] = await Promise.all([
        api.getMcpServerAccess(server.id, apiKey),
        api.getAgents(),
      ]);
      setAccessData(access);
      setAgents(agentList);
      setLocalEntries(access.entries);
      if (!selectedAgent && agentList.length > 0) {
        setSelectedAgent(agentList[0].id);
      }
      setDirty(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load access data');
    }
  }

  function handleEntriesChange(updated: AccessControlEntry[]) {
    setLocalEntries(updated);
    setDirty(true);
  }

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      // Only send server_grant and tool_grant entries (capabilities are managed separately)
      const toSave = localEntries.filter(e => e.entry_type !== 'capability');
      await api.putMcpServerAccess(server.id, toSave, apiKey);
      await loadData();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to save access control');
    } finally {
      setSaving(false);
    }
  }

  const serverGrantCount = localEntries.filter(
    e => e.entry_type === 'server_grant' && e.server_id === server.id
  ).length;

  return (
    <div className="p-4 space-y-4">
      {error && (
        <div className="p-2 text-[10px] font-mono text-red-500 bg-red-500/10 rounded border border-red-500/20">
          {error}
        </div>
      )}

      {/* Default Policy Display */}
      {accessData && (
        <div className="text-[10px] font-mono text-content-tertiary">
          Default Policy: <span className="text-content-secondary">{accessData.default_policy}</span>
          {accessData.default_policy === 'opt-in'
            ? ' (deny by default)'
            : ' (allow by default)'}
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
        <label className="text-[10px] font-mono text-content-muted">Agent:</label>
        <select
          value={selectedAgent}
          onChange={e => setSelectedAgent(e.target.value)}
          className="text-xs font-mono bg-glass border border-edge rounded px-2 py-1 text-content-primary"
        >
          {agents.map(agent => (
            <option key={agent.id} value={agent.id}>{agent.id} — {agent.name}</option>
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
            className="flex items-center gap-1 px-3 py-1.5 text-[10px] font-mono rounded bg-brand/10 hover:bg-brand/20 text-brand disabled:opacity-40 transition-colors border border-brand/20"
          >
            <Save size={10} /> {saving ? 'Saving...' : 'Save Access Changes'}
          </button>
          <button
            onClick={loadData}
            className="px-3 py-1.5 text-[10px] font-mono rounded bg-glass hover:bg-glass-strong text-content-tertiary transition-colors border border-edge"
          >
            Discard
          </button>
        </div>
      )}
    </div>
  );
}
