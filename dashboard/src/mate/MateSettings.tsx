import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import { displayServerId } from '../lib/format';
import type { AgentMemoryTarget, AgentMetadata, McpServerInfo } from '../types';
import { EngineSetup } from './EngineSetup';
import { CHAT_SIZES, type ChatSize, GLASS_PROFILES, type GlassProfile } from './matePreferences';

export type ServerState = 'unset' | 'checking' | 'unknown' | 'missing' | 'running' | 'starting' | 'stopped' | 'error';

export interface ServerSummary {
  name: string;
  state: ServerState;
  /** The kernel's own words when the server failed. Shown as given, never rephrased. */
  message?: string;
}

/**
 * What one configured server (the partner's engine or memory) is doing right now.
 * `servers` is null while the list is on its way (`checking`) or when it could not be
 * read (`unknown`): an empty list in either case would read as "not registered".
 */
export function serverSummary(
  id: string | undefined,
  servers: McpServerInfo[] | null,
  withoutList: 'checking' | 'unknown' = 'unknown',
): ServerSummary | null {
  if (!id) return null;
  const server = servers?.find((s) => s.id === id);
  const name = server?.display_name || displayServerId(id);
  if (!servers) return { name, state: withoutList };
  if (!server) return { name, state: 'missing' };
  switch (server.status) {
    case 'Connected':
      return { name, state: 'running' };
    case 'Connecting':
    case 'Restarting':
      return { name, state: 'starting' };
    case 'Error':
      return { name, state: 'error', message: server.status_message };
    default:
      // Registered (not started yet), Draining and Disconnected all mean it is not answering now.
      return { name, state: 'stopped' };
  }
}

const SIZE_LABEL: Record<ChatSize, string> = { side: 'mate.chat_size_side', expanded: 'mate.chat_size_expanded' };
const GLASS_LABEL: Record<GlassProfile, string> = {
  soft: 'mate.glass_soft',
  crisp: 'mate.glass_crisp',
  still: 'mate.glass_still',
};

