import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useActionsContext } from '../contexts/ActionsContext';
import { useAgentContext } from '../contexts/AgentContext';
import { type FirstMessage, useConversations } from '../contexts/ConversationContext';
import { useUserIdentity } from '../contexts/UserIdentityContext';
import { useApi } from '../hooks/useApi';
import { useEventStream } from '../hooks/useEventStream';
import { useMcpServers } from '../hooks/useMcpServers';
import { useStickToBottom } from '../hooks/useStickToBottom';
import { AgentIcon } from '../lib/agentIdentity';
import { buildOutgoingChat } from '../lib/chatSend';
import { type DayLabel, dayBreaks, dayLabel, relativeTime, timeOfDay } from '../lib/chatTime';
import { displayTitle } from '../lib/conversations';
import { findBranchPoints, flattenConversation } from '../lib/conversationTree';
import { engineErrorOf } from '../lib/engineError';
import { markInline, unmarkInline } from '../lib/inlineApprovals';
import { mostSevere } from '../lib/notificationSeverity';
import { sendNativeNotification } from '../lib/notifications';
import { isEngineServer } from '../lib/serverCategory';
import { openVrmWindow } from '../lib/tauri';
import { thinkingStorageKey } from '../lib/thinkingSteps';
import { EVENTS_URL } from '../services/api';
import type {
  AgentMetadata,
  AgentTokenStreamData,
  AgentToolUseStreamData,
  ChatMessage,
  ClotoMessage,
  CommandApprovalRequest,
  ContentBlock,
  McpServerInfo,
  PendingRejection,
  ToolRejectionData,
} from '../types';
import { useGazeBroadcast } from '../vrm/useGazeBroadcast';
import { ActionsPanel } from './ActionsPanel';
import { BranchNavigator } from './BranchNavigator';
import { ChatInputBar } from './ChatInputBar';
import './ChatRoom.css';
import { CommandApprovalCard } from './CommandApprovalCard';
import { MessageContent } from './ContentBlockView';
import { ContextUsageBadge } from './ContextUsageBadge';
import { DiagnosticsModal } from './DiagnosticsModal';
import { ToolRejectionCard } from './ToolRejectionCard';
import { TypewriterMessage } from './TypewriterMessage';

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'cloto-chat-';
/** How many earlier threads the empty room offers to continue from. */
const CONTINUE_FROM_COUNT = 3;

/** Migrate legacy localStorage session data to server */
async function migrateLegacyData(
  agentId: string,
  postChatMessage: (agentId: string, msg: { id: string; source: string; content: ContentBlock[] }) => Promise<unknown>,
) {
  const key = LEGACY_SESSION_KEY_PREFIX + agentId;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return;
    const legacyMessages: ClotoMessage[] = JSON.parse(raw);
    if (!Array.isArray(legacyMessages) || legacyMessages.length === 0) {
      localStorage.removeItem(key);
      return;
    }

    // Migrate each message to server
    for (const msg of legacyMessages) {
      const source = msg.source.type === 'User' ? 'user' : msg.source.type === 'Agent' ? 'agent' : 'system';
      await postChatMessage(agentId, {
        id: msg.id,
        source,
        content: [{ type: 'text' as const, text: msg.content }],
      }).catch(() => {}); // Ignore duplicate ID errors
    }

    // Remove legacy data
    localStorage.removeItem(key);
  } catch {
    // Silently ignore migration errors
  }
}

function textOf(content: ContentBlock[] | unknown): string {
  return Array.isArray(content)
    ? content
        .filter((b) => b.type === 'text')
        .map((b) => b.text || '')
        .join('\n')
    : '';
}

type ThinkingStep = {
  id: number;
  status: 'ok' | 'fail' | 'done' | 'thought' | 'running';
  text: string;
  detail?: string;
  ts: number;
};

