import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useConversations } from '../../contexts/ConversationContext';
import { useApi } from '../../hooks/useApi';
import { useMcpServers } from '../../hooks/useMcpServers';
import { useUnreadAgents } from '../../hooks/useUnreadAgents';
import { exportAgent } from '../../lib/agentExport';
import { AgentIcon, agentAccentTriplet } from '../../lib/agentIdentity';
import { firstRoutingAlternative } from '../../lib/agentRouting';
import { dayLabel, timeOfDay } from '../../lib/chatTime';
import { displayServerId } from '../../lib/format';
import { isEngineServer } from '../../lib/serverCategory';
import { latestThinkingText } from '../../lib/thinkingSteps';
import type { AccessControlEntry, AgentMetadata, CronJob } from '../../types';
import { CliAgentPanel } from '../CliAgentPanel';
import { PowerToggleModal } from '../PowerToggleModal';
import '../Workshop.css';
import { CreateAgentModal } from './CreateAgentModal';
import { DeleteAgentModal } from './DeleteAgentModal';

/** The agent the kernel ships with: it cannot be exported or deleted. */
const DEFAULT_AGENT_ID = 'agent.cloto_default';
/** How many cron jobs the detail names before it stops at a count. */
const CRON_NAMED = 2;

interface PendingImport {
  agentData: {
    name: string;
    description: string;
    default_engine: string;
    metadata: Record<string, string>;
  };
  grantedServerIds: string[];
  warnings: string[];
  displayEngineId: string;
}

/** What the detail pane learned about the selected agent, per source. */
interface DetailData {
  memories: number | null;
  grantedServers: string[];
  cron: CronJob[];
}

const EMPTY_DETAIL: DetailData = { memories: null, grantedServers: [], cron: [] };

interface Props {
  agents: AgentMetadata[];
  /** Open the chat with this agent. */
  onSelectAgent: (agent: AgentMetadata) => void;
  onRefresh: () => void;
  /** Which agents are generating a response right now. */
  processing: Set<string>;
}

/**
 * The roster (docs/gui/samples/03-agents.html): everyone who exists on the
 * left, one of them in full on the right.
 *
 * The list is monochrome except the selected row — a list of agents is not a
 * place for five accents (docs/DESIGN_PHILOSOPHY.md §4.2). The selected
 * agent's colour is written onto this screen's own container rather than the
 * document root, so opening the workshop does not recolour the app around it.
 */
