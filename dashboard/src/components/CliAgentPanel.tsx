import { Activity, RefreshCw, Terminal } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useAsyncAction } from '../hooks/useAsyncAction';
import { useMcpServers } from '../hooks/useMcpServers';
import {
  buildEnvUpdate,
  findHarnessServer,
  type HarnessProbe,
  isMeteredPlan,
  PROBE_TOOL,
  parseProbeResult,
} from '../lib/cliHarness';
import { extractError } from '../lib/errors';
import { displayServerId } from '../lib/format';
import type { EnvVarDef } from '../types';
import { Modal } from './Modal';
import { AlertCard } from './ui/AlertCard';

/**
 * Connect an external CLI agent harness (Claude Code, Codex CLI) as an engine.
 *
 * Two halves, and the split is the point. What the host *has* is read from the
 * connector's own probe — installed, version, how the account is billed — and
 * shown as reported, with anything the probe could not read left as unknown
 * rather than rendered as "no". What the operator *sets* is driven off the
 * catalog's `optional_env_vars`, so this screen never has to know that any
 * particular option exists: a connector that adds one shows it here without a
 * dashboard release, and nothing here branches on a server id or an option name.
 *
 * Edits follow the deferred pattern required for agent config (CLAUDE.md): they
 * accumulate in `edits` and are applied by Save, which also restarts the
 * connector, because a harness option is read at process start.
 */