export function MateSettings({
  agent,
  connected,
  questions,
  chatSize,
  onChatSize,
  glass,
  onGlass,
  onPartner,
  onReview,
  onAgentSettings,
  onAgentChanged,
  onServers,
  onAppSettings,
  onBack,
}: {
  /** Null when there is no partner yet: the display and app sections still apply. */
  agent: AgentMetadata | null;
  connected: boolean;
  questions: number;
  chatSize: ChatSize;
  onChatSize: (size: ChatSize) => void;
  glass: GlassProfile;
  onGlass: (glass: GlassProfile) => void;
  onPartner: () => void;
  onReview: () => void;
  onAgentSettings: (section: 'engine') => void;
  /** Something on the partner was changed from here; read the partners again. */
  onAgentChanged: () => void;
  onServers: () => void;
  onAppSettings: () => void;
  onBack: () => void;
}) {
  const { t } = useTranslation('agents');
  const api = useApi();
  const { servers, isLoading, error, refetch } = useMcpServers();
  // The kernel picks the memory server by capability, not from the agent's settings, so the row
  // shows the kernel's own answer for this partner rather than anything stored on the agent.
  const [memoryTarget, setMemoryTarget] = useState<AgentMemoryTarget | 'checking' | 'unknown'>('checking');
  const memoryRequest = useRef(0);
  const agentId = agent?.id;
  const loadMemoryTarget = useCallback(() => {
    const request = ++memoryRequest.current;
    if (!agentId) return;
    setMemoryTarget('checking');
    api
      .getAgentMemory(agentId)
      .then((target) => request === memoryRequest.current && setMemoryTarget(target))
      .catch(() => request === memoryRequest.current && setMemoryTarget('unknown'));
  }, [api, agentId]);
  useEffect(() => loadMemoryTarget(), [loadMemoryTarget]);
  // A list read while the kernel was away is an error; read it again once it is back.
  const wasConnected = useRef(connected);
  useEffect(() => {
    if (connected && !wasConnected.current) {
      void refetch();
      loadMemoryTarget();
    }
    wasConnected.current = connected;
  }, [connected, refetch, loadMemoryTarget]);
  // Setting up the engine replaces the panel's contents; switching partner returns to the panel.
  const [view, setView] = useState<'panel' | 'engine'>('panel');
  useEffect(() => setView('panel'), [agentId]);
  const list = isLoading || error ? null : servers;
  const withoutList = isLoading ? 'checking' : 'unknown';
  const engine = serverSummary(agent?.default_engine_id, list, withoutList);

  function serverValue(summary: ServerSummary | null) {
    if (!summary) return <span>{t('mate.server_unset')}</span>;
    return (
      <>
        <span className="mate-setting-name">{summary.name}</span>
        {stateBadge(summary.state)}
        {summary.message && <span className="mate-setting-message">{summary.message}</span>}
      </>
    );
  }
  function stateBadge(state: ServerState) {
    return (
      <span className={`mate-setting-state mate-server-${state}`}>
        <span className="mate-status-dot" />
        {t(`mate.server_${state}`)}
      </span>
    );
  }
  function memoryValue() {
    if (typeof memoryTarget === 'string') return stateBadge(memoryTarget);
    if (memoryTarget.plugin) return serverValue({ name: memoryTarget.plugin, state: 'running' });
    if (!memoryTarget.server_id) return <span>{t('mate.memory_none')}</span>;
    return (
      <>
        {serverValue(serverSummary(memoryTarget.server_id, list, withoutList))}
        {memoryTarget.allowed === false && <span className="mate-setting-note">{t('mate.memory_not_allowed')}</span>}
      </>
    );
  }
  // Only a state that stops this engine from answering gets the consequence; checking and unknown
  // are not facts about the engine, and the chat can still pick another engine per message.
  const engineDown = engine !== null && ['missing', 'stopped', 'error'].includes(engine.state);

  if (view === 'engine' && agent) {
    return (
      <EngineSetup
        agent={agent}
        onSet={() => {
          void refetch();
          onAgentChanged();
        }}
        onClose={() => setView('panel')}
        onDetails={() => onAgentSettings('engine')}
      />
    );
  }

  return (
    <section className="mate-settings mate-preferences" aria-label={t('mate.settings')}>
      <h2>{t('mate.settings')}</h2>
      <section className="mate-field" aria-labelledby="mate-display">
        <h3 id="mate-display">{t('mate.settings_display')}</h3>
        <p>{t('mate.settings_display_hint')}</p>
        <fieldset>
          <legend>{t('mate.chat_size')}</legend>
          <div className="mate-preset-list">
            {CHAT_SIZES.map((size) => (
              <button key={size} type="button" aria-pressed={chatSize === size} onClick={() => onChatSize(size)}>
                {t(SIZE_LABEL[size])}
              </button>
            ))}
          </div>
        </fieldset>
        <fieldset>
          <legend>{t('mate.glass')}</legend>
          <div className="mate-preset-list">
            {GLASS_PROFILES.map((profile) => (
              <button key={profile} type="button" aria-pressed={glass === profile} onClick={() => onGlass(profile)}>
                {t(GLASS_LABEL[profile])}
              </button>
            ))}
          </div>
        </fieldset>
      </section>
      {agent && (
        <section className="mate-field" aria-labelledby="mate-partner-setup">
          <h3 id="mate-partner-setup">{agent.name}</h3>
          <dl className="mate-setting-rows">
            <div>
              <dt>{t('mate.appearance_row')}</dt>
              <dd>
                <button type="button" onClick={onPartner}>
                  {t('mate.change')}
                </button>
              </dd>
            </div>
            <div>
              <dt>{t('mate.engine_row')}</dt>
              <dd>
                {serverValue(engine)}
                {engineDown && <span className="mate-setting-note">{t('mate.engine_cannot_answer')}</span>}
                <button type="button" onClick={() => setView('engine')}>
                  {t(engine && !engineDown ? 'mate.change' : 'mate.set_up')}
                </button>
              </dd>
            </div>
            <div>
              <dt>{t('mate.memory_row')}</dt>
              <dd>
                {memoryValue()}
                <span className="mate-setting-note">{t('mate.memory_auto')}</span>
              </dd>
            </div>
            <div>
              <dt>{t('mate.approvals_row')}</dt>
              <dd>
                <span>{questions > 0 ? t('mate.approvals_count', { n: questions }) : t('mate.approvals_none')}</span>
                {questions > 0 && (
                  <button type="button" disabled={!connected} onClick={onReview}>
                    {t('mate.review_question')}
                  </button>
                )}
              </dd>
            </div>
          </dl>
        </section>
      )}
      <section className="mate-field" aria-labelledby="mate-app">
        <h3 id="mate-app">{t('mate.settings_app')}</h3>
        <dl className="mate-setting-rows">
          <div>
            <dt>{t('mate.kernel_row')}</dt>
            <dd>
              <span className={`mate-setting-state ${connected ? 'mate-server-running' : 'mate-server-stopped'}`}>
                <span className="mate-status-dot" />
                {t(connected ? 'mate.kernel_connected' : 'mate.state_offline')}
              </span>
            </dd>
          </div>
        </dl>
        <div className="mate-memory-footer">
          <button type="button" onClick={onServers}>
            {t('mate.servers_and_permissions')}
          </button>
          <button type="button" onClick={onAppSettings}>
            {t('mate.app_settings')}
          </button>
        </div>
      </section>
      <div className="mate-memory-footer">
        <button type="button" onClick={onBack}>
          {t('mate.back')}
        </button>
      </div>
    </section>
  );
}
