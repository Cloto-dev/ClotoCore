import {
  Activity,
  ArrowLeft,
  Box,
  Pencil,
  RotateCcw as RetryIcon,
  RotateCcw,
  User as UserIcon,
  Volume2,
  Zap,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUserIdentity } from '../contexts/UserIdentityContext';
import { useApi } from '../hooks/useApi';
import { useArtifacts } from '../hooks/useArtifacts';
import { useEventStream } from '../hooks/useEventStream';
import { useLongPress } from '../hooks/useLongPress';
import { useMcpServers } from '../hooks/useMcpServers';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { findBranchPoints, flattenConversation } from '../lib/conversationTree';
import { sendNativeNotification } from '../lib/notifications';
import { openVrmWindow } from '../lib/tauri';
import { EVENTS_URL } from '../services/api';
import type {
  AgentMetadata,
  ChatMessage,
  ClotoMessage,
  CommandApprovalRequest,
  ContentBlock,
  McpServerInfo,
} from '../types';
import { useGazeBroadcast } from '../vrm/useGazeBroadcast';
import { ArtifactPanel } from './ArtifactPanel';
import { BranchNavigator } from './BranchNavigator';
import { ChatInputBar } from './ChatInputBar';
import { CommandApprovalCard } from './CommandApprovalCard';
import { MessageContent } from './ContentBlockView';
import { SkeletonThinking } from './SkeletonThinking';
import { SystemAlertCard } from './SystemAlertCard';
import { TypewriterMessage } from './TypewriterMessage';
import { StatusDot } from './ui/StatusDot';

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'cloto-chat-';
const SCROLL_BOTTOM_THRESHOLD_PX = 50;
const LONG_PRESS_MS = 1500;
const ERROR_DISPLAY_MS = 5000;

function LongPressResetButton({ onReset }: { onReset: () => void }) {
  const { t } = useTranslation('agents');
  const { progress, handlers } = useLongPress(LONG_PRESS_MS, onReset);

  return (
    <button
      {...handlers}
      className="relative px-3 py-1.5 rounded-full border border-edge text-[9px] font-bold text-content-tertiary hover:text-amber-500 hover:border-amber-400/30 transition-all uppercase tracking-widest flex items-center gap-1.5 overflow-hidden"
    >
      {progress > 0 && (
        <span
          className="absolute inset-0 bg-amber-400/20 origin-left transition-none"
          style={{ transform: `scaleX(${progress})` }}
        />
      )}
      <RotateCcw size={10} className={progress > 0 ? 'animate-spin' : ''} />
      <span className="relative">{progress > 0 ? t('console.hold') : t('console.reset')}</span>
    </button>
  );
}

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

