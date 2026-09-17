import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../../hooks/useApi';
import { agentColor } from '../../lib/agentIdentity';
import { describe, nameOf } from '../../lib/mcpGroups';
import type {
  AccessControlEntry,
  AccessPermission,
  AgentMetadata,
  DefaultPolicy,
  MarketplaceCatalogEntry,
  McpServerInfo,
  McpServerSettings,
  McpToolInfo,
} from '../../types';
import { ConfirmDialog } from '../ui/ConfirmDialog';
import { McpLogsSection } from './McpLogsSection';

const ACTION_FEEDBACK_MS = 2000;
/** How many tools the section shows before "show the rest". */
const TOOLS_SHOWN = 8;

type Section = 'overview' | 'env' | 'tools' | 'access' | 'logs';
const SECTIONS: Section[] = ['overview', 'env', 'tools', 'access', 'logs'];

interface EnvEntry {
  key: string;
  value: string;
}

interface Props {
  server: McpServerInfo;
  entry?: MarketplaceCatalogEntry;
  onBack: () => void;
  onRefresh: () => void;
  // bug-471: these resolve to `true` on success / `undefined` on failure
  // (useAsyncAction swallows the error), so handleAction can gate its "done"
  // feedback on actual success instead of assuming no-throw == success.
  onDelete: (id: string) => Promise<boolean | undefined>;
  onStart: (id: string) => Promise<boolean | undefined>;
  onStop: (id: string) => Promise<boolean | undefined>;
  onRestart: (id: string) => Promise<boolean | undefined>;
}

type Grant = AccessPermission | 'inherit';

function serverGrantOf(entries: AccessControlEntry[], agentId: string, serverId: string): Grant {
  const e = entries.find(
    (x) => x.entry_type === 'server_grant' && x.agent_id === agentId && x.server_id === serverId && !x.tool_name,
  );
  return e?.permission ?? 'inherit';
}

function toolGrantOf(entries: AccessControlEntry[], agentId: string, serverId: string, tool: string): Grant {
  const e = entries.find(
    (x) => x.entry_type === 'tool_grant' && x.agent_id === agentId && x.server_id === serverId && x.tool_name === tool,
  );
  return e?.permission ?? 'inherit';
}

/** Replace one grant (server-wide or per tool) in the pending entries. */
export function withGrant(
  entries: AccessControlEntry[],
  agentId: string,
  serverId: string,
  tool: string | null,
  grant: Grant,
): AccessControlEntry[] {
  const kind = tool ? 'tool_grant' : 'server_grant';
  const kept = entries.filter(
    (e) =>
      !(
        e.entry_type === kind &&
        e.agent_id === agentId &&
        e.server_id === serverId &&
        (tool ? e.tool_name === tool : !e.tool_name)
      ),
  );
  if (grant === 'inherit') return kept;
  return [
    ...kept,
    {
      entry_type: kind,
      agent_id: agentId,
      server_id: serverId,
      ...(tool ? { tool_name: tool } : {}),
      permission: grant,
      granted_by: 'user',
      granted_at: new Date().toISOString(),
    },
  ];
}

function sameEntries(a: AccessControlEntry[], b: AccessControlEntry[]): boolean {
  const key = (e: AccessControlEntry) =>
    `${e.entry_type}|${e.agent_id}|${e.server_id}|${e.tool_name ?? ''}|${e.permission}`;
  const ka = a.map(key).sort();
  const kb = b.map(key).sort();
  return ka.length === kb.length && ka.every((k, i) => k === kb[i]);
}

function envOf(settings: McpServerSettings | null): EnvEntry[] {
  return Object.entries(settings?.env ?? {}).map(([key, value]) => ({ key, value }));
}

function sameEnv(a: EnvEntry[], b: EnvEntry[]): boolean {
  const norm = (xs: EnvEntry[]) =>
    xs
      .filter((e) => e.key.trim())
      .map((e) => `${e.key.trim()}=${e.value}`)
      .sort();
  const na = norm(a);
  const nb = norm(b);
  return na.length === nb.length && na.every((x, i) => x === nb[i]);
}