export function AgentRoster({ agents, onSelectAgent, onRefresh, processing }: Props) {
  const api = useApi();
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const navigate = useNavigate();
  const { servers } = useMcpServers();
  const { conversations } = useConversations();
  const { unread } = useUnreadAgents();

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [cliAgentOpen, setCliAgentOpen] = useState(false);
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<AgentMetadata | null>(null);
  const [detail, setDetail] = useState<DetailData>(EMPTY_DETAIL);

  // Import (deferred: parse into pendingImport, commit on Save)
  const importRef = useRef<HTMLInputElement>(null);
  const [pendingImport, setPendingImport] = useState<PendingImport | null>(null);
  const [importWarnings, setImportWarnings] = useState<string[]>([]);
  const [isImporting, setIsImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  const selected = agents.find((a) => a.id === selectedId) ?? null;
  const engines = useMemo(() => servers.filter(isEngineServer), [servers]);

  // The detail's three reads. Each is allowed to fail on its own: a memory
  // server that is down should not take the cron line down with it.
  useEffect(() => {
    if (!selected) {
      setDetail(EMPTY_DETAIL);
      return;
    }
    let cancelled = false;
    const agentId = selected.id;
    void (async () => {
      const [memories, access, cron] = await Promise.all([
        api
          .getMemories(agentId)
          .then((r) => r.memories.length)
          .catch(() => null),
        api
          .getAgentAccess(agentId)
          .then((r) => r.entries)
          .catch(() => [] as AccessControlEntry[]),
        api
          .listCronJobs(agentId)
          .then((r) => r.jobs)
          .catch(() => [] as CronJob[]),
      ]);
      if (cancelled) return;
      setDetail({
        memories,
        grantedServers: access
          .filter((e) => e.entry_type === 'server_grant' && e.permission === 'allow')
          .map((e) => e.server_id),
        cron,
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [api, selected]);

  // Opening the chat is what clears the row's mark; the agent route does that
  // for every way in (hooks/useUnreadAgents.ts, useReadOnOpen).
  const openChat = onSelectAgent;

  const openSettings = (agent: AgentMetadata, section?: string) => {
    navigate(`/agents/${encodeURIComponent(agent.id)}/settings${section ? `?section=${section}` : ''}`);
  };

  // Parse the import file and build a pending preview. No API calls yet:
  // createAgent + putAgentMcpAccess are committed on Save.
  const handleImport = async (file: File) => {
    try {
      const text = await file.text();
      const data = JSON.parse(text);

      if (!data.cloto_agent_export || !data.agent?.name) {
        setImportError(t('import_invalid'));
        return;
      }

      const agentData = data.agent;
      const meta: Record<string, string> = {
        ...(agentData.metadata || {}),
        agent_type: agentData.metadata?.agent_type || 'ai',
      };

      const warnings: string[] = [];

      let engineId = agentData.default_engine_id || '';
      if (engineId && !engines.some((s) => s.id === engineId)) {
        warnings.push(t('import_engine_missing', { engine: engineId }));
        engineId = '';
      }

      // The pending state only carries server_ids that exist — surviving
      // warnings are shown in the preview so the person can decide before
      // committing.
      const knownServerIds = new Set(servers.map((s) => s.id));
      const grantedServerIds: string[] = [];
      if (Array.isArray(data.mcp_access)) {
        for (const access of data.mcp_access) {
          if (!knownServerIds.has(access.server_id)) {
            warnings.push(t('import_server_skipped', { server: access.server_id }));
            continue;
          }
          grantedServerIds.push(access.server_id);
        }
      }

      setPendingImport({
        agentData: {
          name: agentData.name,
          description: agentData.description || '',
          default_engine: engineId,
          metadata: meta,
        },
        grantedServerIds,
        warnings,
        displayEngineId: engineId,
      });
      setImportError(null);
      setImportWarnings([]);
    } catch (e) {
      setImportError(t('import_error', { error: e instanceof Error ? e.message : 'Unknown error' }));
    }
  };

  const handleSaveImport = async () => {
    if (!pendingImport || isImporting) return;
    setIsImporting(true);
    setImportError(null);
    try {
      await api.createAgent({
        name: pendingImport.agentData.name,
        description: pendingImport.agentData.description,
        default_engine: pendingImport.agentData.default_engine,
        metadata: pendingImport.agentData.metadata,
      });

      const finalWarnings = [...pendingImport.warnings];

      if (pendingImport.grantedServerIds.length > 0) {
        const allAgents = await api.getAgents();
        const created = allAgents.find((a: AgentMetadata) => a.name === pendingImport.agentData.name);
        if (created) {
          try {
            await api.putAgentMcpAccess(created.id, pendingImport.grantedServerIds);
          } catch {
            // The agent exists but its grants did not land. Name each one so
            // the person knows what to grant by hand.
            for (const serverId of pendingImport.grantedServerIds) {
              finalWarnings.push(t('import_server_skipped', { server: serverId }));
            }
          }
        }
      }

      onRefresh();
      setImportWarnings(finalWarnings);
      setPendingImport(null);
    } catch (e) {
      setImportError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setIsImporting(false);
    }
  };

  /** Newest conversation activity for an agent, or null when there is none. */
  const lastTalked = (agentId: string): number | null => {
    let newest: number | null = null;
    for (const c of conversations) {
      if (c.agent_id !== agentId) continue;
      if (newest === null || c.updated_at > newest) newest = c.updated_at;
    }
    return newest;
  };

  const lastTalkedLabel = (agentId: string): string => {
    const ts = lastTalked(agentId);
    if (ts === null) return '';
    const label = dayLabel(ts);
    if (label.kind === 'today') return timeOfDay(ts);
    if (label.kind === 'yesterday') return t('console.day_yesterday');
    return t('console.date_md', { month: label.month, day: label.day });
  };

  const stateLine = (agent: AgentMetadata): { text: string; live: boolean } => {
    if (!agent.enabled) return { text: t('roster.state_stopped'), live: false };
    if (processing.has(agent.id)) {
      const step = latestThinkingText(agent.id);
      return {
        text: step ? t('roster.state_responding_step', { step }) : t('roster.state_responding'),
        live: true,
      };
    }
    return { text: t('roster.state_idle'), live: false };
  };

  const running = agents.filter((a) => processing.has(a.id));
  const idle = agents.filter((a) => !processing.has(a.id));

  const row = (agent: AgentMetadata) => {
    const state = stateLine(agent);
    return (
      <button
        type="button"
        key={agent.id}
        className={`r${agent.id === selectedId ? ' on' : ''}${agent.enabled ? '' : ' off'}`}
        aria-pressed={agent.id === selectedId}
        onClick={() => setSelectedId(agent.id)}
        onDoubleClick={() => openChat(agent)}
      >
        <span className="f">
          <AgentIcon agent={agent} size={agent.metadata?.has_avatar === 'true' ? 34 : 18} />
        </span>
        <span className="who">
          <span className="nm">
            {agent.name}
            {unread.has(agent.id) && <span className="unread" role="img" aria-label={t('roster.unread')} />}
          </span>
          <span className={`st${state.live ? ' live' : ''}`}>{state.text}</span>
        </span>
        <span className="last num">{lastTalkedLabel(agent.id)}</span>
      </button>
    );
  };

  const engineLine = (agent: AgentMetadata): string => {
    const engine = agent.default_engine_id ? displayServerId(agent.default_engine_id) : t('no_engine');
    const other = firstRoutingAlternative(agent.metadata?.engine_routing);
    return other ? t('roster.engine_with_routing', { engine, other: displayServerId(other) }) : engine;
  };

  const memoryLine = (agent: AgentMetadata): string => {
    const server = agent.metadata?.preferred_memory;
    if (!server) return t('roster.memory_none');
    const name = displayServerId(server);
    return detail.memories === null ? name : t('roster.memory_with_count', { name, count: detail.memories });
  };

  const toolsLine = (): string => {
    const granted = servers.filter((s) => detail.grantedServers.includes(s.id));
    const tools = granted.reduce((sum, s) => sum + s.tools.length, 0);
    return t('roster.tools_summary', { servers: granted.length, tools });
  };

  const cronLine = (): string => {
    if (detail.cron.length === 0) return t('roster.cron_none');
    const named = detail.cron
      .slice(0, CRON_NAMED)
      .map((job) =>
        job.next_run_at > 0 && job.next_run_at < Number.MAX_SAFE_INTEGER
          ? `${timeOfDay(job.next_run_at)} ${job.name}`
          : job.name,
      )
      .join(t('roster.cron_sep'));
    return t('roster.cron_summary', { count: detail.cron.length, named });
  };

  const avatarLine = (agent: AgentMetadata): string => {
    if (agent.metadata?.has_avatar === 'true') return t('roster.avatar_image');
    if (agent.metadata?.has_vrm === 'true') return t('roster.avatar_vrm');
    return t('roster.avatar_none');
  };

  return (
    <div className="ws">
      <div className="ws-head">
        <h1>{t('title')}</h1>
        <span className="count">{t('roster.summary', { count: agents.length, running: running.length })}</span>
        <span className="spacer" />
        <input
          ref={importRef}
          type="file"
          accept=".json"
          style={{ display: 'none' }}
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) void handleImport(f);
            e.target.value = '';
          }}
        />
        <button type="button" className="btn" onClick={() => setCliAgentOpen(true)}>
          {t('cli_agent.open')}
        </button>
        <button
          type="button"
          className="btn"
          disabled={pendingImport !== null || isImporting}
          onClick={() => importRef.current?.click()}
        >
          {t('import_config')}
        </button>
        <button type="button" className="btn pri" onClick={() => setCreateOpen(true)}>
          {t('create_agent')}
        </button>
      </div>

      {/* The import, before it happens. Nothing has reached the kernel yet. */}
      {pendingImport && (
        <div className="import-preview">
          <div className="hint" style={{ marginTop: 0 }}>
            {t('import_preview.hint')}
          </div>
          <dl className="agent-dl">
            <dt>{t('form.name')}</dt>
            <dd>{pendingImport.agentData.name}</dd>
            <dt>{t('import_preview.engine_label')}</dt>
            <dd>
              {pendingImport.displayEngineId
                ? displayServerId(pendingImport.displayEngineId)
                : t('import_preview.none')}
            </dd>
            <dt>{t('import_preview.access_label')}</dt>
            <dd>{t('import_preview.access_count', { count: pendingImport.grantedServerIds.length })}</dd>
          </dl>
          {pendingImport.warnings.map((w) => (
            <div className="dev warn" key={w}>
              {w}
            </div>
          ))}
          {importError && <div className="dev">{importError}</div>}
          <div className="agent-acts">
            <button type="button" className="btn" disabled={isImporting} onClick={() => setPendingImport(null)}>
              {tc('cancel')}
            </button>
            <button type="button" className="btn pri" disabled={isImporting} onClick={handleSaveImport}>
              {t('import_preview.save')}
            </button>
          </div>
        </div>
      )}

      {importWarnings.length > 0 && (
        <div className="import-preview">
          {importWarnings.map((w) => (
            <div className="dev warn" key={w}>
              {w}
            </div>
          ))}
          <button type="button" className="btn" onClick={() => setImportWarnings([])}>
            {tc('close')}
          </button>
        </div>
      )}
      {importError && !pendingImport && <div className="problem">{importError}</div>}

      {agents.length === 0 ? (
        <div className="empty">{t('no_agents')}</div>
      ) : (
        <div
          className="ws-body roster-split"
          style={selected ? ({ '--agent': agentAccentTriplet(selected) } as React.CSSProperties) : undefined}
        >
          <div className="roster">
            {running.length > 0 && <div className="grp">{t('roster.group_running')}</div>}
            {running.map(row)}
            {idle.length > 0 && <div className="grp">{t('roster.group_idle')}</div>}
            {idle.map(row)}
          </div>

          {selected ? (
            <aside className="agent-detail">
              <div className="who">
                <span className="face-lg mid">
                  <AgentIcon agent={selected} size={selected.metadata?.has_avatar === 'true' ? 40 : 22} />
                </span>
                <span>
                  <span className="nm" style={{ display: 'block' }}>
                    {selected.name}
                  </span>
                  <span className="id">{selected.id}</span>
                </span>
                <span className="spacer" style={{ flex: 1 }} />
                <button type="button" className="btn" onClick={() => openChat(selected)}>
                  {t('chat')}
                </button>
              </div>

              <dl className="agent-dl">
                <dt>{t('roster.role')}</dt>
                <dd className="sub">{selected.description || t('roster.role_none')}</dd>
                <dt>{t('roster.engine')}</dt>
                <dd>{engineLine(selected)}</dd>
                <dt>{t('roster.memory')}</dt>
                <dd>{memoryLine(selected)}</dd>
                <dt>{t('roster.tools')}</dt>
                <dd>{toolsLine()}</dd>
                <dt>{t('roster.cron')}</dt>
                <dd>{cronLine()}</dd>
                <dt>{t('roster.avatar')}</dt>
                <dd>{avatarLine(selected)}</dd>
                <dt>{t('roster.colour')}</dt>
                <dd>
                  <span className="crow">
                    <span className="dot-sm" style={{ background: `hsl(${agentAccentTriplet(selected)})` }} />
                    {`hsl(${agentAccentTriplet(selected)})`}
                  </span>
                </dd>
              </dl>

              <div className="agent-acts">
                <button type="button" className="btn" onClick={() => openSettings(selected)}>
                  {t('roster.open_settings')}
                </button>
                <button type="button" className="btn" onClick={() => openSettings(selected, 'tools')}>
                  {t('roster.tool_permissions')}
                </button>
                {selected.id !== DEFAULT_AGENT_ID && (
                  <button type="button" className="btn" onClick={() => void exportAgent(api, selected)}>
                    {t('export_config')}
                  </button>
                )}
                <span className="spacer" />
                <button type="button" className="btn danger" onClick={() => setPowerTarget(selected)}>
                  {selected.enabled ? t('roster.stop') : t('roster.start')}
                </button>
                {selected.id !== DEFAULT_AGENT_ID && (
                  <button type="button" className="btn danger" onClick={() => setDeleteTarget(selected)}>
                    {tc('delete')}
                  </button>
                )}
              </div>

              <p className="agent-note">{t('roster.note')}</p>
            </aside>
          ) : (
            <aside className="agent-detail">
              <p className="agent-note" style={{ marginTop: 0 }}>
                {t('roster.pick_someone')}
              </p>
            </aside>
          )}
        </div>
      )}

      {createOpen && (
        <CreateAgentModal
          onClose={() => setCreateOpen(false)}
          onCreated={() => {
            setCreateOpen(false);
            onRefresh();
          }}
        />
      )}
      {cliAgentOpen && (
        <CliAgentPanel agents={agents} onAgentsChanged={onRefresh} onClose={() => setCliAgentOpen(false)} />
      )}
      {powerTarget && (
        <PowerToggleModal agent={powerTarget} onClose={() => setPowerTarget(null)} onSuccess={onRefresh} />
      )}
      {deleteTarget && (
        <DeleteAgentModal
          agent={deleteTarget}
          onClose={() => setDeleteTarget(null)}
          onDeleted={() => {
            setDeleteTarget(null);
            setSelectedId(null);
            onRefresh();
          }}
        />
      )}
    </div>
  );
}
