import { Expand, Home, MessageCircle, Monitor, Settings, Sparkles, Users, X } from 'lucide-react';
import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { AgentConsole } from '../components/AgentConsole';
import { CreateAgentModal } from '../components/agents/CreateAgentModal';
import { KernelMonitor } from '../components/KernelMonitor';
import { useAgentContext } from '../contexts/AgentContext';
import { useConnection } from '../contexts/ConnectionContext';
import { useConversations } from '../contexts/ConversationContext';
import { useApi } from '../hooks/useApi';
import { useReadOnOpen } from '../hooks/useUnreadAgents';
import { AgentIcon } from '../lib/agentIdentity';
import { isTauri, openVrmWindow } from '../lib/tauri';
import type { AgentMetadata } from '../types';
import type { VrmAnimationController } from '../vrm/engine/VrmAnimationController';
import { MotionControls } from '../vrm/MotionControls';
import { PartnerMemories } from './PartnerMemories';
import { PartnerSettings } from './PartnerSettings';
import { PartnerStage } from './PartnerStage';
import { attachSoftGlass } from './SoftGlass';
import { type PresenceState, presenceState, usePartnerQuestions } from './usePartnerPresence';
import './MizMate.css';

const STATUS: Record<PresenceState, string> = {
  offline: 'mate.state_offline',
  asking: 'mate.state_asking',
  thinking: 'mate.thinking',
  waiting: 'mate.waiting',
};

function MateConversation({
  agent,
  onBack,
  onConfigure,
}: {
  agent: AgentMetadata;
  onBack: () => void;
  onConfigure: () => void;
}) {
  const { openFor, resolveOpen, draft, commitDraft, mountKeyFor } = useConversations();
  const id = openFor(agent.id);
  const pendingDraft = draft?.agentId === agent.id ? draft : null;
  const [problem, setProblem] = useState(false);
  const { t } = useTranslation('agents');
  useEffect(() => {
    if (!id && !pendingDraft) void resolveOpen(agent.id).catch(() => setProblem(true));
  }, [agent.id, id, pendingDraft, resolveOpen]);
  if (problem) return <p role="alert">{t('mate.chat_failed')}</p>;
  if (!id && !pendingDraft) return <p role="status">{t('mate.loading_chat')}</p>;
  return (
    <AgentConsole
      key={pendingDraft?.key ?? mountKeyFor(agent.id, id as string)}
      agent={agent}
      conversationId={pendingDraft ? null : id}
      onFirstMessage={() => commitDraft(agent.id)}
      initialSend={pendingDraft?.first}
      onBack={onBack}
      onConfigure={onConfigure}
    />
  );
}

