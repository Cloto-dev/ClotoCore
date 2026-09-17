import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Select } from '../components/ui/Select';
import { useAgentContext } from '../contexts/AgentContext';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import {
  type BoundAgent,
  boundAgents,
  buildAgentMetadataUpdate,
  buildEnvUpdate,
  findHarnessServer,
  type HarnessEntry,
  type HarnessProbe,
  harnessInitials,
  harnessName,
  isMeteredPlan,
  PROBE_TOOL,
  parseProbeResult,
} from '../lib/cliHarness';
import { extractError } from '../lib/errors';
import { displayServerId } from '../lib/format';
import type { EnvVarDef } from '../types';
import '../components/Workshop.css';
import './CliAgentsPage.css';

/**
 * CLI agents (docs/gui/samples/09-cli-agents.html): the harnesses this machine
 * has on the left, and on the right the chosen one — its state, the connector's
 * options, and the agents that run on it.
 *
 * Nothing on this page knows a harness, an option or a per-agent field by name.
 * The harnesses are what the connector's probe reports, the options are what
 * its catalog entry declares, and the per-agent fields are the schema the probe
 * carries, so a connector that learns a new harness or option shows it here
 * without a dashboard release.
 *
 * Edits are held until Save (the deferred pattern agent config requires).
 * Everything that can refuse — the agents still existing, their saved settings
 * still being readable, the server settings being readable — is asked before
 * anything is written.
 */
