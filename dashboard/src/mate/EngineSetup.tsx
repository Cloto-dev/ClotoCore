import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import type { AgentMetadata, MarketplaceCatalogEntry } from '../types';
import {
  type EnginePlan,
  EngineSetupError,
  type SetupStep,
  type StepState,
  setUpEngine,
  stepsFor,
  watchInstallOver,
} from './setupEngine';

interface Engine {
  id: string;
  name: string;
  needsKey: boolean;
  installed: boolean;
  hasKey: boolean;
}

type Catalog = { engines: Engine[] } | 'loading' | { failed: string };
type Phase = 'choosing' | 'running' | 'done' | 'failed';

/**
 * Choosing and setting up the partner's LLM engine inside the settings panel. Nothing is sent
 * until "Use this engine"; Cancel leaves without a request. The engines are the catalog's
 * entries that reach their model through the kernel's LLM proxy, so a new engine on the hub
 * appears here without a change to this screen.
 */
export function EngineSetup({
  agent,
  onSet,
  onClose,
  onDetails,
}: {
  agent: AgentMetadata;
  /** The engine was set: the caller reads the partner and the servers again. */
  onSet: () => void;
  /** Back to the settings panel, from Cancel or after the engine was set. */
  onClose: () => void;
  onDetails: () => void;
}) {
  const { t } = useTranslation('agents');
  const api = useApi();
  const { servers } = useMcpServers();
  const [catalog, setCatalog] = useState<Catalog>('loading');
  const [selected, setSelected] = useState<string | null>(null);
  const [key, setKey] = useState('');
  const [phase, setPhase] = useState<Phase>('choosing');
  const [steps, setSteps] = useState<{ step: SetupStep; state: StepState | 'waiting' }[]>([]);
  const [failure, setFailure] = useState<EngineSetupError | null>(null);
  // Set on every mount, not only at creation: StrictMode runs the cleanup once before the real
  // mount, and a flag left false there would drop every answer that arrives afterwards.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const load = useCallback(() => {
    setCatalog('loading');
    Promise.all([api.getMarketplaceCatalog(), api.listLlmProviders().catch(() => ({ providers: [] }))])
      .then(([{ servers: entries }, { providers }]) => {
        if (!mounted.current) return;
        const withKey = new Set(providers.filter((p) => p.has_key).map((p) => p.id));
        const engines = entries
          .filter((e): e is MarketplaceCatalogEntry & { llm_provider: { needs_api_key: boolean } } => !!e.llm_provider)
          .map((e) => ({
            id: e.id,
            name: e.name,
            needsKey: e.llm_provider.needs_api_key,
            installed: e.installed,
            hasKey: withKey.has(e.id),
          }));
        setCatalog({ engines });
        setSelected((current) => {
          if (current && engines.some((e) => e.id === current)) return current;
          return engines.find((e) => e.id === agent.default_engine_id)?.id ?? engines[0]?.id ?? null;
        });
      })
      .catch((err) => mounted.current && setCatalog({ failed: err instanceof Error ? err.message : String(err) }));
  }, [api, agent.default_engine_id]);
  useEffect(() => load(), [load]);

  const engines = typeof catalog === 'object' && 'engines' in catalog ? catalog.engines : [];
  const engine = engines.find((e) => e.id === selected) ?? null;
  const server = engine ? servers.find((s) => s.id === engine.id) : undefined;
  const installed = !!engine && (engine.installed || !!server);
  const keyRequired = !!engine && engine.needsKey && !engine.hasKey;
  const busy = phase === 'running';

  async function use() {
    if (!engine) return;
    const plan: EnginePlan = {
      agentId: agent.id,
      engineId: engine.id,
      install: !installed,
      start: installed && server?.status !== 'Connected',
      key: engine.needsKey && key.trim() ? key.trim() : null,
    };
    setSteps(stepsFor(plan).map((step) => ({ step, state: 'waiting' })));
    setFailure(null);
    setPhase('running');
    try {
      await setUpEngine(plan, {
        api,
        watchInstall: watchInstallOver(api.getMarketplaceProgressUrl()),
        onStep: (step, state) => {
          if (mounted.current) setSteps((list) => list.map((s) => (s.step === step ? { step, state } : s)));
        },
      });
      onSet();
      if (!mounted.current) return;
      setKey('');
      setPhase('done');
    } catch (err) {
      if (!mounted.current) return;
      setFailure(err instanceof EngineSetupError ? err : new EngineSetupError('use', 'other', String(err)));
      setPhase('failed');
    }
  }

  const names = { engine: engine?.name ?? '', name: agent.name };

  return (
    <section className="mate-settings mate-preferences mate-engine-setup" aria-label={t('mate.engine_setup_title')}>
      <h2>{t('mate.engine_setup_title')}</h2>
      <p>{t('mate.engine_setup_hint', { name: agent.name })}</p>
      {catalog === 'loading' && <p>{t('mate.engine_loading')}</p>}
      {typeof catalog === 'object' && 'failed' in catalog && (
        <div className="mate-field">
          <p className="mate-problem">{t('mate.engine_list_failed')}</p>
          <p>{catalog.failed}</p>
          <button type="button" onClick={load}>
            {t('mate.retry')}
          </button>
        </div>
      )}
      {typeof catalog === 'object' && 'engines' in catalog && engines.length === 0 && (
        <p>{t('mate.engine_list_empty')}</p>
      )}
      {engines.length > 0 && (
        <fieldset disabled={busy || phase === 'done'}>
          <legend>{t('mate.engine_row')}</legend>
          <div className="mate-engine-list">
            {engines.map((e) => (
              <button
                key={e.id}
                type="button"
                aria-pressed={selected === e.id}
                onClick={() => {
                  setSelected(e.id);
                  setKey('');
                  if (phase === 'failed') setPhase('choosing');
                }}
              >
                <span className="mate-engine-name">{e.name}</span>
                {agent.default_engine_id === e.id ? (
                  <span className="mate-engine-status">{t('mate.engine_in_use')}</span>
                ) : (
                  (e.installed || servers.some((s) => s.id === e.id)) && (
                    <span className="mate-engine-status">{t('mate.engine_installed')}</span>
                  )
                )}
                <span className="mate-engine-meta">
                  {t(e.needsKey ? 'mate.engine_needs_key' : 'mate.engine_no_key')}
                </span>
              </button>
            ))}
          </div>
        </fieldset>
      )}
      {engine?.needsKey && phase !== 'done' && (
        <label className="mate-engine-key">
          <span>{t('mate.engine_key_label')}</span>
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={key}
            disabled={busy}
            onChange={(event) => setKey(event.target.value)}
          />
          <span className="mate-setting-note">
            {engine.hasKey ? t('mate.engine_key_saved') : t('mate.engine_key_new', { engine: engine.name })}
          </span>
        </label>
      )}
      {steps.length > 0 && (
        <ol className="mate-engine-steps" aria-label={t('mate.engine_steps')}>
          {steps.map(({ step, state }) => (
            <li key={step} className={`mate-engine-step mate-step-${state}`}>
              <span className="mate-status-dot" />
              <span>{t(`mate.step_${step}`, names)}</span>
              <span className="mate-setting-state">{t(`mate.step_state_${state}`)}</span>
            </li>
          ))}
        </ol>
      )}
      {phase === 'failed' && failure && (
        <div className="mate-engine-result" role="alert">
          <p className="mate-problem">{t(`mate.engine_fail_${failure.reason}`, names)}</p>
          {failure.detail && <p className="mate-setting-message">{failure.detail}</p>}
        </div>
      )}
      {phase === 'done' && (
        <p className="mate-engine-result" role="status">
          {t('mate.engine_set', names)}
        </p>
      )}
      <div className="mate-memory-footer">
        {phase === 'done' ? (
          <button type="button" onClick={onClose}>
            {t('mate.back_to_settings')}
          </button>
        ) : (
          <>
            <button type="button" disabled={!engine || busy || (keyRequired && !key.trim())} onClick={() => void use()}>
              {t('mate.engine_use')}
            </button>
            <button type="button" disabled={busy} onClick={onClose}>
              {t('mate.engine_cancel')}
            </button>
            <button type="button" disabled={busy} onClick={onDetails}>
              {t('mate.engine_details')}
            </button>
          </>
        )}
      </div>
    </section>
  );
}