export function CliAgentPanel({ onClose }: { onClose: () => void }) {
  const api = useApi();
  const { t } = useTranslation('agents');
  const { t: tc } = useTranslation('common');
  const { servers } = useMcpServers();

  const harnessServer = useMemo(() => findHarnessServer(servers), [servers]);
  const serverId = harnessServer?.id;
  const isConnected = harnessServer?.status === 'Connected';

  const [probe, setProbe] = useState<HarnessProbe | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);
  const [isProbing, setIsProbing] = useState(false);

  const [options, setOptions] = useState<EnvVarDef[]>([]);
  const [storedEnv, setStoredEnv] = useState<Record<string, string>>({});
  const [edits, setEdits] = useState<Record<string, string>>({});
  const save = useAsyncAction(t('cli_agent.save_failed'));

  /** Ask the connector what is on the host. Only possible while it is running:
   * the probe is one of its tools, not a kernel route. */
  const runProbe = useCallback(async () => {
    if (!serverId || !isConnected) return;
    setIsProbing(true);
    setProbeError(null);
    try {
      setProbe(parseProbeResult(await api.callMcpTool(PROBE_TOOL, {}, serverId)));
    } catch (e) {
      setProbe(null);
      setProbeError(extractError(e, t('cli_agent.probe_failed')));
    } finally {
      setIsProbing(false);
    }
  }, [api, serverId, isConnected, t]);

  useEffect(() => {
    void runProbe();
  }, [runProbe]);

  // The settable options and their current values come from two different
  // owners: the catalog declares what exists, the server settings hold what is
  // set. Neither is guessed here.
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
        const entry = catalog.servers?.find((s) => s.id === serverId);
        setOptions(entry?.optional_env_vars ?? []);
        setStoredEnv(settings.env ?? {});
      } catch {
        // The form degrades to whatever is already set; the probe above is the
        // part of this screen that has to work.
        if (!cancelled) setOptions([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, serverId]);

  const handleSave = () => {
    if (!serverId) return;
    void save.run(async () => {
      // Every stored key is named, or the kernel's merge drops the ones left out.
      await api.updateMcpServerSettings(serverId, { env: buildEnvUpdate(storedEnv, edits) });
      setStoredEnv((prev) => {
        const next = { ...prev };
        for (const [k, v] of Object.entries(edits)) {
          if (v === '') delete next[k];
          else next[k] = v;
        }
        return next;
      });
      setEdits({});
      // Saving restarts the connector, so what it reports afterwards is what
      // the next run will actually use.
      await runProbe();
    });
  };

  const dirty = Object.keys(edits).length > 0;

  return (
    <Modal title={t('cli_agent.title')} icon={Terminal} size="lg" onClose={onClose}>
      <div className="space-y-4">
        {!harnessServer ? (
          <AlertCard variant="info">{t('cli_agent.not_installed')}</AlertCard>
        ) : (
          <>
            <div className="flex items-center gap-2 text-[11px] font-mono">
              <span className="text-content-tertiary">{t('cli_agent.connector_label')}:</span>
              <span className="text-content-primary font-bold">{displayServerId(harnessServer.id)}</span>
              <span className={isConnected ? 'text-emerald-400' : 'text-content-tertiary'}>
                {isConnected ? t('cli_agent.connected') : t('cli_agent.offline')}
              </span>
              <button
                type="button"
                onClick={() => void runProbe()}
                disabled={isProbing || !isConnected}
                aria-label={t('cli_agent.rescan')}
                title={t('cli_agent.rescan')}
                className="ml-auto p-1.5 rounded-lg border border-edge bg-glass text-content-secondary hover:text-brand hover:border-brand disabled:opacity-30"
              >
                <RefreshCw size={12} className={isProbing ? 'animate-spin' : ''} />
              </button>
            </div>

            {!isConnected && <AlertCard variant="warning">{t('cli_agent.offline_hint')}</AlertCard>}
            {probeError && <AlertCard variant="error">{probeError}</AlertCard>}

            {probe && (
              <div className="space-y-2">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-content-secondary">
                  {t('cli_agent.detected')}
                </h3>
                {probe.harnesses.length === 0 ? (
                  <AlertCard variant="warning">{t('cli_agent.none_found')}</AlertCard>
                ) : (
                  <div className="space-y-1.5">
                    {probe.harnesses.map((h) => {
                      const active = probe.active_harness === h.id;
                      return (
                        <div
                          key={h.id}
                          className="p-2.5 rounded-lg border border-edge bg-glass font-mono text-[11px] space-y-1"
                        >
                          <div className="flex items-center gap-2">
                            <span
                              className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                                h.installed === true
                                  ? 'bg-emerald-400'
                                  : h.installed === false
                                    ? 'bg-neutral-600'
                                    : 'bg-amber-400'
                              }`}
                            />
                            <span className="font-bold text-content-primary">{h.id}</span>
                            {active && <span className="text-brand text-[9px] uppercase">{t('cli_agent.active')}</span>}
                            <span className="ml-auto text-content-tertiary">
                              {h.version || (h.installed === false ? t('cli_agent.not_found') : tc('unknown'))}
                            </span>
                          </div>
                          <div className="flex items-center gap-3 text-[10px] text-content-tertiary">
                            <span>
                              {t('cli_agent.plan_label')}: {h.plan || tc('unknown')}
                            </span>
                            <span>
                              {t('cli_agent.credential_label')}: {h.credential_store || tc('unknown')}
                            </span>
                          </div>
                          {isMeteredPlan(h.plan) && (
                            <p className="text-[10px] text-amber-400">{t('cli_agent.metered_warning')}</p>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
                <p className="text-[10px] text-content-tertiary font-mono">
                  {t('cli_agent.working_directory')}: {probe.working_directory || tc('unknown')}
                </p>
              </div>
            )}

            {options.length > 0 && (
              <div className="space-y-2">
                <h3 className="text-[10px] font-bold uppercase tracking-widest text-content-secondary">
                  {t('cli_agent.options')}
                </h3>
                <div className="space-y-2">
                  {options.map((opt) => (
                    <label key={opt.name} className="block space-y-1">
                      <span className="font-mono text-[10px] text-content-secondary">{opt.name}</span>
                      {opt.description && (
                        <span className="block text-[10px] text-content-tertiary">{opt.description}</span>
                      )}
                      <input
                        type="text"
                        value={edits[opt.name] ?? storedEnv[opt.name] ?? ''}
                        placeholder={opt.default || tc('unknown')}
                        onChange={(e) => setEdits((prev) => ({ ...prev, [opt.name]: e.target.value }))}
                        className="w-full px-2 py-1.5 rounded-lg bg-glass-strong border border-edge text-[11px] font-mono text-content-primary focus:border-brand focus:outline-none"
                      />
                    </label>
                  ))}
                </div>
              </div>
            )}

            {save.error && <AlertCard variant="error">{save.error}</AlertCard>}

            <div className="flex gap-2 pt-1">
              <button
                type="button"
                onClick={() => setEdits({})}
                disabled={!dirty || save.isLoading}
                aria-label={tc('cancel')}
                className="flex-1 py-2 rounded-lg border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all disabled:opacity-50"
              >
                {tc('cancel')}
              </button>
              <button
                type="button"
                onClick={handleSave}
                disabled={!dirty || save.isLoading}
                aria-label={tc('save')}
                className="flex-1 py-2 rounded-lg bg-brand text-white text-xs font-bold hover:bg-brand/90 transition-all disabled:opacity-50 flex items-center justify-center gap-1"
              >
                {save.isLoading && <Activity size={12} className="animate-spin" />}
                {tc('save')}
              </button>
            </div>
            {dirty && <p className="text-[10px] text-content-tertiary font-mono">{t('cli_agent.restart_note')}</p>}
          </>
        )}
      </div>
    </Modal>
  );
}
