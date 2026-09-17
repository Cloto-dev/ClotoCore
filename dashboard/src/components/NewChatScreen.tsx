import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAgentContext } from '../contexts/AgentContext';
import { useConversations } from '../contexts/ConversationContext';
import { useApi } from '../hooks/useApi';
import { useMcpServers } from '../hooks/useMcpServers';
import { AgentIcon } from '../lib/agentIdentity';
import { presenceOrder, stepFace, turnOf } from '../lib/presenceOrder';
import { isEngineServer } from '../lib/serverCategory';
import type { McpServerInfo } from '../types';
import { CreateAgentModal } from './agents/CreateAgentModal';
import { ChatInputBar } from './ChatInputBar';
import './ChatRoom.css';

/** How long after one swipe turn a further wheel movement is ignored, so one
 * gesture (which keeps emitting wheel events as it decays) turns one face. */
const SWIPE_COOLDOWN_MS = 450;
/** How many opening remarks the locale carries (`console.remarks_0` …). */
const REMARK_COUNT = 5;

/**
 * The new chat: who to talk to, and the first thing to say.
 *
 * Nothing exists for it until something is said. The faces — "create an
 * agent" at the left end, then every agent — turn by a horizontal swipe, a
 * drag, or the arrow buttons;
 * the composer below belongs to whoever is facing. With nobody to talk to yet,
 * "create an agent" is the only face.
 */
