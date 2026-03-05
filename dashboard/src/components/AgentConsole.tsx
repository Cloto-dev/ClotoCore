import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Activity, Zap, User as UserIcon, RotateCcw, ArrowLeft, Volume2, Pencil, RotateCcw as RetryIcon } from 'lucide-react';
import { AgentMetadata, ClotoMessage, ChatMessage, ContentBlock, CommandApprovalRequest, McpServerInfo } from '../types';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { useLongPress } from '../hooks/useLongPress';
import { MessageContent } from './ContentBlockView';
import { ChatInputBar } from './ChatInputBar';
import { CommandApprovalCard } from './CommandApprovalCard';
import { SystemAlertCard } from './SystemAlertCard';
import { BranchNavigator } from './BranchNavigator';
import { api, EVENTS_URL } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';
import { useMcpServers } from '../hooks/useMcpServers';
import { SkeletonThinking } from './SkeletonThinking';
import { TypewriterMessage } from './TypewriterMessage';
import { ArtifactPanel } from './ArtifactPanel';
import { useArtifacts } from '../hooks/useArtifacts';
import { sendNativeNotification } from '../lib/notifications';
import { flattenConversation, findBranchPoints } from '../lib/conversationTree';

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'cloto-chat-';

function LongPressResetButton({ onReset }: { onReset: () => void }) {
  const { progress, handlers } = useLongPress(1500, onReset);

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
      <span className="relative">{progress > 0 ? 'Hold...' : 'Reset'}</span>
    </button>
  );
}

/** Migrate legacy localStorage session data to server */
async function migrateLegacyData(agentId: string, apiKey: string) {
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
      await api.postChatMessage(agentId, {
        id: msg.id,
        source,
        content: [{ type: 'text', text: msg.content }],
      }, apiKey).catch(() => {}); // Ignore duplicate ID errors
    }

    // Remove legacy data
    localStorage.removeItem(key);
    console.log(`Migrated ${legacyMessages.length} legacy messages for agent ${agentId}`);
  } catch {
    // Silently ignore migration errors
  }
}