export function AgentConsole({
  agent,
  conversationId: openedConversationId,
  onFirstMessage,
  initialSend,
  onBack,
  onConfigure,
}: {
  agent: AgentMetadata;
  /** The conversation to show. `null` is a draft: nothing exists yet, and the
   * first message creates it through `onFirstMessage`. */
  conversationId: string | null;
  /** Create the draft's conversation and answer its id. */
  onFirstMessage?: () => Promise<string>;
  /** What the new chat screen was given to say: sent once, on mount. */
  initialSend?: FirstMessage;
  onBack: () => void;
  onConfigure?: () => void;
}) {
  const { t, i18n } = useTranslation('agents');
  const api = useApi();
  const { identity } = useUserIdentity();
  const { agents, setSelectedAgentId } = useAgentContext();
  const { conversations, open: openConversation, refresh: refreshConversations } = useConversations();
  const { servers: mcpServers } = useMcpServers();
  const [agentEngines, setAgentEngines] = useState<McpServerInfo[]>([]);
  // The conversation this console talks in. It starts as what was opened and,
  // for a draft, becomes the conversation its first message created — held here
  // because the console outlives that moment (it is not remounted).
  const [conversationId, setConversationId] = useState<string | null>(openedConversationId);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isTyping, setIsTyping] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasMore, setHasMore] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [pendingResponse, setPendingResponse] = useState<{
    id: string;
    text: string;
    elapsedSecs: number;
    parentId?: string;
    /** Live MGP §12 streaming in progress — render raw text (no typewriter)
     * and skip the final TypewriterMessage animation when ThoughtResponse
     * arrives (text has already been shown to the user chunk-by-chunk). */
    streaming?: boolean;
  } | null>(null);
  const [thinkingSteps, setThinkingStepsRaw] = useState<ThinkingStep[]>(() => {
    try {
      const saved = sessionStorage.getItem(thinkingStorageKey(agent.id));
      return saved ? JSON.parse(saved) : [];
    } catch {
      return [];
    }
  });
  // Wrap setter to persist to sessionStorage
  const setThinkingSteps: typeof setThinkingStepsRaw = (action) => {
    setThinkingStepsRaw((prev) => {
      const next = typeof action === 'function' ? action(prev) : action;
      try {
        if (next.length === 0) {
          sessionStorage.removeItem(thinkingStorageKey(agent.id));
        } else {
          sessionStorage.setItem(thinkingStorageKey(agent.id), JSON.stringify(next.slice(-50)));
        }
      } catch {
        /* storage full */
      }
      return next;
    });
  };
  const [pendingApprovals, setPendingApprovals] = useState<CommandApprovalRequest[]>([]);
  const [pendingRejections, setPendingRejections] = useState<PendingRejection[]>([]);
  const thinkingIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const initialLoadDone = useRef(false);
  const sendTimestampRef = useRef<number>(0);
  // Holds the correct parent_id during retry (null = not retrying)
  const retryParentIdRef = useRef<string | null>(null);
  // The user message whose reply is being produced, and the ones the user
  // stopped waiting for. The kernel has no way to be told to stop, so a stop
  // is the room ceasing to listen: the reply that still arrives for a stopped
  // message is not drawn (it is in the store, and shows on the next load).
  const inflightSourceIdRef = useRef<string | null>(null);
  const stoppedSourceIdsRef = useRef<Set<string>>(new Set());
  const actions = useActionsContext();
  const [activeBranches, setActiveBranches] = useState<Record<string, number>>({});
  const [editingMessage, setEditingMessage] = useState<ChatMessage | null>(null);
  // The engine error a report is being written for, while the report is open.
  const [reportOf, setReportOf] = useState<string | null>(null);
  // Messages that never reached the kernel, by id: why, and what to send again.
  // They stay in the room where they were written; nothing here is stored.
  const [failedSends, setFailedSends] = useState<
    Record<string, { reason: string; blocks: ContentBlock[]; engineOverride: string | null }>
  >({});
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);
  const hasVrm = agent.metadata?.has_vrm === 'true';
  useGazeBroadcast(hasVrm);
  // Follow the newest turn as it arrives (bug-498), unless the user scrolled up.
  const { onScroll: handleScroll, pinned, scrollToBottom } = useStickToBottom(scrollRef);

  // Flatten branching conversation to linear display
  const displayMessages = useMemo(() => flattenConversation(messages, activeBranches), [messages, activeBranches]);
  const branchPoints = useMemo(() => findBranchPoints(messages, activeBranches), [messages, activeBranches]);
  const breaks = useMemo(() => dayBreaks(displayMessages.map((m) => m.created_at)), [displayMessages]);

  const conversation = conversations.find((c) => c.id === conversationId);
  const earlierThreads = useMemo(
    () =>
      conversations
        .filter((c) => c.agent_id === agent.id && c.id !== conversationId && c.message_count > 0)
        .slice(0, CONTINUE_FROM_COUNT),
    [conversations, agent.id, conversationId],
  );

  // Resolve the agent's granted engine servers for the engine selector.
  useEffect(() => {
    api
      .getAgentAccess(agent.id)
      .then(({ entries }) => {
        const grantedServerIds = new Set(
          entries.filter((e) => e.entry_type === 'server_grant' && e.permission === 'allow').map((e) => e.server_id),
        );
        // Keep any granted server the kernel treats as an engine — including
        // de-prefixed ClotoHub catalog engines (e.g. `deepseek`) that lack the
        // legacy `mind.` prefix (bug-388/396). The old prefix-only filter
        // dropped them, so catalog engines showed as "not connected" here even
        // when granted. Mirror AgentTerminal / AgentPluginWorkspace.
        setAgentEngines(mcpServers.filter((s) => grantedServerIds.has(s.id) && isEngineServer(s)));
      })
      .catch(() => {
        /* engine list may be unavailable */
      });
  }, [agent.id, mcpServers, api.getAgentAccess]);

  // Load initial messages from server
  useEffect(() => {
    if (initialLoadDone.current) return;
    initialLoadDone.current = true;

    const loadMessages = async () => {
      // A draft has no history, and asking without a conversation id would
      // answer with every message this agent has ever been sent.
      if (conversationId === null) {
        setIsLoading(false);
        return;
      }
      try {
        // First, check for legacy localStorage data and migrate
        await migrateLegacyData(agent.id, api.postChatMessage);

        const { messages: loaded, has_more } = await api.getChatMessages(
          agent.id,
          undefined,
          50,
          identity.id,
          conversationId,
        );
        // API returns newest-first; reverse for display (oldest at top)
        const reversed = loaded.reverse();
        setMessages(reversed);
        setHasMore(has_more);
        // Restore typing state: if last message is from user, agent may still be processing.
        // Set a safety timeout to recover if the SSE response was missed.
        if (reversed.length > 0 && reversed[reversed.length - 1].source === 'user') {
          setIsTyping(true);
        }
      } catch (err) {
        if (import.meta.env.DEV) console.error('Failed to load chat messages:', err);
      } finally {
        setIsLoading(false);
      }
    };
    loadMessages();
  }, [agent.id, api, identity.id, conversationId]);

  // The list's order and titles change when a reply lands: refresh it.
  useEffect(() => {
    if (!isTyping) refreshConversations();
  }, [isTyping, refreshConversations]);

  // The questions this agent was already blocked on when the room opened. The
  // stream only carries arrivals; without this a reload would show the agent
  // waiting with nothing to answer.
  useEffect(() => {
    let cancelled = false;
    api
      .getNotifications(true)
      .then((items) => {
        if (cancelled) return;
        const mine = items.filter((i) => i.kind === 'approval' && i.blocking && i.agent_id === agent.id);
        if (mine.length === 0) return;
        setPendingApprovals((prev) => {
          const known = new Set(prev.map((a) => a.approval_id));
          const added = mine
            .filter((i) => !known.has(i.item_id))
            .map((i) => ({
              approval_id: i.item_id,
              agent_id: agent.id,
              commands: (Array.isArray(i.metadata?.commands)
                ? i.metadata.commands
                : []) as CommandApprovalRequest['commands'],
              severity: i.severity,
            }));
          return added.length > 0 ? [...prev, ...added] : prev;
        });
      })
      .catch(() => {
        /* the stream still delivers what arrives from now on */
      });
    return () => {
      cancelled = true;
    };
  }, [api, agent.id]);

  // This room is asking these; the deck must not ask them again over it.
  useEffect(() => {
    const ids = pendingApprovals.map((a) => a.approval_id);
    for (const id of ids) markInline(id);
    return () => {
      for (const id of ids) unmarkInline(id);
    };
  }, [pendingApprovals]);

  useEffect(() => {
    if (!moreOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!moreRef.current?.contains(e.target as Node)) setMoreOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMoreOpen(false);
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [moreOpen]);

  // Recovery: if isTyping is true but we missed the SSE response,
  // re-check the server for messages. Triggers on:
  // 1. 30s timeout while typing
  // 2. Page becoming visible again (user navigated away and back)
  const recoverTypingState = useCallback(async () => {
    if (!isTyping || retryParentIdRef.current || conversationId === null) return;
    try {
      const { messages: latest } = await api.getChatMessages(agent.id, undefined, 5, identity.id, conversationId);
      if (latest.length > 0 && latest[0].source === 'agent') {
        const reversed = latest.reverse();
        setMessages((prev) => {
          const existingIds = new Set(prev.map((m) => m.id));
          const newMsgs = reversed.filter((m) => !existingIds.has(m.id));
          return newMsgs.length > 0 ? [...prev, ...newMsgs] : prev;
        });
        setIsTyping(false);
        setThinkingSteps([]);
      }
    } catch {
      // Silently ignore — next event or timeout will retry
    }
  }, [isTyping, agent.id, api, identity.id, conversationId]);

  useEffect(() => {
    if (!isTyping) return;
    const timer = setTimeout(recoverTypingState, 30_000);
    const onVisibilityChange = () => {
      if (document.visibilityState === 'visible') recoverTypingState();
    };
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [isTyping, recoverTypingState]);

  const loadOlderMessages = useCallback(async () => {
    if (isLoadingMore || !hasMore || messages.length === 0 || conversationId === null) return;
    setIsLoadingMore(true);

    try {
      const oldestTs = messages[0]?.created_at;
      const { messages: older, has_more } = await api.getChatMessages(
        agent.id,
        oldestTs,
        50,
        identity.id,
        conversationId,
      );

      if (older.length > 0) {
        // Preserve scroll position
        const scrollEl = scrollRef.current;
        const prevHeight = scrollEl?.scrollHeight || 0;

        setMessages((prev) => [...older.reverse(), ...prev]);
        setHasMore(has_more);

        // Restore scroll position after prepending
        requestAnimationFrame(() => {
          if (scrollEl) {
            scrollEl.scrollTop = scrollEl.scrollHeight - prevHeight;
          }
        });
      } else {
        setHasMore(false);
      }
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to load older messages:', err);
    } finally {
      setIsLoadingMore(false);
    }
  }, [agent.id, api, messages, isLoadingMore, hasMore, identity.id, conversationId]);

  // Lazy load older messages on scroll to top
  useEffect(() => {
    if (!hasMore || isLoading) return;
    const sentinel = sentinelRef.current;
    if (!sentinel) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && hasMore && !isLoadingMore) {
          loadOlderMessages();
        }
      },
      { root: scrollRef.current, threshold: 0.1 },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, isLoading, isLoadingMore, loadOlderMessages]);

  // Subscribe to system-wide events
  useEventStream(
    EVENTS_URL,
    (event) => {
      // SSE reconnection / lagged recovery: refetch recent messages from API
      if (event.type === '__reconnected' || event.type === '__lagged') {
        api
          .getChatMessages(agent.id, undefined, 10, identity.id)
          .then(({ messages: latest }) => {
            if (latest.length > 0) {
              const reversed = latest.reverse();
              setMessages((prev) => {
                const existingIds = new Set(prev.map((m) => m.id));
                const newMsgs = reversed.filter((m) => !existingIds.has(m.id));
                if (newMsgs.length > 0) {
                  if (newMsgs.some((m) => m.source === 'agent')) {
                    setIsTyping(false);
                    setThinkingSteps([]);
                    // A streaming pendingResponse would otherwise linger on
                    // screen as a zombie after the authoritative final
                    // message has already been loaded from the DB.
                    setPendingResponse((prev) => (prev?.streaming ? null : prev));
                  }
                  return [...prev, ...newMsgs];
                }
                return prev;
              });
            }
          })
          .catch(() => {
            /* non-critical: may fail during reconnect */
          });
        return;
      }

      // Thinking process visualization. An event carrying engine_id is an
      // engine event by construction — ids are bare, so presence
      // is the signal, not a prefix.
      if (event.data?.agent_id === agent.id || event.data?.engine_id !== undefined) {
        if (event.type === 'ToolInvoked' && event.data.agent_id === agent.id) {
          const tool = (event.data.tool_name as string) || 'unknown';
          const hint = event.data.tool_hint as string | undefined;
          const success = event.data.success as boolean;
          setThinkingSteps((prev) => [
            ...prev,
            {
              id: thinkingIdRef.current++,
              status: success ? 'ok' : 'fail',
              text: hint || tool,
              detail: success ? `${event.data.duration_ms}ms` : 'failed',
              ts: Date.now(),
            },
          ]);
        }
        // ToolRejected — kernel-issued structured rejection. Emitted alongside
        // ToolInvoked{success:false}; drives the dedicated ToolRejectionCard
        // rendered below (dismissable, purely informational).
        if (event.type === 'ToolRejected' && event.data?.agent_id === agent.id) {
          const rej = event.data as unknown as ToolRejectionData;
          const localId = `${rej.call_id}-rejection`;
          setPendingRejections((prev) => {
            if (prev.some((r) => r.local_id === localId)) return prev;
            return [
              ...prev,
              {
                ...rej,
                local_id: localId,
                received_at: Date.now(),
              },
            ];
          });
        }
        if (event.type === 'AgenticLoopCompleted' && event.data.agent_id === agent.id) {
          setThinkingSteps((prev) => [
            ...prev,
            {
              id: thinkingIdRef.current++,
              status: 'done',
              text: 'complete',
              detail: `${event.data.total_iterations || 0} iter, ${event.data.total_tool_calls || 0} calls`,
              ts: Date.now(),
            },
          ]);
        }
        if (event.type === 'AgentThinking' && event.data?.agent_id === agent.id && event.data.content) {
          setThinkingSteps((prev) => [
            ...prev,
            {
              id: thinkingIdRef.current++,
              status: 'thought',
              text: (event.data.content as string).slice(0, 120),
              ts: Date.now(),
            },
          ]);
        }
        // MGP §12 streaming chunk of kind `tool_use`: the engine started one
        // of its own tools. Shown as a thinking step rather than appended to
        // the reply, because it is not part of what the agent says — the
        // pending response must still end up equal to the authoritative text.
        // The kernel did not broker this call and will never learn how it
        // ended, so the step carries no outcome.
        if (event.type === 'AgentToolUseStream' && event.data?.agent_id === agent.id) {
          const use = event.data as unknown as AgentToolUseStreamData;
          setThinkingSteps((prev) => [
            ...prev,
            {
              id: thinkingIdRef.current++,
              status: 'running',
              text: use.tool_name,
              ts: Date.now(),
            },
          ]);
        }
        // MGP §12 streaming chunk (Phase C). Append the delta to the pending
        // response so the user sees tokens as they arrive from the mind.*
        // engine. The authoritative final text still arrives via
        // ThoughtResponse below and overwrites what we've shown so far.
        if (event.type === 'AgentTokenStream' && event.data?.agent_id === agent.id) {
          const stream = event.data as unknown as AgentTokenStreamData;
          // The user stopped waiting for this reply: do not draw its chunks.
          if (stoppedSourceIdsRef.current.has(stream.source_message_id)) return;
          setPendingResponse((prev) => {
            // bug-469: only append when the chunk belongs to the response we are
            // currently showing. A concurrent loop for the same agent (cron,
            // discord, …) can emit chunks for a DIFFERENT source_message_id;
            // appending them would interleave two replies, and wholesale-
            // replacing a non-streaming pendingResponse would drop the user's
            // real answer. Finalize the current pending response first, then
            // start a fresh streaming entry for the new source message.
            if (prev?.streaming && prev.parentId === stream.source_message_id) {
              return { ...prev, text: prev.text + stream.delta };
            }
            if (prev) {
              const prevMsg: ChatMessage = {
                id: prev.id,
                agent_id: agent.id,
                user_id: identity.id,
                source: 'agent',
                content: [{ type: 'text', text: prev.text }],
                metadata: { elapsed_secs: prev.elapsedSecs },
                created_at: Date.now(),
                parent_id: prev.parentId,
              };
              setMessages((msgs) => [...msgs, prevMsg]);
            }
            return {
              id: `${stream.source_message_id}-stream`,
              text: stream.delta,
              elapsedSecs: 0,
              parentId: stream.source_message_id,
              streaming: true,
            };
          });
        }
      }

      // Command approval request from kernel (batch)
      if (event.type === 'CommandApprovalRequested' && event.data?.agent_id === agent.id) {
        const approvalData = event.data as {
          approval_id: string;
          agent_id: string;
          commands?: Array<{ command: string; command_name: string; severity?: unknown }>;
        };
        setPendingApprovals((prev) => {
          if (prev.some((a) => a.approval_id === approvalData.approval_id)) return prev;
          return [
            ...prev,
            {
              approval_id: approvalData.approval_id,
              agent_id: approvalData.agent_id,
              commands: approvalData.commands || [],
              severity: mostSevere((approvalData.commands ?? []).map((c) => c.severity)),
            },
          ];
        });
        if (document.hidden) {
          const count = approvalData.commands?.length || 1;
          sendNativeNotification('Command Approval', `${agent.name}: ${count} command(s) pending`);
        }
      }
      if (event.type === 'CommandApprovalResult') {
        setPendingApprovals((prev) => prev.filter((a) => a.approval_id !== event.data.approval_id));
      }

      // A reply stopped — here or from another window. No ThoughtResponse
      // follows, so this is what ends the wait wherever it is still shown.
      if (event.type === 'ResponseStopped' && event.data.agent_id === agent.id) {
        const sourceId = event.data.source_message_id as string;
        stoppedSourceIdsRef.current.delete(sourceId);
        if (inflightSourceIdRef.current === sourceId) {
          inflightSourceIdRef.current = null;
          setIsTyping(false);
          setThinkingSteps([]);
          setPendingResponse(null);
        }
        return;
      }

      if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
        const sourceId = event.data.source_message_id as string;
        // The reply to a message the user stopped waiting for: it is in the
        // store, but this room said it would stop listening.
        if (stoppedSourceIdsRef.current.delete(sourceId)) return;
        setIsTyping(false);
        setThinkingSteps([]);
        inflightSourceIdRef.current = null;
        const msgId = event.data.source_message_id + '-resp';
        const now = Date.now();
        const elapsedSecs = sendTimestampRef.current > 0 ? Math.round((now - sendTimestampRef.current) / 100) / 10 : 0;
        // Reset for next response in the agentic loop — measures per-message, not cumulative
        sendTimestampRef.current = now;

        // Use correct parent_id: retryParentIdRef during retry, otherwise SSE source
        const parentId = retryParentIdRef.current ?? sourceId;
        // Clear retry guard so recoverTypingState can resume
        retryParentIdRef.current = null;

        // If a previous typewriter is still running, finalize it immediately.
        // When the previous pendingResponse was in MGP §12 streaming mode
        // (Phase C), we've already shown the text to the user chunk-by-chunk;
        // we trust `event.data.content` as authoritative (MGP §12.5) and push
        // it directly to messages without running the typewriter animation
        // a second time.
        setPendingResponse((prev) => {
          if (prev?.streaming) {
            const streamedMsg: ChatMessage = {
              id: msgId,
              agent_id: agent.id,
              user_id: identity.id,
              source: 'agent',
              content: [{ type: 'text', text: event.data.content as string }],
              metadata: { elapsed_secs: elapsedSecs },
              created_at: Date.now(),
              parent_id: parentId as string | undefined,
            };
            setMessages((msgs) => [...msgs, streamedMsg]);
            return null;
          }
          if (prev) {
            const prevMsg: ChatMessage = {
              id: prev.id,
              agent_id: agent.id,
              user_id: identity.id,
              source: 'agent',
              content: [{ type: 'text', text: prev.text }],
              metadata: { elapsed_secs: prev.elapsedSecs },
              created_at: Date.now(),
              parent_id: prev.parentId,
            };
            setMessages((msgs) => [...msgs, prevMsg]);
          }
          // A turn the engine failed to produce is not typed out as if the
          // agent were saying it: it goes into the room already in its own form.
          if (engineErrorOf(event.data.content as string) !== null) {
            const failedMsg: ChatMessage = {
              id: msgId,
              agent_id: agent.id,
              user_id: identity.id,
              source: 'agent',
              content: [{ type: 'text', text: event.data.content as string }],
              created_at: Date.now(),
              parent_id: parentId as string | undefined,
            };
            setMessages((msgs) => [...msgs, failedMsg]);
            return null;
          }
          return {
            id: msgId,
            text: event.data.content as string,
            elapsedSecs,
            parentId: parentId as string | undefined,
          };
        });

        // Agent response is persisted backend-side (system.rs) before SSE emission.
      }

      // AgentDialogue events handled globally by ActionsProvider
    },
    api.apiKey,
  );

  // Typewriter completion: move pending response to static messages
  const handleTypewriterComplete = useCallback(() => {
    setPendingResponse((prev) => {
      if (!prev) return null;
      const agentMsg: ChatMessage = {
        id: prev.id,
        agent_id: agent.id,
        user_id: identity.id,
        source: 'agent',
        content: [{ type: 'text', text: prev.text }],
        metadata: { elapsed_secs: prev.elapsedSecs },
        created_at: Date.now(),
        parent_id: prev.parentId,
      };
      setMessages((msgs) => [...msgs, agentMsg]);
      return null;
    });
  }, [agent.id, identity.id]);

  const handleCodeBlockExtracted = useCallback(
    (code: string, language: string, lineCount: number) => {
      if (lineCount >= 15) {
        actions.addArtifact({ code, language, lineCount });
      }
    },
    [actions.addArtifact],
  );

  const sendMessage = async (blocks?: ContentBlock[], rawText?: string, engineOverride?: string | null) => {
    const text = rawText ?? '';
    const contentBlocks = blocks ?? (text.trim() ? [{ type: 'text' as const, text: text.trim() }] : []);
    if (contentBlocks.length === 0 || isTyping || pendingResponse) return;
    actions.clearArtifacts();

    const msgId = Date.now().toString();
    const userMsg: ChatMessage = {
      id: msgId,
      agent_id: agent.id,
      user_id: identity.id,
      source: 'user',
      content: contentBlocks,
      created_at: Date.now(),
    };

    setMessages((prev) => [...prev, userMsg]);
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = Date.now();
    inflightSourceIdRef.current = msgId;

    try {
      // The first message of a draft is what makes it a conversation.
      let target = conversationId;
      if (target === null) {
        if (!onFirstMessage) throw new Error('No conversation is open');
        target = await onFirstMessage();
        setConversationId(target);
      }
      const hasMedia = contentBlocks.some((b) => b.type === 'image' || b.type === 'audio');
      const outgoing = buildOutgoingChat({
        messageId: msgId,
        agentId: agent.id,
        conversationId: target,
        identity,
        contentBlocks,
        engineOverride,
        hasMedia,
      });
      // If content blocks include media (image/audio), persist them via
      // postChatMessage first so the kernel can find attachments in DB
      // when running maybe_analyze_images / maybe_transcribe_audio.
      if (hasMedia) {
        await api.postChatMessage(agent.id, outgoing.stored);
      }
      await api.postChat(outgoing.dispatched);
      refreshConversations();
    } catch (err) {
      // The message stays where it was written — taking it away took what the
      // user typed with it — marked as not sent, with the reason, until it is
      // sent again or edited.
      setIsTyping(false);
      inflightSourceIdRef.current = null;
      const reason = err instanceof Error ? err.message : '';
      if (import.meta.env.DEV) console.error('Failed to send message:', reason);
      setFailedSends((prev) => ({
        ...prev,
        [msgId]: { reason, blocks: contentBlocks, engineOverride: engineOverride ?? null },
      }));
    }
  };

  // Take a message that was never sent out of the room (it is sent again, or
  // replaced by its edit, as a new message).
  const dropFailedSend = (id: string) => {
    setMessages((prev) => prev.filter((m) => m.id !== id));
    setFailedSends((prev) => {
      const { [id]: _, ...rest } = prev;
      return rest;
    });
  };

  const sendAgain = (msg: ChatMessage) => {
    const failed = failedSends[msg.id];
    if (!failed || isTyping || pendingResponse) return;
    dropFailedSend(msg.id);
    sendMessage(failed.blocks, undefined, failed.engineOverride);
  };

  // The new chat screen hands its first message over by mounting this console
  // with it. A ref, not state: it must go out once even if this renders twice.
  const initialSent = useRef(false);
  // Once, on mount: `sendMessage` is a fresh function each render and is not a dependency.
  useEffect(() => {
    if (!initialSend || initialSent.current) return;
    initialSent.current = true;
    void sendMessage(initialSend.blocks, initialSend.rawText, initialSend.engineOverride);
  }, []);

  /** Stop waiting for the reply: keep what was shown, say so, and ignore the rest. */
  const handleStop = () => {
    if (!isTyping && !pendingResponse) return;
    const sourceId = inflightSourceIdRef.current;
    if (sourceId) {
      stoppedSourceIdsRef.current.add(sourceId);
      // Stop the reply where it is produced, so nothing of it is stored. When
      // the kernel answers that there was nothing to stop, the reply had
      // already finished and is stored: stop holding it back from this room.
      // A failed call keeps it held back — nothing says whether it stopped.
      api
        .stopResponse(agent.id, sourceId)
        .then(({ stopped }) => {
          if (!stopped) stoppedSourceIdsRef.current.delete(sourceId);
        })
        .catch(() => {});
    }
    inflightSourceIdRef.current = null;
    retryParentIdRef.current = null;
    setIsTyping(false);
    setThinkingSteps([]);
    const now = Date.now();
    const shown = pendingResponse;
    setPendingResponse(null);
    if (shown?.text) {
      const partial: ChatMessage = {
        id: shown.id,
        agent_id: agent.id,
        user_id: identity.id,
        source: 'agent',
        content: [{ type: 'text', text: shown.text }],
        metadata: { elapsed_secs: shown.elapsedSecs },
        created_at: now,
        parent_id: shown.parentId,
      };
      setMessages((msgs) => [...msgs, partial]);
    }
    const note: ChatMessage = {
      id: `stopped-${now}`,
      agent_id: agent.id,
      user_id: identity.id,
      source: 'system',
      content: [{ type: 'text', text: t('console.stopped') }],
      created_at: now,
    };
    setMessages((prev) => [...prev, note]);
  };

  const speakText = async (content: ContentBlock[]) => {
    const text = content
      .filter((b) => b.type === 'text')
      .map((b) => b.text || '')
      .join(' ');
    if (!text.trim()) return;
    try {
      const clotoMsg: ClotoMessage = {
        id: Date.now().toString(),
        source: { type: 'User', id: 'system', name: 'System' },
        target_agent: agent.id,
        content: text,
        timestamp: new Date().toISOString(),
        metadata: { target_agent_id: agent.id, tool_hint: 'speak', skip_user_persist: 'true' },
      };
      await api.postChat(clotoMsg);
    } catch (err) {
      if (import.meta.env.DEV) console.error('TTS request failed:', err);
    }
  };

  const copyText = (content: ContentBlock[]) => {
    const text = textOf(content);
    if (text) navigator.clipboard?.writeText(text).catch(() => {});
  };

  // Edit handler: resend edited user message as a new branch
  const handleEditMessage = async (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => {
    if (!editingMessage || isTyping || pendingResponse) return;

    const text = rawText?.trim() || '';
    const contentBlocks = blocks?.length ? blocks : text ? [{ type: 'text' as const, text }] : [];
    if (contentBlocks.length === 0) return;

    const now = Date.now();
    const editId = `edit-${now}`;
    const parentId = editingMessage.parent_id ?? undefined;

    // Count existing siblings to determine branch_index
    const siblingCount = messages.filter((m) => m.parent_id === parentId && m.source === 'user').length;

    const userMsg: ChatMessage = {
      id: editId,
      agent_id: agent.id,
      user_id: identity.id,
      source: 'user',
      content: contentBlocks,
      created_at: now,
      parent_id: parentId,
      branch_index: siblingCount,
    };

    setMessages((prev) => [...prev, userMsg]);
    setEditingMessage(null);
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = now;
    inflightSourceIdRef.current = editId;
    actions.clearArtifacts();

    // Update active branch to show the new edit
    if (parentId) {
      setActiveBranches((prev) => ({ ...prev, [parentId + ':user']: siblingCount }));
    }

    const textContent = contentBlocks
      .filter((b) => b.type === 'text')
      .map((b) => b.text || '')
      .join(' ');

    try {
      const clotoMsg: ClotoMessage = {
        id: editId,
        source: { type: 'User', id: identity.id, name: identity.name },
        target_agent: agent.id,
        content: textContent || '[attachment]',
        timestamp: new Date().toISOString(),
        metadata: {
          target_agent_id: agent.id,
          ...(parentId ? { parent_id: parentId, branch_index: String(siblingCount) } : {}),
          ...(engineOverride ? { engine_override: engineOverride } : {}),
        },
      };
      await api.postChat(clotoMsg);
    } catch (err) {
      setMessages((prev) => prev.filter((m) => m.id !== editId));
      setIsTyping(false);
      inflightSourceIdRef.current = null;
      if (import.meta.env.DEV) console.error('Failed to send edited message:', err);
    }
  };

  const handleChatSend = useCallback(
    (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => {
      if (editingMessage && failedSends[editingMessage.id]) {
        // A message the kernel never received has nothing to branch from: its
        // edit is sent as a new message in its place.
        dropFailedSend(editingMessage.id);
        setEditingMessage(null);
        sendMessage(blocks, rawText, engineOverride);
      } else if (editingMessage) {
        handleEditMessage(blocks, rawText, engineOverride);
      } else {
        sendMessage(blocks, rawText, engineOverride);
      }
    },
    [editingMessage, failedSends, dropFailedSend, handleEditMessage, sendMessage],
  );

  // Retry handler: remove old response immediately, re-generate in place
  const handleRetry = async (agentResponseMsg: ChatMessage) => {
    if (isTyping || pendingResponse) return;

    // Find the user message this response was generated from
    const userMsgId = agentResponseMsg.parent_id ?? agentResponseMsg.id.replace(/-resp$/, '');

    // Store correct parent_id for the new response (also acts as retry-in-progress guard)
    retryParentIdRef.current = userMsgId;
    inflightSourceIdRef.current = userMsgId;
    setMessages((prev) => prev.filter((m) => m.id !== agentResponseMsg.id));
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = Date.now();
    actions.clearArtifacts();

    try {
      await api.retryResponse(agent.id, userMsgId);
    } catch (err) {
      setMessages((prev) => [...prev, agentResponseMsg]);
      retryParentIdRef.current = null;
      inflightSourceIdRef.current = null;
      setIsTyping(false);
      if (import.meta.env.DEV) console.error('Failed to retry response:', err);
    }
  };

  const switchAgent = (agentId: string) => {
    setSelectedAgentId(agentId);
  };

  const generating = isTyping || !!pendingResponse;
  const toolsUsed = thinkingSteps.filter((s) => s.status === 'ok' || s.status === 'fail' || s.status === 'running');
  const lastTurnAt = displayMessages.length > 0 ? displayMessages[displayMessages.length - 1].created_at : null;
  // A question the agent is asking, or a refusal it met, is drawn in the
  // conversation — so a room with one is not empty even before anyone spoke.
  const empty =
    !isLoading &&
    displayMessages.length === 0 &&
    !generating &&
    pendingApprovals.length === 0 &&
    pendingRejections.length === 0;

  const dayText = (label: DayLabel) =>
    label.kind === 'today'
      ? t('console.day_today')
      : label.kind === 'yesterday'
        ? t('console.day_yesterday')
        : t('console.date_md', { month: label.month, day: label.day });

  const threadDate = (ts: number) => {
    const label = dayLabel(ts);
    return label.kind === 'today' ? `${dayText(label)} ${timeOfDay(ts, i18n.language)}` : dayText(label);
  };

  const state = !agent.enabled ? (
    <b>{t('console.offline')}</b>
  ) : generating ? (
    <>
      <b>{t('console.state_thinking')}</b>
      {toolsUsed.length > 0 && `${t('console.sentence_end')}${t('console.tools_used', { count: toolsUsed.length })}`}
    </>
  ) : (
    <>
      <b>{t('console.state_idle')}</b>
      {lastTurnAt !== null &&
        `${t('console.sentence_end')}${t('console.talked', { when: relativeTime(lastTurnAt, Date.now(), i18n.language) })}`}
    </>
  );

  const tools = (
    <>
      <span className="spacer" />
      <button
        type="button"
        className="tool"
        onClick={onConfigure}
        title={t('console.agent_settings')}
        aria-label={t('console.agent_settings')}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" />
        </svg>
      </button>
      <div className="menu-anchor" ref={moreRef}>
        <button
          type="button"
          className="tool"
          onClick={() => setMoreOpen((v) => !v)}
          title={t('console.more')}
          aria-label={t('console.more')}
          aria-haspopup="menu"
          aria-expanded={moreOpen}
        >
          <svg viewBox="0 0 24 24" fill="currentColor">
            <circle cx="5" cy="12" r="1.4" />
            <circle cx="12" cy="12" r="1.4" />
            <circle cx="19" cy="12" r="1.4" />
          </svg>
        </button>
        {moreOpen && (
          <div className="menu" role="menu">
            {hasVrm && (
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMoreOpen(false);
                  openVrmWindow(agent.id, api.apiKey);
                }}
              >
                {t('console.open_vrm')}
              </button>
            )}
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMoreOpen(false);
                onBack();
              }}
            >
              {t('console.back')}
            </button>
          </div>
        )}
      </div>
    </>
  );

  return (
    <div className="flex h-full">
      <div className="room flex-1 min-w-0">
        {empty ? (
          <header className="face slim">{tools}</header>
        ) : (
          <header className="face talk">
            <div className="pic">
              <AgentIcon agent={agent} size={40} />
            </div>
            <div className="who">
              <div className="name">{agent.name}</div>
              <div className="state">{state}</div>
            </div>
            {conversation && (
              <span className="topic" title={displayTitle(conversation, t('console.untitled'))}>
                {displayTitle(conversation, t('console.untitled'))}
              </span>
            )}
            {tools}
          </header>
        )}

        <div
          ref={scrollRef}
          onScroll={handleScroll}
          onClick={() => editingMessage && setEditingMessage(null)}
          className={`stream${empty ? ' empty' : ''}`}
        >
          {empty ? (
            <div className="col presence">
              <div className="pic">
                <AgentIcon agent={agent} size={84} />
              </div>
              <div className="name">{agent.name}</div>
              <div className="state">{state}</div>
              <p className="remark">{t('console.remark')}</p>
              {earlierThreads.length > 0 && (
                <div className="threads">
                  <div className="h">{t('console.continue_from')}</div>
                  {earlierThreads.map((c) => (
                    <button type="button" key={c.id} onClick={() => openConversation(agent.id, c.id)}>
                      <span className="t">{displayTitle(c, t('console.untitled'))}</span>
                      <span className="d">{threadDate(c.updated_at)}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ) : (
            <div className="col flow">
              {/* Sentinel for lazy loading older messages */}
              {hasMore && <div ref={sentinelRef} className="h-1" />}
              {isLoadingMore && <div className="day">{t('console.loading_older')}</div>}
              {isLoading && <div className="day">{t('console.loading_session')}</div>}

              {displayMessages.map((msg, i) => {
                const isUser = msg.source === 'user';
                const firstText = Array.isArray(msg.content)
                  ? msg.content.find((b) => b.type === 'text')?.text || ''
                  : '';
                const engineError = isUser ? null : engineErrorOf(firstText);
                // Check if this message's parent has branch siblings
                const branchKey = msg.parent_id ? msg.parent_id + ':' + msg.source : null;
                const branch = branchKey ? branchPoints.get(branchKey) : undefined;
                const navigator = branch && (
                  <BranchNavigator
                    count={branch.count}
                    activeIndex={branch.activeIndex}
                    indices={branch.indices}
                    onNavigate={(idx) => {
                      if (branchKey) {
                        setActiveBranches((prev) => ({ ...prev, [branchKey]: idx }));
                      }
                    }}
                  />
                );
                const when = <span>{timeOfDay(msg.created_at, i18n.language)}</span>;
                let turn: React.ReactNode;
                if (engineError !== null) {
                  // Where the reply would be, quietly: the mark is hollow because
                  // the agent said nothing, and the colour is on the engine's own
                  // words. The actions are the ones this turn needs, always shown.
                  turn = (
                    <div className="msg fail">
                      <span className="mark" />
                      <div className="b">
                        <p className="lead">{t('console.reply_failed')}</p>
                        <div className="err select-text">{engineError}</div>
                        <div className="meta">
                          {when}
                          {navigator}
                          <span className="acts">
                            <button type="button" disabled={generating} onClick={() => handleRetry(msg)}>
                              {t('console.retry')}
                            </button>
                            <button type="button" onClick={() => copyText([{ type: 'text', text: engineError }])}>
                              {t('console.copy')}
                            </button>
                            <button type="button" onClick={() => setReportOf(engineError)}>
                              {t('console.report_details')}
                            </button>
                          </span>
                        </div>
                      </div>
                    </div>
                  );
                } else if (msg.source === 'system') {
                  turn = <div className="day">{firstText}</div>;
                } else if (isUser) {
                  const failed = failedSends[msg.id];
                  turn = (
                    <div className={`me${failed ? ' failed' : ''}${editingMessage?.id === msg.id ? ' editing' : ''}`}>
                      <div className="wrap">
                        <div className="b select-text">
                          <MessageContent content={msg.content} />
                        </div>
                        <div className="meta">
                          {failed && (
                            <span className="why">
                              {t('console.send_failed')} {failed.reason}
                            </span>
                          )}
                          <span className="acts">
                            {failed && (
                              <button type="button" disabled={generating} onClick={() => sendAgain(msg)}>
                                {t('console.send_again')}
                              </button>
                            )}
                            <button
                              type="button"
                              disabled={generating}
                              onClick={(e) => {
                                e.stopPropagation();
                                setEditingMessage(msg);
                              }}
                            >
                              {t('console.edit_message')}
                            </button>
                            {!failed && (
                              <button type="button" onClick={() => copyText(msg.content as ContentBlock[])}>
                                {t('console.copy')}
                              </button>
                            )}
                          </span>
                          {navigator}
                          {when}
                        </div>
                      </div>
                    </div>
                  );
                } else {
                  turn = (
                    <div className="msg">
                      <span className="mark" />
                      <div className="b select-text">
                        <MessageContent content={msg.content} />
                        <div className="meta">
                          {when}
                          {navigator}
                          <span className="acts">
                            <button type="button" onClick={() => copyText(msg.content as ContentBlock[])}>
                              {t('console.copy')}
                            </button>
                            <button type="button" disabled={generating} onClick={() => handleRetry(msg)}>
                              {t('console.retry')}
                            </button>
                            <button type="button" onClick={() => speakText(msg.content as ContentBlock[])}>
                              {t('console.read_aloud')}
                            </button>
                          </span>
                        </div>
                      </div>
                    </div>
                  );
                }
                return (
                  <div key={msg.id} className="contents">
                    {breaks[i] && <div className="day">{dayText(dayLabel(msg.created_at))}</div>}
                    {turn}
                  </div>
                );
              })}

              {/* Typewriter animation for current response (or live streaming
                  when MGP §12 chunks are flowing — in that case we render raw
                  text and skip the typewriter since the user already sees the
                  tokens arrive in real time). */}
              {pendingResponse && (
                <div className="msg">
                  <span className="mark" />
                  <div className="b select-text">
                    {pendingResponse.streaming ? (
                      <p className="whitespace-pre-wrap">
                        {pendingResponse.text}
                        <span className="typing" />
                      </p>
                    ) : (
                      <TypewriterMessage
                        text={pendingResponse.text}
                        onComplete={handleTypewriterComplete}
                        onCodeBlock={handleCodeBlockExtracted}
                      />
                    )}
                  </div>
                </div>
              )}

              {/* What the agent is doing before it speaks: its tool calls and
                  thoughts, or — with nothing to show yet — a cursor. The
                  approval card is drawn by CommandApprovalDeck at the window
                  level; while the agent is stopped behind a question, "waiting
                  for an answer" must not be drawn as "thinking". */}
              {isTyping && !pendingResponse && pendingApprovals.length === 0 && (
                <div className="msg">
                  <span className="mark" />
                  <div className="b">
                    {thinkingSteps.length > 0 ? (
                      <details className="inner" open>
                        <summary>
                          <span className="lbl">{t('console.inner')}</span>
                          {t('console.tools_used', { count: toolsUsed.length })}
                          <span className="chev" />
                        </summary>
                        <ol>
                          {thinkingSteps.map((step, i) => (
                            <li key={step.id} className={step.status === 'fail' ? 'fail' : ''}>
                              <span className="num">{i + 1}</span>
                              <span>{step.status === 'thought' ? step.text : <code>{step.text}</code>}</span>
                              {step.detail && <span className="dur num">{step.detail}</span>}
                            </li>
                          ))}
                        </ol>
                      </details>
                    ) : (
                      <span className="typing" />
                    )}
                  </div>
                </div>
              )}

              {/* The agent's questions and the refusals it met, in its own
                  words, at the end of the conversation. The deck leaves these
                  to this room while it is open. */}
              {(pendingApprovals.length > 0 || pendingRejections.length > 0) && (
                <div className="msg">
                  <span className="mark" />
                  <div className="b">
                    {pendingApprovals.map((a, i) => (
                      <CommandApprovalCard
                        key={a.approval_id}
                        approvalId={a.approval_id}
                        commands={a.commands}
                        severity={a.severity}
                        first={i === 0}
                        onResolved={(id) => setPendingApprovals((prev) => prev.filter((x) => x.approval_id !== id))}
                      />
                    ))}
                    {pendingRejections.map((rejection, i) => (
                      <ToolRejectionCard
                        key={rejection.local_id}
                        rejection={rejection}
                        first={pendingApprovals.length === 0 && i === 0}
                        onDismiss={(localId) =>
                          setPendingRejections((prev) => prev.filter((r) => r.local_id !== localId))
                        }
                      />
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>

        {!pinned && !empty && (
          <button type="button" className="latest" onClick={scrollToBottom}>
            {t('console.latest')}
          </button>
        )}

        <ChatInputBar
          onSend={handleChatSend}
          onStop={handleStop}
          generating={generating}
          disabled={!agent.enabled}
          servers={agentEngines}
          editMode={
            editingMessage
              ? {
                  messageId: editingMessage.id,
                  initialContent: Array.isArray(editingMessage.content)
                    ? editingMessage.content
                        .filter((b) => b.type === 'text')
                        .map((b) => b.text || '')
                        .join(' ')
                    : '',
                  onCancel: () => setEditingMessage(null),
                }
              : null
          }
          agentId={agent.id}
          agentName={agent.name}
          agents={agents}
          onSwitchAgent={switchAgent}
          meter={<ContextUsageBadge agentId={agent.id} refreshKey={messages.length} />}
        />
      </div>

      {/* Actions Panel */}
      <ActionsPanel
        isOpen={actions.isOpen}
        onClose={actions.closePanel}
        onOpen={actions.openPanel}
        activeCategory={actions.activeCategory}
        onCategoryChange={actions.setActiveCategory}
        hasDialogues={actions.hasDialogues}
        hasExternalActions={actions.hasExternalActions}
        hasConsensus={actions.hasConsensus}
        artifacts={actions.artifacts}
        activeArtifactIndex={actions.activeArtifactIndex}
        onArtifactTabChange={actions.setActiveArtifactIndex}
        dialogues={actions.dialogues}
        externalActions={actions.externalActions}
        consensusRounds={actions.consensusRounds}
        unreadDialogueCount={actions.unreadDialogueCount}
        unreadExternalCount={actions.unreadExternalCount}
        unreadConsensusCount={actions.unreadConsensusCount}
        totalCount={actions.totalCount}
      />

      {reportOf !== null && (
        <DiagnosticsModal context={t('console.report_context')} message={reportOf} onClose={() => setReportOf(null)} />
      )}
    </div>
  );
}