export function NewChatScreen() {
  const { t } = useTranslation('agents');
  const api = useApi();
  const { agents, refetchAgents } = useAgentContext();
  const { conversations, draft, setDraftAgent, sendDraft } = useConversations();
  const { servers } = useMcpServers();
  const [createOpen, setCreateOpen] = useState(false);
  // The agent just made, by name, until the list that contains it arrives.
  const [arriving, setArriving] = useState<string | null>(null);
  const [engines, setEngines] = useState<McpServerInfo[]>([]);

  const faces = useMemo(() => presenceOrder(agents, conversations), [agents, conversations]);
  // One more face than there are agents: face 0 makes a new agent, and the
  // agents follow it. Making someone is where the row begins, not where it
  // trails off — it is one movement from whoever the new chat opens on.
  const faceCount = faces.length + 1;
  const found = draft?.agentId ? faces.findIndex((a) => a.id === draft.agentId) : -1;
  const index = found + 1;
  const agent = found >= 0 ? faces[found] : null;

  const turn = useCallback(
    (step: -1 | 0 | 1) => {
      if (step === 0) return;
      const next = stepFace(index, step, faceCount);
      if (next === index) return;
      setDraftAgent(next === 0 ? null : faces[next - 1].id);
    },
    [faceCount, faces, index, setDraftAgent],
  );

  // The engines the facing agent may use, for the composer's engine picker.
  const agentId = agent?.id;
  useEffect(() => {
    if (!agentId) {
      setEngines([]);
      return;
    }
    let cancelled = false;
    api
      .getAgentAccess(agentId)
      .then(({ entries }) => {
        if (cancelled) return;
        const granted = new Set(
          entries.filter((e) => e.entry_type === 'server_grant' && e.permission === 'allow').map((e) => e.server_id),
        );
        setEngines(servers.filter((s) => granted.has(s.id) && isEngineServer(s)));
      })
      .catch(() => {
        if (!cancelled) setEngines([]);
      });
    return () => {
      cancelled = true;
    };
  }, [agentId, api, servers]);

  // Someone was just made: turn to them once they are in the list.
  useEffect(() => {
    if (!arriving) return;
    const made = agents.find((a) => a.name === arriving);
    if (!made) return;
    setArriving(null);
    setDraftAgent(made.id);
  }, [agents, arriving, setDraftAgent]);

  // Swipe: a trackpad's horizontal scroll. The movement is summed until it has
  // gone far enough, then one face turns and the rest of the gesture is ignored.
  const swipe = useRef({ dx: 0, until: 0 });
  const onWheel = (e: React.WheelEvent) => {
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    const now = Date.now();
    if (now < swipe.current.until) return;
    // deltaX grows as the content would scroll right, i.e. as it is pulled left.
    swipe.current.dx -= e.deltaX;
    const step = turnOf(swipe.current.dx);
    if (step !== 0) {
      swipe.current = { dx: 0, until: now + SWIPE_COOLDOWN_MS };
      turn(step);
    }
  };

  // Drag: press, move sideways, let go.
  const drag = useRef<{ x: number; id: number } | null>(null);
  const [pull, setPull] = useState(0);
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest('button')) return;
    drag.current = { x: e.clientX, id: e.pointerId };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (drag.current?.id === e.pointerId) setPull(e.clientX - drag.current.x);
  };
  const endDrag = (e: React.PointerEvent) => {
    if (drag.current?.id !== e.pointerId) return;
    const dx = e.clientX - drag.current.x;
    drag.current = null;
    setPull(0);
    turn(turnOf(dx));
  };

  // A remark per new chat, not per render: the draft's key picks it.
  const remarkIndex = useMemo(() => {
    const key = draft?.key ?? '';
    let sum = 0;
    for (let i = 0; i < key.length; i++) sum += key.charCodeAt(i);
    return sum % REMARK_COUNT;
  }, [draft?.key]);

  return (
    <div className="room flex-1 min-w-0">
      <div className="stream empty">
        <div
          className={`col presence turning${pull !== 0 ? ' pulled' : ''}`}
          data-testid="new-chat-faces"
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <button
            type="button"
            className="turn prev"
            aria-label={t('new_chat.previous')}
            title={t('new_chat.previous')}
            disabled={index === 0}
            onClick={() => turn(-1)}
          >
            ‹
          </button>
          <button
            type="button"
            className="turn next"
            aria-label={t('new_chat.next')}
            title={t('new_chat.next')}
            disabled={index === faceCount - 1}
            onClick={() => turn(1)}
          >
            ›
          </button>

          {/* Keyed by who is facing, so a turn is a new face arriving, and the
              pull follows the finger while a drag is under way. */}
          <div className="facing" key={agent?.id ?? 'create'} style={{ transform: `translateX(${pull}px)` }}>
            {agent ? (
              <>
                <div className="pic">
                  <AgentIcon agent={agent} size={84} />
                </div>
                <div className="name">{agent.name}</div>
                <div className="state">
                  {agent.enabled ? <b>{t('console.state_idle')}</b> : t('roster.state_stopped')}
                </div>
                <p className="remark">{t(`console.remarks_${remarkIndex}`)}</p>
              </>
            ) : (
              <>
                <div className="pic nobody">
                  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
                    <circle cx="12" cy="9" r="4" />
                    <path d="M5 20a7 7 0 0 1 14 0" />
                  </svg>
                </div>
                <button type="button" className="name create" onClick={() => setCreateOpen(true)}>
                  {t('new_chat.create_agent')}
                </button>
                <p className="remark">
                  {faces.length === 0 ? t('new_chat.create_first') : t('new_chat.create_another')}
                </p>
              </>
            )}
          </div>
        </div>
      </div>

      <div className="col write">
        <ChatInputBar
          // The composer is the draft's, not the face's: turning keeps what was typed.
          key={draft?.key}
          onSend={(blocks, rawText, engineOverride) => sendDraft({ blocks, rawText, engineOverride })}
          disabled={!agent?.enabled}
          servers={engines}
          agentId={agent?.id}
          agentName={agent?.name}
          agents={faces}
          onSwitchAgent={(id) => setDraftAgent(id)}
        />
      </div>

      {createOpen && (
        <CreateAgentModal
          onClose={() => setCreateOpen(false)}
          onCreated={(name) => {
            setCreateOpen(false);
            setArriving(name);
            void refetchAgents();
          }}
        />
      )}
    </div>
  );
}