export function AgentConsole({ agent, onBack }: { agent: AgentMetadata; onBack: () => void }) {
  const { t } = useTranslation('agents');
  const api = useApi();
  const { identity } = useUserIdentity();
  const { servers: mcpServers } = useMcpServers();
  const [agentEngines, setAgentEngines] = useState<McpServerInfo[]>([]);
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
  } | null>(null);
  const [thinkingSteps, setThinkingStepsRaw] = useState<
    Array<{ id: number; status: 'ok' | 'fail' | 'done' | 'thought'; text: string; detail?: string; ts: number }>
  >(() => {
    try {
      const saved = sessionStorage.getItem(`cloto-thinking-${agent.id}`);
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
          sessionStorage.removeItem(`cloto-thinking-${agent.id}`);
        } else {
          sessionStorage.setItem(`cloto-thinking-${agent.id}`, JSON.stringify(next.slice(-50)));
        }
      } catch {
        /* storage full */
      }
      return next;
    });
  };
  const [pendingApprovals, setPendingApprovals] = useState<CommandApprovalRequest[]>([]);
  const thinkingIdRef = useRef(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const initialLoadDone = useRef(false);
  const isScrolledToBottom = useRef(true);
  const sendTimestampRef = useRef<number>(0);
  // Holds the correct parent_id during retry (null = not retrying)
  const retryParentIdRef = useRef<string | null>(null);
  const artifactPanel = useArtifacts();
  const [activeBranches, setActiveBranches] = useState<Record<string, number>>({});
  const [editingMessage, setEditingMessage] = useState<ChatMessage | null>(null);
  const hasVrm = agent.metadata?.has_vrm === 'true';
  useGazeBroadcast(hasVrm);

  // Flatten branching conversation to linear display
  const displayMessages = useMemo(() => flattenConversation(messages, activeBranches), [messages, activeBranches]);
  const branchPoints = useMemo(() => findBranchPoints(messages, activeBranches), [messages, activeBranches]);

  // Resolve agent's granted mind.* servers for engine selector
  useEffect(() => {
    api
      .getAgentAccess(agent.id)
      .then(({ entries }) => {
        const grantedMindIds = new Set(
          entries
            .filter(
              (e) => e.entry_type === 'server_grant' && e.permission === 'allow' && e.server_id.startsWith('mind.'),
            )
            .map((e) => e.server_id),
        );
        setAgentEngines(mcpServers.filter((s) => grantedMindIds.has(s.id)));
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
      try {
        // First, check for legacy localStorage data and migrate
        await migrateLegacyData(agent.id, api.postChatMessage);

        const { messages: loaded, has_more } = await api.getChatMessages(agent.id, undefined, 50, identity.id);
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
  }, [agent.id, api, identity.id]);

  // Recovery: if isTyping is true but we missed the SSE response,
  // re-check the server for messages. Triggers on:
  // 1. 30s timeout while typing
  // 2. Page becoming visible again (user navigated away and back)
  const recoverTypingState = useCallback(async () => {
    if (!isTyping || retryParentIdRef.current) return;
    try {
      const { messages: latest } = await api.getChatMessages(agent.id, undefined, 5, identity.id);
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
  }, [isTyping, agent.id, api, identity.id]);

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

  // Scroll to bottom on initial load and new messages (only if user is at bottom)
  useEffect(() => {
    if (!isLoading && isScrolledToBottom.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [isLoading]);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    isScrolledToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < SCROLL_BOTTOM_THRESHOLD_PX;
  }, []);

  const loadOlderMessages = useCallback(async () => {
    if (isLoadingMore || !hasMore || messages.length === 0) return;
    setIsLoadingMore(true);

    try {
      const oldestTs = messages[0]?.created_at;
      const { messages: older, has_more } = await api.getChatMessages(agent.id, oldestTs, 50, identity.id);

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
  }, [agent.id, api, messages, isLoadingMore, hasMore, identity.id]);

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

      // Thinking process visualization
      if (event.data?.agent_id === agent.id || (event.data?.engine_id as string | undefined)?.startsWith('mind.')) {
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
      }

      // Command approval request from kernel (batch)
      if (event.type === 'CommandApprovalRequested' && event.data?.agent_id === agent.id) {
        const approvalData = event.data as {
          approval_id: string;
          agent_id: string;
          commands?: Array<{ command: string; command_name: string }>;
        };
        setPendingApprovals((prev) => {
          if (prev.some((a) => a.approval_id === approvalData.approval_id)) return prev;
          return [
            ...prev,
            {
              approval_id: approvalData.approval_id,
              agent_id: approvalData.agent_id,
              commands: approvalData.commands || [],
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

      if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
        setIsTyping(false);
        setThinkingSteps([]);
        const msgId = event.data.source_message_id + '-resp';
        const now = Date.now();
        const elapsedSecs = sendTimestampRef.current > 0 ? Math.round((now - sendTimestampRef.current) / 100) / 10 : 0;
        // Reset for next response in the agentic loop — measures per-message, not cumulative
        sendTimestampRef.current = now;

        // Use correct parent_id: retryParentIdRef during retry, otherwise SSE source
        const sourceId = event.data.source_message_id;
        const parentId = retryParentIdRef.current ?? sourceId;
        // Clear retry guard so recoverTypingState can resume
        retryParentIdRef.current = null;

        // If a previous typewriter is still running, finalize it immediately
        setPendingResponse((prev) => {
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
            id: msgId,
            text: event.data.content as string,
            elapsedSecs,
            parentId: parentId as string | undefined,
          };
        });

        // Agent response is persisted backend-side (system.rs) before SSE emission.
      }
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
        artifactPanel.addArtifact({ code, language, lineCount });
      }
    },
    [artifactPanel.addArtifact],
  );

  const sendMessage = async (blocks?: ContentBlock[], rawText?: string, engineOverride?: string | null) => {
    const text = rawText ?? '';
    const contentBlocks = blocks ?? (text.trim() ? [{ type: 'text' as const, text: text.trim() }] : []);
    if (contentBlocks.length === 0 || isTyping || pendingResponse) return;
    artifactPanel.clearArtifacts();

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

    // Extract text content for event bus (which expects a plain string)
    const textContent = contentBlocks
      .filter((b) => b.type === 'text')
      .map((b) => b.text || '')
      .join(' ');

    try {
      // If content blocks include media (image/audio), persist them via
      // postChatMessage first so the kernel can find attachments in DB
      // when running maybe_analyze_images / maybe_transcribe_audio.
      const hasMedia = contentBlocks.some((b) => b.type === 'image' || b.type === 'audio');
      if (hasMedia) {
        await api.postChatMessage(agent.id, {
          id: msgId,
          source: 'user',
          content: contentBlocks,
          metadata: {
            user_id: identity.id,
            user_name: identity.name,
            ...(engineOverride ? { engine_override: engineOverride } : {}),
          },
        });
      }

      const clotoMsg: ClotoMessage = {
        id: msgId,
        source: { type: 'User', id: identity.id, name: identity.name },
        target_agent: agent.id,
        content: textContent || '[attachment]',
        timestamp: new Date().toISOString(),
        metadata: {
          target_agent_id: agent.id,
          ...(engineOverride ? { engine_override: engineOverride } : {}),
          // Tell system.rs not to re-persist the user message (already saved above)
          ...(hasMedia ? { skip_user_persist: 'true' } : {}),
        },
      };

      await api.postChat(clotoMsg);
    } catch (err) {
      setMessages((prev) => prev.filter((m) => m.id !== msgId));
      setIsTyping(false);
      const errMsg = err instanceof Error ? err.message : 'Failed to send message';
      if (import.meta.env.DEV) console.error('Failed to send message:', errMsg);
      const errId = `err-${msgId}`;
      const errBubble: ChatMessage = {
        id: errId,
        agent_id: agent.id,
        user_id: identity.id,
        source: 'system',
        content: [{ type: 'text', text: `⚠ ${errMsg}` }],
        created_at: Date.now(),
      };
      setMessages((prev) => [...prev, errBubble]);
      setTimeout(() => setMessages((prev) => prev.filter((m) => m.id !== errId)), ERROR_DISPLAY_MS);
    }
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
    artifactPanel.clearArtifacts();

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
      if (import.meta.env.DEV) console.error('Failed to send edited message:', err);
    }
  };

  const handleChatSend = useCallback(
    (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => {
      if (editingMessage) {
        handleEditMessage(blocks, rawText, engineOverride);
      } else {
        sendMessage(blocks, rawText, engineOverride);
      }
    },
    [editingMessage, handleEditMessage, sendMessage],
  );

  // Retry handler: remove old response immediately, re-generate in place
  const handleRetry = async (agentResponseMsg: ChatMessage) => {
    if (isTyping || pendingResponse) return;

    // Find the user message this response was generated from
    const userMsgId = agentResponseMsg.parent_id ?? agentResponseMsg.id.replace(/-resp$/, '');

    // Store correct parent_id for the new response (also acts as retry-in-progress guard)
    retryParentIdRef.current = userMsgId;
    setMessages((prev) => prev.filter((m) => m.id !== agentResponseMsg.id));
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = Date.now();
    artifactPanel.clearArtifacts();

    try {
      await api.retryResponse(agent.id, userMsgId);
    } catch (err) {
      setMessages((prev) => [...prev, agentResponseMsg]);
      retryParentIdRef.current = null;
      setIsTyping(false);
      if (import.meta.env.DEV) console.error('Failed to retry response:', err);
    }
  };

  const handleReset = async () => {
    setMessages([]);
    setIsTyping(false);
    setPendingResponse(null);
    setHasMore(false);
    setActiveBranches({});
    setEditingMessage(null);
    initialLoadDone.current = false;
    artifactPanel.clearArtifacts();
    try {
      await api.deleteChatMessages(agent.id, identity.id);
    } catch (err) {
      if (import.meta.env.DEV) console.error('Failed to delete chat messages:', err);
    }
  };

  return (
    <div className="flex flex-col h-full animate-in fade-in duration-500">
      {/* Header */}
      <div className="p-4 border-b border-edge-subtle flex items-center justify-between bg-glass">
        <div className="flex items-center gap-3">
          <button
            onClick={onBack}
            className="p-2 rounded-full bg-glass-subtle border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={16} />
          </button>
          <div
            className="w-10 h-10 text-white rounded-md shadow-sm overflow-hidden flex items-center justify-center"
            style={{ backgroundColor: agentColor(agent) }}
          >
            <AgentIcon agent={agent} size={40} />
          </div>
          <div>
            <h2 className="text-xl font-black text-content-primary tracking-tighter uppercase">{agent.name}</h2>
            <div className="flex items-center gap-2">
              <StatusDot status={agent.enabled ? 'online' : 'offline'} size="sm" />
              <span className="text-[10px] font-mono text-content-tertiary uppercase tracking-[0.2em]">
                {agent.enabled ? t('console.connected') : t('console.offline')}
              </span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {hasVrm && (
            <button
              onClick={() => openVrmWindow(agent.id, api.apiKey)}
              className="px-3 py-1.5 rounded-full border border-edge text-[9px] font-bold uppercase tracking-widest flex items-center gap-1.5 transition-all text-content-tertiary hover:text-brand hover:border-brand/30"
              title="Open 3D Avatar Window"
            >
              <Box size={10} />
              <span>VRM</span>
            </button>
          )}
          <LongPressResetButton onReset={handleReset} />
        </div>
      </div>

      {/* Content area: chat + optional artifact panel */}
      <div className="flex flex-1 overflow-hidden">
        {/* Chat column */}
        <div className="flex flex-col flex-1 min-w-0">
          {/* Message Stream */}
          <div
            ref={scrollRef}
            onScroll={handleScroll}
            onClick={() => editingMessage && setEditingMessage(null)}
            className="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar"
          >
            {/* Sentinel for lazy loading older messages */}
            {hasMore && <div ref={sentinelRef} className="h-1" />}
            {isLoadingMore && (
              <div className="text-center text-[9px] font-mono text-content-tertiary py-2 animate-pulse">
                {t('console.loading_older')}
              </div>
            )}

            {isLoading ? (
              <div className="h-full flex flex-col items-center justify-center text-content-tertiary space-y-4">
                <Activity size={24} className="animate-pulse" />
                <p className="text-[10px] font-mono tracking-[0.2em] uppercase">{t('console.loading_session')}</p>
              </div>
            ) : displayMessages.length === 0 && !pendingResponse && !isTyping ? (
              <div className="h-full flex flex-col items-center justify-center text-content-tertiary space-y-4">
                <Zap size={32} strokeWidth={1} className="opacity-20" />
                <p className="text-[10px] font-mono tracking-[0.2em] uppercase">{t('console.ready')}</p>
              </div>
            ) : (
              displayMessages.map((msg) => {
                const isUser = msg.source === 'user';
                const firstText = Array.isArray(msg.content)
                  ? msg.content.find((b) => b.type === 'text')?.text || ''
                  : '';
                const isError = !isUser && firstText.startsWith('[Error]');
                // Check if this message's parent has branch siblings
                const branchKey = msg.parent_id ? msg.parent_id + ':' + msg.source : null;
                const branch = branchKey ? branchPoints.get(branchKey) : undefined;
                return (
                  <div key={msg.id}>
                    {isError ? (
                      <SystemAlertCard icon={<Activity size={14} />} title={t('console.engine_error')}>
                        <div className="text-xs text-content-secondary whitespace-pre-line">
                          {firstText.replace(/^\[Error\]\s*/, '')}
                        </div>
                      </SystemAlertCard>
                    ) : (
                      <div className={`group flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
                        <div
                          className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm overflow-hidden ${
                            isUser ? 'bg-surface-primary border border-edge-subtle text-content-tertiary' : 'text-white'
                          }`}
                          style={!isUser ? { backgroundColor: agentColor(agent) } : undefined}
                        >
                          {isUser ? <UserIcon size={14} /> : <AgentIcon agent={agent} size={32} />}
                        </div>
                        <div
                          className={`max-w-[80%] text-base leading-7 select-text ${
                            isUser
                              ? 'p-4 rounded-2xl rounded-tr-none shadow-sm bg-surface-primary text-content-primary'
                              : 'pt-1 text-content-primary'
                          }`}
                        >
                          <MessageContent content={msg.content} />
                          {!isUser && (
                            <div className="mt-2 flex items-center gap-2">
                              {msg.metadata?.elapsed_secs != null && (
                                <span className="text-xs font-mono text-content-tertiary">
                                  {String(msg.metadata.elapsed_secs)}s
                                </span>
                              )}
                              <button
                                onClick={() => speakText(msg.content as ContentBlock[])}
                                className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-brand transition-colors"
                                title={t('console.read_aloud')}
                              >
                                <Volume2 size={12} />
                              </button>
                              {!isTyping && !pendingResponse && (
                                <button
                                  onClick={() => handleRetry(msg)}
                                  className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-brand transition-colors"
                                  title={t('console.retry')}
                                >
                                  <RetryIcon size={12} />
                                </button>
                              )}
                            </div>
                          )}
                        </div>
                        {/* Edit button — outside bubble, to the left of user messages */}
                        {isUser && !isTyping && !pendingResponse && (
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              setEditingMessage(msg);
                            }}
                            className="self-start mt-1 p-1.5 rounded-full hover:bg-glass text-content-primary/40 hover:text-brand transition-all shrink-0"
                            title={t('console.edit_message')}
                          >
                            <Pencil size={13} />
                          </button>
                        )}
                      </div>
                    )}
                    {/* Branch navigator */}
                    {branch && (
                      <div className={`flex ${isUser ? 'justify-end mr-11' : 'ml-11'}`}>
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
                      </div>
                    )}
                  </div>
                );
              })
            )}
            {/* Typewriter animation for current response */}
            {pendingResponse && (
              <div className="flex items-start gap-3 message-enter">
                <div
                  className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm overflow-hidden"
                  style={{ backgroundColor: agentColor(agent) }}
                >
                  <AgentIcon agent={agent} size={32} />
                </div>
                <div className="max-w-[80%] pt-1 text-base leading-7 select-text text-content-primary">
                  <TypewriterMessage
                    text={pendingResponse.text}
                    onComplete={handleTypewriterComplete}
                    onCodeBlock={handleCodeBlockExtracted}
                  />
                  {pendingResponse.elapsedSecs > 0 && (
                    <div className="mt-1 text-[10px] font-mono text-content-tertiary">
                      {pendingResponse.elapsedSecs}s
                    </div>
                  )}
                </div>
              </div>
            )}
            {/* Thinking process steps (real-time tool invocations) */}
            {isTyping && thinkingSteps.length > 0 && (
              <div className="flex items-start gap-3">
                <div
                  className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm overflow-hidden opacity-40"
                  style={{ backgroundColor: agentColor(agent) }}
                >
                  <AgentIcon agent={agent} size={32} />
                </div>
                <div className="flex-1 space-y-0.5 py-1">
                  {thinkingSteps.map((step) => (
                    <div
                      key={step.id}
                      className={`flex items-center gap-2 text-[10px] font-mono animate-in fade-in duration-200 ${step.status === 'thought' ? 'italic' : ''}`}
                    >
                      <span
                        className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                          step.status === 'ok'
                            ? 'bg-brand'
                            : step.status === 'fail'
                              ? 'bg-red-500'
                              : step.status === 'done'
                                ? 'bg-emerald-500'
                                : step.status === 'thought'
                                  ? 'bg-blue-400 animate-pulse'
                                  : 'bg-amber-500 animate-pulse'
                        }`}
                      />
                      <span
                        className={`${step.status === 'fail' ? 'text-red-400' : step.status === 'thought' ? 'text-content-secondary' : 'text-content-tertiary'}`}
                      >
                        {step.text}
                      </span>
                      {step.detail && (
                        <span
                          className={`ml-auto ${step.status === 'fail' ? 'text-red-400/60' : 'text-content-tertiary'}`}
                        >
                          {step.detail}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
            {/* Command Approval Cards */}
            {pendingApprovals.map((approval) => (
              <CommandApprovalCard
                key={approval.approval_id}
                approvalId={approval.approval_id}
                commands={approval.commands}
                onResolved={(id) => setPendingApprovals((prev) => prev.filter((a) => a.approval_id !== id))}
              />
            ))}
            {/* Skeleton (waiting for SSE response) */}
            {isTyping && pendingApprovals.length === 0 && (
              <SkeletonThinking agentColor={agentColor(agent)} agentIcon={<AgentIcon agent={agent} size={32} />} />
            )}
          </div>

          {/* Input Area */}
          <ChatInputBar
            onSend={handleChatSend}
            disabled={isTyping || !!pendingResponse}
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
          />
        </div>
        {/* end chat column */}

        {/* Artifact Panel */}
        <ArtifactPanel
          artifacts={artifactPanel.artifacts}
          activeIndex={artifactPanel.activeIndex}
          onTabChange={artifactPanel.setActiveIndex}
          isOpen={artifactPanel.isOpen}
          onClose={artifactPanel.closePanel}
          onOpen={artifactPanel.openPanel}
        />
      </div>
      {/* end content area */}
    </div>
  );
}