export function AgentConsole({ agent, onBack }: { agent: AgentMetadata, onBack: () => void }) {
  const { apiKey } = useApiKey();
  const { servers: mcpServers } = useMcpServers(apiKey);
  const [agentEngines, setAgentEngines] = useState<McpServerInfo[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isTyping, setIsTyping] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasMore, setHasMore] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [pendingResponse, setPendingResponse] = useState<{ id: string; text: string; elapsedSecs: number; parentId?: string } | null>(null);
  const [thinkingSteps, setThinkingSteps] = useState<Array<{ id: number; status: 'ok' | 'fail' | 'done' | 'thought'; text: string; detail?: string; ts: number }>>([]);
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

  // Flatten branching conversation to linear display
  const displayMessages = useMemo(
    () => flattenConversation(messages, activeBranches),
    [messages, activeBranches],
  );
  const branchPoints = useMemo(
    () => findBranchPoints(messages, activeBranches),
    [messages, activeBranches],
  );

  // Resolve agent's granted mind.* servers for engine selector
  useEffect(() => {
    api.getAgentAccess(agent.id).then(({ entries }) => {
      const grantedMindIds = new Set(
        entries
          .filter(e => e.entry_type === 'server_grant' && e.permission === 'allow' && e.server_id.startsWith('mind.'))
          .map(e => e.server_id)
      );
      setAgentEngines(mcpServers.filter(s => grantedMindIds.has(s.id)));
    }).catch(() => {});
  }, [agent.id, mcpServers]);

  // Load initial messages from server
  useEffect(() => {
    if (initialLoadDone.current) return;
    initialLoadDone.current = true;

    const loadMessages = async () => {
      try {
        // First, check for legacy localStorage data and migrate
        await migrateLegacyData(agent.id, apiKey);

        const { messages: loaded, has_more } = await api.getChatMessages(agent.id, apiKey, undefined, 50);
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
        console.error('Failed to load chat messages:', err);
      } finally {
        setIsLoading(false);
      }
    };
    loadMessages();
  }, [agent.id, apiKey]);

  // Recovery: if isTyping is true but we missed the SSE response,
  // re-check the server for messages. Triggers on:
  // 1. 30s timeout while typing
  // 2. Page becoming visible again (user navigated away and back)
  const recoverTypingState = useCallback(async () => {
    if (!isTyping || retryParentIdRef.current) return;
    try {
      const { messages: latest } = await api.getChatMessages(agent.id, apiKey, undefined, 5);
      if (latest.length > 0 && latest[0].source === 'agent') {
        const reversed = latest.reverse();
        setMessages(prev => {
          const existingIds = new Set(prev.map(m => m.id));
          const newMsgs = reversed.filter(m => !existingIds.has(m.id));
          return newMsgs.length > 0 ? [...prev, ...newMsgs] : prev;
        });
        setIsTyping(false);
        setThinkingSteps([]);
      }
    } catch {
      // Silently ignore — next event or timeout will retry
    }
  }, [isTyping, agent.id, apiKey]);

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
  }, [messages.length, isLoading, pendingResponse]);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    isScrolledToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 50;
  }, []);

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
      { root: scrollRef.current, threshold: 0.1 }
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, isLoading, isLoadingMore, messages]);

  const loadOlderMessages = useCallback(async () => {
    if (isLoadingMore || !hasMore || messages.length === 0) return;
    setIsLoadingMore(true);

    try {
      const oldestTs = messages[0]?.created_at;
      const { messages: older, has_more } = await api.getChatMessages(agent.id, apiKey, oldestTs, 50);

      if (older.length > 0) {
        // Preserve scroll position
        const scrollEl = scrollRef.current;
        const prevHeight = scrollEl?.scrollHeight || 0;

        setMessages(prev => [...older.reverse(), ...prev]);
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
      console.error('Failed to load older messages:', err);
    } finally {
      setIsLoadingMore(false);
    }
  }, [agent.id, apiKey, messages, isLoadingMore, hasMore]);

  // Subscribe to system-wide events
  useEventStream(EVENTS_URL, (event) => {
    // Thinking process visualization
    if (event.data?.agent_id === agent.id || event.data?.engine_id?.startsWith('mind.')) {
      if (event.type === 'ToolInvoked' && event.data.agent_id === agent.id) {
        const tool = event.data.tool_name || 'unknown';
        setThinkingSteps(prev => [...prev, {
          id: thinkingIdRef.current++,
          status: event.data.success ? 'ok' : 'fail',
          text: tool,
          detail: event.data.success ? `${event.data.duration_ms}ms` : 'failed',
          ts: Date.now(),
        }]);
      }
      if (event.type === 'AgenticLoopCompleted' && event.data.agent_id === agent.id) {
        setThinkingSteps(prev => [...prev, {
          id: thinkingIdRef.current++,
          status: 'done',
          text: 'complete',
          detail: `${event.data.total_iterations || 0} iter, ${event.data.total_tool_calls || 0} calls`,
          ts: Date.now(),
        }]);
      }
      if (event.type === 'Thought' && event.payload?.content) {
        setThinkingSteps(prev => [...prev, {
          id: thinkingIdRef.current++,
          status: 'thought',
          text: event.payload.content.slice(0, 120),
          ts: Date.now(),
        }]);
      }
    }

    // Command approval request from kernel (batch)
    if (event.type === 'CommandApprovalRequested' && event.data?.agent_id === agent.id) {
      setPendingApprovals(prev => {
        if (prev.some(a => a.approval_id === event.data.approval_id)) return prev;
        return [...prev, {
          approval_id: event.data.approval_id,
          agent_id: event.data.agent_id,
          commands: event.data.commands || [],
        }];
      });
      if (document.hidden) {
        const count = event.data.commands?.length || 1;
        sendNativeNotification('Command Approval', `${agent.name}: ${count} command(s) pending`);
      }
    }
    if (event.type === 'CommandApprovalResult') {
      setPendingApprovals(prev => prev.filter(a => a.approval_id !== event.data.approval_id));
    }

    if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
      setIsTyping(false);
      setThinkingSteps([]);
      const msgId = event.data.source_message_id + "-resp";
      const now = Date.now();
      const elapsedSecs = sendTimestampRef.current > 0
        ? Math.round((now - sendTimestampRef.current) / 100) / 10
        : 0;
      // Reset for next response in the agentic loop — measures per-message, not cumulative
      sendTimestampRef.current = now;

      // Use correct parent_id: retryParentIdRef during retry, otherwise SSE source
      const sourceId = event.data.source_message_id;
      const parentId = retryParentIdRef.current ?? sourceId;
      // Clear retry guard so recoverTypingState can resume
      retryParentIdRef.current = null;

      // If a previous typewriter is still running, finalize it immediately
      setPendingResponse(prev => {
        if (prev) {
          const prevMsg: ChatMessage = {
            id: prev.id, agent_id: agent.id, user_id: 'default',
            source: 'agent',
            content: [{ type: 'text', text: prev.text }],
            metadata: { elapsed_secs: prev.elapsedSecs },
            created_at: Date.now(),
            parent_id: prev.parentId,
          };
          setMessages(msgs => [...msgs, prevMsg]);
        }
        return { id: msgId, text: event.data.content, elapsedSecs, parentId };
      });

      // Agent response is persisted backend-side (system.rs) before SSE emission.
    }
  });

  // Typewriter completion: move pending response to static messages
  const handleTypewriterComplete = useCallback(() => {
    setPendingResponse(prev => {
      if (!prev) return null;
      const agentMsg: ChatMessage = {
        id: prev.id, agent_id: agent.id, user_id: 'default',
        source: 'agent',
        content: [{ type: 'text', text: prev.text }],
        metadata: { elapsed_secs: prev.elapsedSecs },
        created_at: Date.now(),
        parent_id: prev.parentId,
      };
      setMessages(msgs => [...msgs, agentMsg]);
      return null;
    });
  }, [agent.id]);

  const handleCodeBlockExtracted = useCallback((code: string, language: string, lineCount: number) => {
    if (lineCount >= 15) {
      artifactPanel.addArtifact({ code, language, lineCount });
    }
  }, [artifactPanel.addArtifact]);

  const sendMessage = async (blocks?: ContentBlock[], rawText?: string, engineOverride?: string | null) => {
    const text = rawText ?? '';
    const contentBlocks = blocks ?? (text.trim() ? [{ type: 'text' as const, text: text.trim() }] : []);
    if (contentBlocks.length === 0 || isTyping || pendingResponse) return;
    artifactPanel.clearArtifacts();

    const msgId = Date.now().toString();
    const userMsg: ChatMessage = {
      id: msgId,
      agent_id: agent.id,
      user_id: 'default',
      source: 'user',
      content: contentBlocks,
      created_at: Date.now(),
    };

    setMessages(prev => [...prev, userMsg]);
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = Date.now();

    // Extract text content for event bus (which expects a plain string)
    const textContent = contentBlocks
      .filter(b => b.type === 'text')
      .map(b => b.text || '')
      .join(' ');

    try {
      // User message is persisted backend-side (system.rs) on receipt.
      const clotoMsg: ClotoMessage = {
        id: msgId,
        source: { type: 'User', id: 'user', name: 'User' },
        target_agent: agent.id,
        content: textContent || '[attachment]',
        timestamp: new Date().toISOString(),
        metadata: {
          target_agent_id: agent.id,
          ...(engineOverride ? { engine_override: engineOverride } : {}),
        }
      };

      await api.postChat(clotoMsg, apiKey);
    } catch (err) {
      setMessages(prev => prev.filter(m => m.id !== msgId));
      setIsTyping(false);
      const errMsg = err instanceof Error ? err.message : 'Failed to send message';
      console.error("Failed to send message:", errMsg);
      const errId = `err-${msgId}`;
      const errBubble: ChatMessage = {
        id: errId,
        agent_id: agent.id,
        user_id: 'default',
        source: 'system',
        content: [{ type: 'text', text: `⚠ ${errMsg}` }],
        created_at: Date.now(),
      };
      setMessages(prev => [...prev, errBubble]);
      setTimeout(() => setMessages(prev => prev.filter(m => m.id !== errId)), 5000);
    }
  };

  const speakText = async (content: ContentBlock[]) => {
    const text = content.filter(b => b.type === 'text').map(b => b.text || '').join(' ');
    if (!text.trim()) return;
    try {
      const clotoMsg: ClotoMessage = {
        id: Date.now().toString(),
        source: { type: 'User', id: 'system', name: 'System' },
        target_agent: agent.id,
        content: `Use the speak tool to say aloud: ${text}`,
        timestamp: new Date().toISOString(),
        metadata: { target_agent_id: agent.id, tool_hint: 'speak', skip_user_persist: 'true' }
      };
      await api.postChat(clotoMsg, apiKey);
    } catch (err) {
      console.error('TTS request failed:', err);
    }
  };

  // Edit handler: resend edited user message as a new branch
  const handleEditMessage = async (blocks: ContentBlock[], rawText: string, engineOverride: string | null) => {
    if (!editingMessage || isTyping || pendingResponse) return;

    const text = rawText?.trim() || '';
    const contentBlocks = blocks?.length ? blocks : (text ? [{ type: 'text' as const, text }] : []);
    if (contentBlocks.length === 0) return;

    const now = Date.now();
    const editId = `edit-${now}`;
    const parentId = editingMessage.parent_id ?? undefined;

    // Count existing siblings to determine branch_index
    const siblingCount = messages.filter(
      m => m.parent_id === parentId && m.source === 'user'
    ).length;

    const userMsg: ChatMessage = {
      id: editId,
      agent_id: agent.id,
      user_id: 'default',
      source: 'user',
      content: contentBlocks,
      created_at: now,
      parent_id: parentId,
      branch_index: siblingCount,
    };

    setMessages(prev => [...prev, userMsg]);
    setEditingMessage(null);
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = now;
    artifactPanel.clearArtifacts();

    // Update active branch to show the new edit
    if (parentId) {
      setActiveBranches(prev => ({ ...prev, [parentId + ':user']: siblingCount }));
    }

    const textContent = contentBlocks
      .filter(b => b.type === 'text')
      .map(b => b.text || '')
      .join(' ');

    try {
      const clotoMsg: ClotoMessage = {
        id: editId,
        source: { type: 'User', id: 'user', name: 'User' },
        target_agent: agent.id,
        content: textContent || '[attachment]',
        timestamp: new Date().toISOString(),
        metadata: {
          target_agent_id: agent.id,
          ...(parentId ? { parent_id: parentId, branch_index: String(siblingCount) } : {}),
          ...(engineOverride ? { engine_override: engineOverride } : {}),
        }
      };
      await api.postChat(clotoMsg, apiKey);
    } catch (err) {
      setMessages(prev => prev.filter(m => m.id !== editId));
      setIsTyping(false);
      console.error('Failed to send edited message:', err);
    }
  };

  // Retry handler: remove old response immediately, re-generate in place
  const handleRetry = async (agentResponseMsg: ChatMessage) => {
    if (isTyping || pendingResponse) return;

    // Find the user message this response was generated from
    const userMsgId = agentResponseMsg.parent_id
      ?? agentResponseMsg.id.replace(/-resp$/, '');

    // Store correct parent_id for the new response (also acts as retry-in-progress guard)
    retryParentIdRef.current = userMsgId;
    setMessages(prev => prev.filter(m => m.id !== agentResponseMsg.id));
    setIsTyping(true);
    setThinkingSteps([]);
    sendTimestampRef.current = Date.now();
    artifactPanel.clearArtifacts();

    try {
      await api.retryResponse(agent.id, userMsgId, apiKey);
    } catch (err) {
      setMessages(prev => [...prev, agentResponseMsg]);
      retryParentIdRef.current = null;
      setIsTyping(false);
      console.error('Failed to retry response:', err);
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
      await api.deleteChatMessages(agent.id, apiKey);
    } catch (err) {
      console.error('Failed to delete chat messages:', err);
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
          <div className="w-10 h-10 text-white rounded-md shadow-sm overflow-hidden flex items-center justify-center" style={{ backgroundColor: agentColor(agent) }}>
            <AgentIcon agent={agent} size={40} />
          </div>
          <div>
            <h2 className="text-xl font-black text-content-primary tracking-tighter uppercase">{agent.name}</h2>
            <div className="flex items-center gap-2">
              <span className="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse" />
              <span className="text-[10px] font-mono text-content-tertiary uppercase tracking-[0.2em]">Connected</span>
            </div>
          </div>
        </div>
        <LongPressResetButton onReset={handleReset} />
      </div>

      {/* Content area: chat + optional artifact panel */}
      <div className="flex flex-1 overflow-hidden">
      {/* Chat column */}
      <div className="flex flex-col flex-1 min-w-0">
      {/* Message Stream */}
      <div ref={scrollRef} onScroll={handleScroll} onClick={() => editingMessage && setEditingMessage(null)} className="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar">
        {/* Sentinel for lazy loading older messages */}
        {hasMore && <div ref={sentinelRef} className="h-1" />}
        {isLoadingMore && (
          <div className="text-center text-[9px] font-mono text-content-muted py-2 animate-pulse">
            Loading older messages...
          </div>
        )}

        {isLoading ? (
          <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
            <Activity size={24} className="animate-pulse" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Loading session...</p>
          </div>
        ) : displayMessages.length === 0 && !pendingResponse && !isTyping ? (
          <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
            <Zap size={32} strokeWidth={1} className="opacity-20" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Ready for instructions</p>
          </div>
        ) : (
          displayMessages.map((msg) => {
            const isUser = msg.source === 'user';
            const firstText = Array.isArray(msg.content) ? msg.content.find(b => b.type === 'text')?.text || '' : '';
            const isError = !isUser && firstText.startsWith('[Error]');
            // Check if this message's parent has branch siblings
            const branchKey = msg.parent_id ? msg.parent_id + ':' + msg.source : null;
            const branch = branchKey ? branchPoints.get(branchKey) : undefined;
            return (
              <div key={msg.id}>
                {isError ? (
                  <SystemAlertCard
                    icon={<Activity size={14} />}
                    title="Engine Error"
                  >
                    <div className="text-xs text-content-secondary whitespace-pre-line">{firstText.replace(/^\[Error\]\s*/, '')}</div>
                  </SystemAlertCard>
                ) : (
                <div className={`group flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
                  <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm overflow-hidden ${
                    isUser ? 'bg-surface-primary border border-edge-subtle text-content-tertiary'
                    : 'text-white'
                  }`} style={!isUser ? { backgroundColor: agentColor(agent) } : undefined}>
                    {isUser ? <UserIcon size={14} />
                     : <AgentIcon agent={agent} size={32} />}
                  </div>
                  <div className={`max-w-[80%] text-base leading-7 select-text ${
                    isUser
                      ? 'p-4 rounded-2xl rounded-tr-none shadow-sm bg-surface-primary text-content-primary'
                      : 'pt-1 text-content-primary'
                  }`}>
                    <MessageContent content={msg.content} />
                    {!isUser && (
                      <div className="mt-2 flex items-center gap-2">
                        {msg.metadata?.elapsed_secs != null && (
                          <span className="text-xs font-mono text-content-tertiary">
                            {msg.metadata.elapsed_secs}s
                          </span>
                        )}
                        <button
                          onClick={() => speakText(msg.content as ContentBlock[])}
                          className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-brand transition-colors"
                          title="Read aloud"
                        >
                          <Volume2 size={12} />
                        </button>
                        {!isTyping && !pendingResponse && (
                          <button
                            onClick={() => handleRetry(msg)}
                            className="p-1 rounded hover:bg-glass text-content-tertiary hover:text-brand transition-colors"
                            title="Retry response"
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
                      onClick={(e) => { e.stopPropagation(); setEditingMessage(msg); }}
                      className="self-start mt-1 p-1.5 rounded-full hover:bg-glass text-content-primary/40 hover:text-brand transition-all shrink-0"
                      title="Edit message"
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
                          setActiveBranches(prev => ({ ...prev, [branchKey]: idx }));
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
            <div className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm overflow-hidden"
                 style={{ backgroundColor: agentColor(agent) }}>
              <AgentIcon agent={agent} size={32} />
            </div>
            <div className="max-w-[80%] pt-1 text-base leading-7 select-text text-content-primary">
              <TypewriterMessage
                text={pendingResponse.text}
                onComplete={handleTypewriterComplete}
                onCodeBlock={handleCodeBlockExtracted}
              />
              {pendingResponse.elapsedSecs > 0 && (
                <div className="mt-1 text-[10px] font-mono text-content-muted">
                  {pendingResponse.elapsedSecs}s
                </div>
              )}
            </div>
          </div>
        )}
        {/* Thinking process steps (real-time tool invocations) */}
        {isTyping && thinkingSteps.length > 0 && (
          <div className="flex items-start gap-3">
            <div className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm overflow-hidden opacity-40"
                 style={{ backgroundColor: agentColor(agent) }}>
              <AgentIcon agent={agent} size={32} />
            </div>
            <div className="flex-1 space-y-0.5 py-1">
              {thinkingSteps.map(step => (
                <div key={step.id} className="flex items-center gap-2 text-[10px] font-mono animate-in fade-in duration-200">
                  <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                    step.status === 'ok' ? 'bg-brand'
                    : step.status === 'fail' ? 'bg-red-500'
                    : step.status === 'done' ? 'bg-emerald-500'
                    : 'bg-amber-500 animate-pulse'
                  }`} />
                  <span className={`${step.status === 'fail' ? 'text-red-400' : 'text-content-tertiary'}`}>
                    {step.text}
                  </span>
                  {step.detail && (
                    <span className={`ml-auto ${step.status === 'fail' ? 'text-red-400/60' : 'text-content-muted'}`}>
                      {step.detail}
                    </span>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}
        {/* Command Approval Cards */}
        {pendingApprovals.map(approval => (
          <CommandApprovalCard
            key={approval.approval_id}
            approvalId={approval.approval_id}
            commands={approval.commands}
            onResolved={(id) => setPendingApprovals(prev => prev.filter(a => a.approval_id !== id))}
          />
        ))}
        {/* Skeleton (waiting for SSE response) */}
        {isTyping && pendingApprovals.length === 0 && (
          <SkeletonThinking
            agentColor={agentColor(agent)}
            agentIcon={<AgentIcon agent={agent} size={32} />}
          />
        )}
      </div>

      {/* Input Area */}
      <ChatInputBar
        onSend={(blocks, rawText, engineOverride) => {
          if (editingMessage) {
            handleEditMessage(blocks, rawText, engineOverride);
          } else {
            sendMessage(blocks, rawText, engineOverride);
          }
        }}
        disabled={isTyping || !!pendingResponse}
        servers={agentEngines}
        editMode={editingMessage ? {
          messageId: editingMessage.id,
          initialContent: Array.isArray(editingMessage.content)
            ? editingMessage.content.filter(b => b.type === 'text').map(b => b.text || '').join(' ')
            : '',
          onCancel: () => setEditingMessage(null),
        } : null}
      />
      </div>{/* end chat column */}

      {/* Artifact Panel */}
      <ArtifactPanel
        artifacts={artifactPanel.artifacts}
        activeIndex={artifactPanel.activeIndex}
        onTabChange={artifactPanel.setActiveIndex}
        isOpen={artifactPanel.isOpen}
        onClose={artifactPanel.closePanel}
      />
      </div>{/* end content area */}
    </div>
  );
}