/**
 * The server's page (docs/gui/samples/07-mcp-server-detail.html): sections on
 * the left, "item + explanation + control" rows on the right, and a save bar
 * below — nothing reaches the kernel until it is pressed.
 */
export function McpServerDetail({ server, entry, onBack, onRefresh, onDelete, onStart, onStop, onRestart }: Props) {
  const api = useApi();
  const { t, i18n } = useTranslation('mcp');
  const [section, setSection] = useState<Section>('overview');
  const paneRef = useRef<HTMLDivElement>(null);
  const sectionRefs = useRef<Record<Section, HTMLHeadingElement | null>>({
    overview: null,
    env: null,
    tools: null,
    access: null,
    logs: null,
  });

  // Loaded state, and the pending copy the person edits.
  const [settings, setSettings] = useState<McpServerSettings | null>(null);
  const [policy, setPolicy] = useState<DefaultPolicy>('opt-in');
  const [env, setEnv] = useState<EnvEntry[]>([]);
  const [newEnv, setNewEnv] = useState<EnvEntry>({ key: '', value: '' });
  const [tools, setTools] = useState<McpToolInfo[]>([]);
  const [allTools, setAllTools] = useState(false);
  const [agents, setAgents] = useState<AgentMetadata[]>([]);
  const [loadedEntries, setLoadedEntries] = useState<AccessControlEntry[]>([]);
  const [entries, setEntries] = useState<AccessControlEntry[]>([]);
  const [openAgents, setOpenAgents] = useState<Set<string>>(new Set());
  const [problem, setProblem] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [actionDone, setActionDone] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const isRunning = server.status === 'Connected';
  const isConnecting =
    server.status === 'Connecting' || server.status === 'Restarting' || server.status === 'Registered';
  const isError = server.status === 'Error';

  const load = useCallback(async () => {
    setProblem(null);
    const [s, tl, access, agentList] = await Promise.all([
      api.getMcpServerSettings(server.id).catch(() => null),
      api.getMcpServerTools(server.id).catch(() => [] as McpToolInfo[]),
      api.getMcpServerAccess(server.id).catch(() => null),
      api.getAgents().catch(() => [] as AgentMetadata[]),
    ]);
    setSettings(s);
    setPolicy(s?.default_policy ?? 'opt-in');
    setEnv(envOf(s));
    setNewEnv({ key: '', value: '' });
    setTools(tl);
    setAgents(agentList);
    setLoadedEntries(access?.entries ?? []);
    setEntries(access?.entries ?? []);
  }, [api, server.id]);

  useEffect(() => {
    load();
  }, [load]);

  const policyChanged = settings !== null && policy !== settings.default_policy;
  const envChanged = settings !== null && !sameEnv(env, envOf(settings));
  const accessChanged = !sameEntries(entries, loadedEntries);
  const dirty = policyChanged || envChanged || accessChanged;

  async function handleSave() {
    if (!dirty || saving) return;
    setSaving(true);
    setProblem(null);
    try {
      if (policyChanged || envChanged) {
        const envObj: Record<string, string> = {};
        for (const e of env) if (e.key.trim()) envObj[e.key.trim()] = e.value;
        await api.updateMcpServerSettings(server.id, { default_policy: policy, env: envObj });
      }
      if (accessChanged) {
        await api.putMcpServerAccess(
          server.id,
          entries.filter((e) => e.entry_type !== 'capability'),
        );
      }
      await load();
      onRefresh();
    } catch (err) {
      setProblem(err instanceof Error ? err.message : t('detail.save_failed'));
    } finally {
      setSaving(false);
    }
  }

  function discard() {
    setPolicy(settings?.default_policy ?? 'opt-in');
    setEnv(envOf(settings));
    setNewEnv({ key: '', value: '' });
    setEntries(loadedEntries);
  }

  async function handleAction(action: string, fn: () => Promise<boolean | undefined>) {
    setActionLoading(action);
    setActionDone(null);
    try {
      // bug-471: only show the success mark when the action actually
      // succeeded — the callbacks swallow errors and resolve `undefined` on
      // failure, so a bare `await` would flip to "done" even on a failed op.
      const ok = await fn();
      if (ok) {
        setActionDone(action);
        setTimeout(() => setActionDone(null), ACTION_FEEDBACK_MS);
      }
      setTimeout(onRefresh, 500);
    } finally {
      setActionLoading(null);
    }
  }

  const jumpTo = (s: Section) => {
    setSection(s);
    sectionRefs.current[s]?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  };

  const toolNames = useMemo(() => (tools.length > 0 ? tools.map((x) => x.name) : server.tools), [tools, server.tools]);

  const origin = (() => {
    const when = server.installed_at
      ? new Intl.DateTimeFormat(i18n.language, { year: 'numeric', month: 'long', day: 'numeric' }).format(
          new Date(server.installed_at * 1000),
        )
      : null;
    if (server.marketplace_id) {
      return t('detail.origin_marketplace', {
        when: when ?? t('detail.origin_when_unknown'),
        version: server.installed_version ?? entry?.installed_version ?? t('detail.origin_version_unknown'),
      });
    }
    return t('detail.origin_manual', { when: when ?? t('detail.origin_when_unknown') });
  })();

  const command = [settings?.command ?? server.command, ...(settings?.args ?? server.args ?? [])].join(' ');
  const description = describe(server, entry);

  const grantSeg = (value: Grant, onChange: (g: Grant) => void, label: string) => (
    <fieldset className="grant" aria-label={label}>
      <button type="button" className={value === 'inherit' ? 'on' : ''} onClick={() => onChange('inherit')}>
        {t('access.inherit')}
      </button>
      <button type="button" className={`allow${value === 'allow' ? ' on' : ''}`} onClick={() => onChange('allow')}>
        {t('access.allow')}
      </button>
      <button type="button" className={`deny${value === 'deny' ? ' on' : ''}`} onClick={() => onChange('deny')}>
        {t('access.deny')}
      </button>
    </fieldset>
  );

  const agentSummary = (agentId: string) => {
    const grant = serverGrantOf(entries, agentId, server.id);
    const overrides = toolNames.filter((tool) => toolGrantOf(entries, agentId, server.id, tool) !== 'inherit');
    if (grant === 'allow' && overrides.length === 0) return t('access.all_tools', { count: toolNames.length });
    if (overrides.length > 0) return t('access.overrides', { count: overrides.length });
    return '';
  };

  return (
    <div className="ws">
      <div className="ws-head">
        <button type="button" className="btn back" onClick={onBack} aria-label={t('detail.back')}>
          ‹ {t('title')}
        </button>
        <h1 className="after-back">{nameOf(server)}</h1>
        <span className="count mono">{server.id}</span>
        {server.mgp_supported && <span className="count">MGP</span>}
        <span className="count">
          <span
            className={`st ${isRunning ? 'ok' : isError ? 'bad' : isConnecting ? 'warn' : ''}`}
            role="status"
            aria-label={
              isRunning
                ? t('status_running')
                : isConnecting
                  ? t('status_connecting')
                  : isError
                    ? t('status_error')
                    : t('status_stopped')
            }
          >
            {isRunning
              ? t('detail.running_with_tools', { count: server.tools.length })
              : isConnecting
                ? t('status_connecting')
                : isError
                  ? t('status_error')
                  : t('status_stopped')}
          </span>
        </span>
        <span className="spacer" />
        <button
          type="button"
          className="btn"
          onClick={() => handleAction('restart', () => onRestart(server.id))}
          disabled={actionLoading !== null}
          aria-label={t('detail.restart')}
        >
          {actionDone === 'restart' ? t('detail.done') : t('detail.restart')}
        </button>
        {isRunning ? (
          <button
            type="button"
            className="btn"
            onClick={() => handleAction('stop', () => onStop(server.id))}
            disabled={actionLoading !== null}
            aria-label={t('detail.stop')}
          >
            {actionDone === 'stop' ? t('detail.done') : t('detail.stop')}
          </button>
        ) : (
          <button
            type="button"
            className="btn"
            onClick={() => handleAction('start', () => onStart(server.id))}
            disabled={actionLoading !== null}
            aria-label={t('detail.start')}
          >
            {actionDone === 'start' ? t('detail.done') : t('detail.start')}
          </button>
        )}
        <button
          type="button"
          className="btn danger"
          onClick={() => setConfirmDelete(true)}
          disabled={actionLoading !== null}
          aria-label={t('detail.delete')}
        >
          {t('detail.delete')}
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
      </div>

      <div className="detail">
        <nav className="rail" aria-label={t('detail.sections')}>
          {SECTIONS.map((s) => (
            <button type="button" key={s} className={section === s ? 'on' : ''} onClick={() => jumpTo(s)}>
              {t(`sections.${s}`)}
            </button>
          ))}
        </nav>
        <div className="pane" ref={paneRef}>
          <h2
            ref={(el) => {
              sectionRefs.current.overview = el;
            }}
          >
            {t('sections.overview')}
          </h2>
          <div className="frow">
            <div className="k">
              {t('detail.description')}
              <small>{t('detail.description_from')}</small>
            </div>
            <div className="v">
              <div className="txt">{description ?? t('detail.no_description')}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">{t('detail.origin')}</div>
            <div className="v">
              <div className="txt">{origin}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('detail.launch')}
              <small>{t('detail.transport_is', { transport: server.transport ?? 'stdio' })}</small>
            </div>
            <div className="v">
              <input className="in mono" value={server.url ?? command} readOnly aria-label={t('detail.launch')} />
              <div className="hint">{t('detail.launch_hint')}</div>
            </div>
          </div>
          <div className="frow">
            <div className="k">
              {t('detail.policy')}
              <small>{t('detail.policy_sub')}</small>
            </div>
            <div className="v">
              <fieldset className="seg" aria-label={t('detail.policy')}>
                <button
                  type="button"
                  className={policy === 'opt-in' ? 'on' : ''}
                  aria-pressed={policy === 'opt-in'}
                  onClick={() => setPolicy('opt-in')}
                >
                  {t('detail.policy_opt_in')}
                </button>
                <button
                  type="button"
                  className={policy === 'opt-out' ? 'on' : ''}
                  aria-pressed={policy === 'opt-out'}
                  onClick={() => setPolicy('opt-out')}
                >
                  {t('detail.policy_opt_out')}
                </button>
              </fieldset>
              <div className="hint">{t('detail.policy_hint')}</div>
            </div>
          </div>

          <h2
            ref={(el) => {
              sectionRefs.current.env = el;
            }}
          >
            {t('sections.env')}
          </h2>
          {env.map((e, i) => (
            <div className="kv" key={`${i}-${e.key}`}>
              <input
                className="in mono"
                value={e.key}
                aria-label={t('detail.env_key')}
                onChange={(ev) => setEnv(env.map((x, j) => (j === i ? { ...x, key: ev.target.value } : x)))}
              />
              <input
                className="in mono"
                type={/KEY|SECRET|TOKEN|PASSWORD/i.test(e.key) ? 'password' : 'text'}
                value={e.value}
                aria-label={t('detail.env_value')}
                onChange={(ev) => setEnv(env.map((x, j) => (j === i ? { ...x, value: ev.target.value } : x)))}
              />
              <button
                type="button"
                className="btn"
                aria-label={t('detail.env_remove')}
                onClick={() => setEnv(env.filter((_, j) => j !== i))}
              >
                ×
              </button>
            </div>
          ))}
          <div className="kv">
            <input
              className="in mono"
              placeholder="KEY"
              aria-label={t('detail.env_new_key')}
              value={newEnv.key}
              onChange={(ev) => setNewEnv({ ...newEnv, key: ev.target.value })}
            />
            <input
              className="in mono"
              placeholder="value"
              aria-label={t('detail.env_new_value')}
              value={newEnv.value}
              onChange={(ev) => setNewEnv({ ...newEnv, value: ev.target.value })}
            />
            <button
              type="button"
              className="btn"
              aria-label={t('detail.env_add')}
              disabled={!newEnv.key.trim()}
              onClick={() => {
                setEnv([...env, { key: newEnv.key.trim(), value: newEnv.value }]);
                setNewEnv({ key: '', value: '' });
              }}
            >
              ＋
            </button>
          </div>
          <div className="hint">{t('detail.env_hint')}</div>

          <h2
            ref={(el) => {
              sectionRefs.current.tools = el;
            }}
          >
            {t('sections.tools')}
            <span className="num">{toolNames.length}</span>
          </h2>
          {(allTools ? tools : tools.slice(0, TOOLS_SHOWN)).map((tool) => (
            <div className="tool" key={tool.name}>
              <span className="nm">{tool.name}</span>
              <span className="d">{tool.description ?? ''}</span>
              <span />
            </div>
          ))}
          {tools.length === 0 && <div className="hint">{t('detail.no_tools')}</div>}
          {!allTools && tools.length > TOOLS_SHOWN && (
            <button type="button" className="btn" onClick={() => setAllTools(true)}>
              {t('detail.show_rest', { count: tools.length - TOOLS_SHOWN })}
            </button>
          )}

          <h2
            ref={(el) => {
              sectionRefs.current.access = el;
            }}
          >
            {t('sections.access')}
          </h2>
          <div className="hint" style={{ margin: '0 0 8px' }}>
            {t('access.hint')}
          </div>
          {agents.map((agent) => {
            const open = openAgents.has(agent.id);
            return (
              <div key={agent.id}>
                <div className="tool">
                  <button
                    type="button"
                    className="who"
                    aria-expanded={open}
                    onClick={() =>
                      setOpenAgents((prev) => {
                        const next = new Set(prev);
                        if (next.has(agent.id)) next.delete(agent.id);
                        else next.add(agent.id);
                        return next;
                      })
                    }
                  >
                    <span className="dot" style={{ background: agentColor(agent) }} />
                    {agent.name}
                  </button>
                  <span className="d">{agentSummary(agent.id)}</span>
                  {grantSeg(
                    serverGrantOf(entries, agent.id, server.id),
                    (g) => setEntries(withGrant(entries, agent.id, server.id, null, g)),
                    t('access.server_grant_for', { name: agent.name }),
                  )}
                </div>
                {open &&
                  toolNames.map((tool) => (
                    <div className="tool sub" key={tool}>
                      <span className="nm">{tool}</span>
                      <span className="d" />
                      {grantSeg(
                        toolGrantOf(entries, agent.id, server.id, tool),
                        (g) => setEntries(withGrant(entries, agent.id, server.id, tool, g)),
                        t('access.tool_grant_for', { name: agent.name, tool }),
                      )}
                    </div>
                  ))}
              </div>
            );
          })}

          <h2
            ref={(el) => {
              sectionRefs.current.logs = el;
            }}
          >
            {t('sections.logs')}
          </h2>
          <McpLogsSection serverId={server.id} />
        </div>
      </div>

      <div className="savebar">
        <span>{problem ? <span className="danger">{problem}</span> : t('detail.unsaved_note')}</span>
        <span className="spacer" />
        <button type="button" className="btn" onClick={discard} disabled={!dirty || saving}>
          {t('detail.discard')}
        </button>
        <button type="button" className="btn pri" onClick={handleSave} disabled={!dirty || saving}>
          {saving ? t('detail.saving') : t('detail.save')}
        </button>
      </div>
    </div>
  );
}