export function CliAgentsPage() {
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  // Read through a ref where an effect needs a message: `t` changes identity
  // with the language, and a scan should not re-run for that.
  const tRef = useRef(t);
  tRef.current = t;
  const navigate = useNavigate();
  const api = useApi();
  const { agents, refetchAgents } = useAgentContext();
  const { servers } = useMcpServers();

  const harnessServer = useMemo(() => findHarnessServer(servers), [servers]);
  const serverId = harnessServer?.id;
  const isConnected = harnessServer?.status === 'Connected';

  const [probe, setProbe] = useState<HarnessProbe | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [isProbing, setIsProbing] = useState(false);
  const [probedAt, setProbedAt] = useState<Date | null>(null);

  const [options, setOptions] = useState<EnvVarDef[]>([]);
  const [storedEnv, setStoredEnv] = useState<Record<string, string>>({});
  const [optionsError, setOptionsError] = useState<string | null>(null);

  const [selected, setSelected] = useState<string | null>(null);
  const [openAgent, setOpenAgent] = useState<string | null>(null);
  const [connEdits, setConnEdits] = useState<Record<string, string>>({});
  const [agentEdits, setAgentEdits] = useState<Record<string, Record<string, string>>>({});
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  /** Ask the connector what is on the host. Only possible while it runs: the
   * probe is one of its tools, not a kernel route. */
  const runProbe = useCallback(async () => {
    if (!serverId || !isConnected) return;
    setIsProbing(true);
    setProbeError(null);
    try {
      setProbe(parseProbeResult(await api.callMcpTool(PROBE_TOOL, {}, serverId)));
      setProbedAt(new Date());
    } catch (e) {
      setProbe(null);
      setProbeError(extractError(e, tRef.current('cli_agent.probe_failed')));
    } finally {
      setIsProbing(false);
    }
  }, [api, serverId, isConnected]);

  useEffect(() => {
    void runProbe();
  }, [runProbe]);

  // What can be set comes from the catalog, what is set from the server
  // settings. A failed read is said, not drawn as "no options": an empty form
  // would claim the connector has nothing to configure.
  useEffect(() => {
    if (!serverId) return;
    let cancelled = false;
    void (async () => {
      try {
        const [catalog, settings] = await Promise.all([
          api.getMarketplaceCatalog(),
          api.getMcpServerSettings(serverId),
        ]);
        if (cancelled) return;
        setOptions(catalog.servers?.find((s) => s.id === serverId)?.optional_env_vars ?? []);
        setStoredEnv(settings.env ?? {});
        setOptionsError(null);
      } catch (e) {
        if (cancelled) return;
        setOptions([]);
        setOptionsError(extractError(e, tRef.current('cli_agent.options_failed')));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, serverId]);

  const harnesses = useMemo(() => probe?.harnesses ?? [], [probe]);
  const schema = probe?.agent_config;
  const bound = useMemo(
    () => (probe && serverId ? boundAgents(agents, serverId, probe) : []),
    [agents, serverId, probe],
  );

  // The selection follows the probe: what the connector would run, else the
  // first one installed, else the first one reported.
  useEffect(() => {
    if (harnesses.some((h) => h.id === selected)) return;
    const pick =
      harnesses.find((h) => h.id === probe?.active_harness) ??
      harnesses.find((h) => h.installed === true) ??
      harnesses[0];
    setSelected(pick?.id ?? null);
  }, [harnesses, probe, selected]);

  const current = harnesses.find((h) => h.id === selected) ?? null;
  const knownIds = new Set(harnesses.map((h) => h.id));
  const onCurrent = bound.filter((b) => current && b.harness === current.id);
  // Agents whose run would not start on any harness this machine has: shown
  // whichever harness is selected, because they belong to none of them.
  const stranded = bound.filter((b) => b.harness === null || !knownIds.has(b.harness));

  const agentEditIds = Object.keys(agentEdits).filter((id) => Object.keys(agentEdits[id]).length > 0);
  const connDirty = Object.keys(connEdits).length > 0;
  const dirty = connDirty || agentEditIds.length > 0;

  const discard = () => {
    setConnEdits({});
    setAgentEdits({});
    setSaveError(null);
  };

  const handleSave = async () => {
    if (!serverId) return;
    setSaving(true);
    setSaveError(null);
    try {
      // --- asked first: anything that can refuse, before anything is written.
      const key = schema?.metadata_key;
      const rows = agentEditIds.length > 0 ? await api.getAgents() : [];
      const agentWrites: { id: string; metadata: Record<string, string> }[] = [];
      for (const id of agentEditIds) {
        if (!key) throw new Error(t('cli_agent.binding_schema_missing'));
        const row = rows.find((a) => a.id === id);
        if (!row) throw new Error(t('cli_agent.agent_missing'));
        // Built from the row as it is now; throws if its saved settings have
        // become unreadable since the page read them.
        agentWrites.push({ id, metadata: buildAgentMetadataUpdate(row.metadata ?? {}, key, agentEdits[id]) });
      }
      // Every stored key has to be named on save or the kernel drops it, so the
      // names come from the settings as they are now, not as the page first saw them.
      const freshEnv = connDirty ? ((await api.getMcpServerSettings(serverId)).env ?? {}) : null;

      // --- then written.
      for (const write of agentWrites) {
        await api.updateAgent(write.id, { metadata: write.metadata });
        setAgentEdits((prev) => {
          const next = { ...prev };
          delete next[write.id];
          return next;
        });
      }
      if (agentWrites.length > 0) await refetchAgents();

      if (freshEnv) {
        await api.updateMcpServerSettings(serverId, { env: buildEnvUpdate(freshEnv, connEdits) });
        const next = { ...freshEnv };
        for (const [k, v] of Object.entries(connEdits)) {
          if (v === '') delete next[k];
          else next[k] = v;
        }
        setStoredEnv(next);
        setConnEdits({});
        // Options are read when the connector starts; what it reports after the
        // restart is the confirmation.
        await runProbe();
      }
    } catch (e) {
      setSaveError(extractError(e, t('cli_agent.save_failed')));
    } finally {
      setSaving(false);
    }
  };

  const timeFormat = useMemo(() => new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' }), []);

  const storeLabel = (h: HarnessEntry) =>
    h.credential_store === 'keychain'
      ? t('cli_agent.store_keychain')
      : h.credential_store === 'file'
        ? t('cli_agent.store_file')
        : '—';

  const planText = (h: HarnessEntry) => {
    if (isMeteredPlan(h.plan)) return t('cli_agent.plan_api_key');
    if (h.plan === 'subscription') {
      const tier = h.organizationType || h.organizationRateLimitTier;
      return tier ? t('cli_agent.plan_subscription_tier', { tier }) : t('cli_agent.plan_subscription');
    }
    return t('cli_agent.plan_unknown');
  };

  const usedBy = (id: string) => bound.filter((b) => b.harness === id).length;

  const bindingSummary = (b: BoundAgent) => {
    if (b.bindingError) return <span className="danger">{t('cli_agent.binding_invalid')}</span>;
    if (b.harness === null) return <span className="danger">{t('cli_agent.no_harness_decided')}</span>;
    if (!knownIds.has(b.harness)) {
      return <span className="danger">{t('cli_agent.binding_unknown_harness', { harness: b.harness })}</span>;
    }
    const parts = (schema?.fields ?? [])
      .filter((f) => f.key !== 'harness' || !current || b.binding[f.key] !== current.id)
      .filter((f) => b.binding[f.key])
      .map((f) => `${f.label} ${b.binding[f.key]}`);
    const text = parts.length > 0 ? parts.join(t('cli_agent.list_sep')) : t('cli_agent.uses_defaults');
    return agentEdits[b.agent.id] && Object.keys(agentEdits[b.agent.id]).length > 0
      ? t('cli_agent.unsaved', { text })
      : text;
  };

  const bindingRow = (b: BoundAgent) => {
    const open = openAgent === b.agent.id;
    const editable = Boolean(schema) && !b.bindingError;
    return (
      <div key={b.agent.id} data-testid={`binding-${b.agent.id}`}>
        <div className="tool bind">
          <span className="who">
            <span className="dot" />
            {b.agent.name}
          </span>
          <span className="d">{bindingSummary(b)}</span>
          {editable && (
            <button
              type="button"
              className="btn"
              aria-expanded={open}
              onClick={() => setOpenAgent(open ? null : b.agent.id)}
            >
              {open ? t('cli_agent.close_binding') : t('cli_agent.edit_binding')}
            </button>
          )}
        </div>
        {open &&
          editable &&
          schema?.fields.map((field) => {
            const value = agentEdits[b.agent.id]?.[field.key] ?? b.binding[field.key] ?? '';
            const set = (v: string) =>
              setAgentEdits((prev) => ({ ...prev, [b.agent.id]: { ...prev[b.agent.id], [field.key]: v } }));
            const label = `${b.agent.name} ${field.label}`;
            return (
              <div className="frow bind-field" key={field.key}>
                <div className="k">
                  {field.label}
                  {field.description && <small>{field.description}</small>}
                </div>
                <div className="v">
                  {field.input === 'select' ? (
                    <Select
                      label={label}
                      value={value}
                      onChange={set}
                      options={[
                        { value: '', label: t('cli_agent.inherit_default') },
                        ...(field.options ?? []).map((o) => ({ value: o, label: o })),
                      ]}
                    />
                  ) : (
                    <input
                      className="in mono"
                      aria-label={label}
                      value={value}
                      placeholder={field.default || t('cli_agent.inherit_default')}
                      onChange={(e) => set(e.target.value)}
                    />
                  )}
                </div>
              </div>
            );
          })}
      </div>
    );
  };

  return (
    <div className="ws cli">
      <div className="ws-head">
        <button type="button" className="btn back" onClick={() => navigate('/')}>
          ‹ {t('title')}
        </button>
        <h1 className="after-back">{t('cli_agent.page_title')}</h1>
        <span className="count">{t('cli_agent.page_lead')}</span>
        <span className="spacer" />
        {harnessServer && (
          <button type="button" className="btn" onClick={() => void runProbe()} disabled={!isConnected || isProbing}>
            {t('cli_agent.rescan')}
          </button>
        )}
      </div>

      {!harnessServer ? (
        <div className="empty" data-testid="cli-no-connector">
          <p>{t('cli_agent.not_installed')}</p>
          <button type="button" className="btn" onClick={() => navigate('/mcp-servers')}>
            {t('cli_agent.open_mcp')}
          </button>
        </div>
      ) : (
        <>
          <div className="detail">
            <div className="cli-list">
              <p className="hint lead">
                {t('cli_agent.connector_label')} <span className="mono">{displayServerId(harnessServer.id)}</span>{' '}
                <span className={isConnected ? 'ok' : 'danger'}>
                  {isConnected ? t('cli_agent.connected') : t('cli_agent.offline')}
                </span>
                {probedAt && t('cli_agent.scanned_at', { time: timeFormat.format(probedAt) })}
              </p>
              {!isConnected && <p className="hint">{t('cli_agent.offline_hint')}</p>}
              {probeError && <p className="hint danger">{probeError}</p>}
              {isConnected && !probe && !probeError && <p className="hint">{t('cli_agent.scanning')}</p>}
              {/* The connector lists every harness it knows, installed or not, so
                  "none found" means none of them is installed — not an empty list. */}
              {probe && !harnesses.some((h) => h.installed === true) && (
                <p className="hint">{t('cli_agent.none_found')}</p>
              )}
              {harnesses.map((h) => {
                const on = h.id === selected;
                const n = usedBy(h.id);
                return (
                  <button
                    type="button"
                    key={h.id}
                    className={`harness${on ? ' on' : ''}`}
                    aria-pressed={on}
                    onClick={() => setSelected(h.id)}
                  >
                    <span className="ic" aria-hidden="true">
                      {harnessInitials(h)}
                    </span>
                    <span>
                      <span className="nm">{harnessName(h)}</span>
                      <span className="hst">
                        {h.installed === true ? (
                          <>
                            <span className="ok">{t('cli_agent.usable')}</span>
                            {t('cli_agent.harness_line', {
                              version: h.version || tc('unknown'),
                              plan: planText(h),
                              used: n > 0 ? t('cli_agent.used_by', { count: n }) : t('cli_agent.unused'),
                            })}
                          </>
                        ) : h.installed === false ? (
                          t('cli_agent.not_found')
                        ) : (
                          t('cli_agent.state_unknown')
                        )}
                      </span>
                    </span>
                    <span className="r">{storeLabel(h)}</span>
                  </button>
                );
              })}
              {probe && harnesses.length > 0 && <p className="hint foot">{t('cli_agent.metered_note')}</p>}
            </div>

            <div className="pane">
              {current && (
                <>
                  <h2>{harnessName(current)}</h2>
                  <div className="frow">
                    <div className="k">{t('cli_agent.state_label')}</div>
                    <div className="v">
                      <div className="txt" data-testid="cli-state">
                        {current.installed === true ? (
                          <>
                            {t('cli_agent.state_usable')}
                            <span className="mono">{current.binary ?? current.id}</span>{' '}
                            {t('cli_agent.state_usable_detail', {
                              version: current.version || tc('unknown'),
                              store: storeLabel(current),
                              plan: planText(current),
                            })}
                          </>
                        ) : current.installed === false ? (
                          <>
                            {t('cli_agent.state_missing')} <span className="mono">{current.binary ?? current.id}</span>
                          </>
                        ) : (
                          t('cli_agent.state_unknown')
                        )}
                      </div>
                      {isMeteredPlan(current.plan) && <div className="hint warn">{t('cli_agent.metered_warning')}</div>}
                    </div>
                  </div>
                  <div className="frow">
                    <div className="k">
                      {t('cli_agent.working_directory')}
                      <small>{t('cli_agent.working_directory_sub')}</small>
                    </div>
                    <div className="v">
                      <div className="txt mono">{probe?.working_directory || tc('unknown')}</div>
                      <div className="hint">{t('cli_agent.working_directory_hint')}</div>
                    </div>
                  </div>
                </>
              )}

              <h2>{t('cli_agent.options')}</h2>
              <div className="hint">{t('cli_agent.restart_note')}</div>
              {optionsError && <div className="hint danger">{optionsError}</div>}
              {!optionsError && options.length === 0 && <div className="hint">{t('cli_agent.no_options')}</div>}
              {options.map((opt) => (
                <div className="frow" key={opt.name}>
                  <div className="k">
                    <span className="mono">{opt.name}</span>
                    {opt.description && <small>{opt.description}</small>}
                  </div>
                  <div className="v">
                    <input
                      className="in mono"
                      aria-label={opt.name}
                      value={connEdits[opt.name] ?? storedEnv[opt.name] ?? ''}
                      placeholder={opt.default || t('cli_agent.option_unset')}
                      onChange={(e) => setConnEdits((prev) => ({ ...prev, [opt.name]: e.target.value }))}
                    />
                  </div>
                </div>
              ))}

              <h2>{t('cli_agent.agent_binding')}</h2>
              <div className="hint">{t('cli_agent.binding_note')}</div>
              {!probe ? (
                <div className="hint">{t('cli_agent.binding_needs_probe')}</div>
              ) : !schema ? (
                <div className="hint">{t('cli_agent.binding_schema_missing')}</div>
              ) : onCurrent.length === 0 && stranded.length === 0 ? (
                <div className="hint">
                  {current
                    ? t('cli_agent.no_agents_on_harness', { harness: harnessName(current) })
                    : t('cli_agent.no_bound_agents')}
                </div>
              ) : null}
              {probe && schema && [...onCurrent, ...stranded].map(bindingRow)}

              <h2>{t('cli_agent.run_title')}</h2>
              <div className="hint">{t('cli_agent.run_explain')}</div>
            </div>
          </div>

          <div className="savebar">
            <span>{saveError ? <span className="danger">{saveError}</span> : t('cli_agent.savebar_note')}</span>
            <span className="spacer" />
            <button type="button" className="btn" onClick={discard} disabled={!dirty || saving}>
              {t('settings.discard')}
            </button>
            <button type="button" className="btn pri" onClick={() => void handleSave()} disabled={!dirty || saving}>
              {saving ? t('settings.saving') : tc('save')}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