export function MizMatePage() {
  const { t } = useTranslation('agents');
  const api = useApi();
  const navigate = useNavigate();
  const location = useLocation();
  const [params] = useSearchParams();
  const {
    agents,
    selectedAgentId,
    setSelectedAgentId,
    systemActive,
    setSystemActive,
    processingAgentIds,
    refetchAgents,
  } = useAgentContext();
  const [page, setPage] = useState<'home' | 'partner' | 'motion' | 'memories'>('home');
  const [chatOpen, setChatOpen] = useState(false);
  const [chatMounted, setChatMounted] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [creating, setCreating] = useState(false);
  const [glass, setGlass] = useState<'soft' | 'crisp' | 'still'>('soft');
  const glassRef = useRef<HTMLButtonElement>(null);
  const motionRef = useRef<VrmAnimationController | null>(null);
  const [motionReady, setMotionReady] = useState(false);
  const receiveRuntime = useCallback((controller: RefObject<VrmAnimationController | null>, ready: boolean) => {
    motionRef.current = controller.current;
    setMotionReady(ready);
  }, []);
  const agent = agents.find((a) => a.id === selectedAgentId) ?? agents[0] ?? null;
  const { connected, checking } = useConnection();
  const questions = usePartnerQuestions(agent?.id ?? null);
  const presence = presenceState({
    connected,
    checking,
    asking: questions > 0,
    thinking: agent ? processingAgentIds.has(agent.id) : false,
  });
  const active = location.pathname === '/';
  useReadOnOpen(chatOpen && active ? (agent?.id ?? null) : null);
  useEffect(() => {
    if (!active) return;
    const requested = params.get('agent');
    if (params.get('system') === 'true') {
      setSystemActive(true);
      return;
    }
    setSystemActive(false);
    if (requested) {
      setSelectedAgentId(requested);
      setChatOpen(true);
      setChatMounted(true);
    }
  }, [active, params, setSelectedAgentId, setSystemActive]);
  useEffect(() => {
    if (!selectedAgentId && agents.length) setSelectedAgentId(agents[0].id);
  }, [agents, selectedAgentId, setSelectedAgentId]);
  useEffect(() => {
    if (!glassRef.current) return;
    return attachSoftGlass(glassRef.current);
  }, [Boolean(agent)]);
  function openChat() {
    setPage('home');
    setChatOpen(true);
    setChatMounted(true);
  }
  if (systemActive)
    return (
      <KernelMonitor
        onClose={() => {
          setSystemActive(false);
          navigate('/');
        }}
      />
    );
  return (
    <div className={`mizmate ${chatOpen ? 'mate-chat-open' : ''} ${expanded ? 'mate-chat-expanded' : ''}`}>
      <header className="mate-header">
        <div className="mate-brand">
          MizMate<small>by MIZPRISM</small>
        </div>
        <div className="mate-header-controls">
          <label className="mate-partner-picker">
            <span>{t('mate.partner')}</span>
            <select
              value={agent?.id ?? ''}
              onChange={(e) => {
                setSelectedAgentId(e.target.value);
                navigate('/');
              }}
            >
              {agents.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
          <button type="button" onClick={() => setCreating(true)}>
            {t('mate.add_partner')}
          </button>
          {agent && (
            <button
              type="button"
              onClick={() => {
                setPage('partner');
                setChatOpen(false);
              }}
            >
              <Settings size={16} />
              {t('mate.partner_settings')}
            </button>
          )}
        </div>
      </header>
      <div className="mate-workspace">
        <nav className="mate-rail" aria-label={t('mate.navigation')}>
          <button type="button" aria-pressed={page === 'home'} onClick={() => setPage('home')}>
            <Home size={19} />
            <span>{t('mate.room')}</span>
          </button>
          <button
            type="button"
            aria-pressed={page === 'motion'}
            onClick={() => {
              setPage('motion');
              setChatOpen(false);
            }}
          >
            <Sparkles size={19} />
            <span>{t('mate.motion')}</span>
          </button>
          <button
            type="button"
            aria-pressed={page === 'memories'}
            onClick={() => {
              setPage('memories');
              setChatOpen(false);
            }}
          >
            <MessageCircle size={19} />
            <span>{t('mate.memories')}</span>
          </button>
          <button type="button" onClick={() => navigate('/mcp-servers')}>
            <Users size={19} />
            <span>{t('mate.capabilities')}</span>
          </button>
          <button type="button" className="mate-rail-bottom" onClick={() => navigate('/settings')}>
            <Settings size={19} />
            <span>{t('mate.settings')}</span>
          </button>
        </nav>
        <div className={`mate-body ${page !== 'home' ? 'mate-editing' : ''}`}>
          {agent ? (
            <>
              <section className="mate-presence" aria-label={t('mate.room')}>
                <div className="mate-presence-heading">
                  <h1>
                    <span className="mate-name-icon">
                      <AgentIcon agent={agent} size={30} />
                    </span>
                    {agent.name}
                  </h1>
                </div>
                <PartnerStage key={agent.id} agent={agent} active={active} onRuntime={receiveRuntime} />
                <p className={`mate-presence-status mate-state-${presence}`} role="status">
                  <span className="mate-status-dot" />
                  {t(STATUS[presence], { n: questions })}
                </p>
                <div className="mate-presence-actions" hidden={page !== 'home'}>
                  <button
                    ref={glassRef}
                    type="button"
                    className="mate-glass"
                    data-glass="light"
                    data-glass-always
                    data-glass-profile={glass}
                    disabled={presence === 'offline'}
                    onClick={openChat}
                  >
                    <span className="label">
                      <MessageCircle size={19} />
                      {t(presence === 'asking' ? 'mate.review_question' : 'mate.talk')}
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      if (isTauri) void openVrmWindow(agent.id, api.apiKey);
                      else window.open(`/vrm-viewer/${encodeURIComponent(agent.id)}`, '_blank', 'noopener');
                    }}
                  >
                    <Monitor size={19} />
                    {t(isTauri ? 'mate.desktop' : 'mate.viewer')}
                  </button>
                </div>
                <div className="mate-presence-footer" hidden={page !== 'home'}>
                  <label>
                    {t('mate.glass')}
                    <select value={glass} onChange={(e) => setGlass(e.target.value as typeof glass)}>
                      <option value="soft">{t('mate.glass_soft')}</option>
                      <option value="crisp">{t('mate.glass_crisp')}</option>
                      <option value="still">{t('mate.glass_still')}</option>
                    </select>
                  </label>
                  <button type="button" onClick={() => setPage('partner')}>
                    {t('mate.change_appearance')}
                  </button>
                </div>
              </section>
              {page === 'partner' && (
                <PartnerSettings
                  key={agent.id}
                  agent={agent}
                  onBack={() => setPage('home')}
                  onAdvanced={() => navigate(`/agents/${encodeURIComponent(agent.id)}/settings`)}
                />
              )}
              {page === 'memories' && (
                <PartnerMemories
                  key={agent.id}
                  agent={agent}
                  onBack={() => setPage('home')}
                  onTalk={openChat}
                  onManage={() => navigate('/dashboard')}
                />
              )}
              {page === 'motion' && (
                <MotionControls
                  key={agent.id}
                  agent={agent}
                  controller={motionRef}
                  ready={motionReady}
                  onSaved={refetchAgents}
                />
              )}
              <aside
                className="mate-conversation"
                hidden={!chatOpen || page !== 'home'}
                aria-label={t('mate.conversation')}
              >
                <div className="mate-conversation-top">
                  <span>
                    <AgentIcon agent={agent} size={26} />
                    {agent.name}
                  </span>
                  <div>
                    <button type="button" aria-pressed={expanded} onClick={() => setExpanded(!expanded)}>
                      <Expand size={16} />
                      {t(expanded ? 'mate.restore' : 'mate.expand')}
                    </button>
                    <button type="button" aria-label={t('mate.close_chat')} onClick={() => setChatOpen(false)}>
                      <X size={19} />
                    </button>
                  </div>
                </div>
                {chatMounted && (
                  <MateConversation
                    key={agent.id}
                    agent={agent}
                    onBack={() => setChatOpen(false)}
                    onConfigure={() => navigate(`/agents/${encodeURIComponent(agent.id)}/settings`)}
                  />
                )}
              </aside>
            </>
          ) : (
            <section className="mate-empty">
              <h1>{t('mate.no_partner')}</h1>
              <button type="button" onClick={() => setCreating(true)}>
                {t('mate.add_partner')}
              </button>
            </section>
          )}
        </div>
      </div>
      {creating && (
        <CreateAgentModal
          onClose={() => setCreating(false)}
          onCreated={async (created) => {
            await refetchAgents();
            setSelectedAgentId(created.id);
            setCreating(false);
            setPage('partner');
          }}
        />
      )}
    </div>
  );
}
