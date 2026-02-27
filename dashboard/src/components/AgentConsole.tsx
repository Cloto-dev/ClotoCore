import { useState, useEffect, useRef, useCallback } from 'react';
import { Activity, Send, Zap, User as UserIcon, RotateCcw, ArrowLeft } from 'lucide-react';
import { AgentMetadata, ClotoMessage, ChatMessage } from '../types';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor } from '../lib/agentIdentity';
import { useLongPress } from '../hooks/useLongPress';
import { MessageContent } from './ContentBlockView';
import { api, EVENTS_URL } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';
import { SkeletonThinking } from './SkeletonThinking';
import { TypewriterMessage } from './TypewriterMessage';
import { ArtifactPanel } from './ArtifactPanel';
import { useArtifacts } from '../hooks/useArtifacts';

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'cloto-chat-';

function LongPressResetButton({ onReset }: { onReset: () => void }) {
  const { progress, handlers } = useLongPress(2000, onReset);

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
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [isTyping, setIsTyping] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasMore, setHasMore] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [pendingResponse, setPendingResponse] = useState<{ id: string; text: string; elapsedSecs: number } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const initialLoadDone = useRef(false);
  const isScrolledToBottom = useRef(true);
  const sendTimestampRef = useRef<number>(0);
  const artifactPanel = useArtifacts();

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
        // Restore typing state: if last message is from user, agent is likely still processing
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
  }, [agent.id, messages, isLoadingMore, hasMore]);

  // Subscribe to system-wide events
  useEventStream(EVENTS_URL, (event) => {
    if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
      setIsTyping(false);
      const msgId = event.data.source_message_id + "-resp";
      const elapsedSecs = sendTimestampRef.current > 0
        ? Math.round((Date.now() - sendTimestampRef.current) / 100) / 10
        : 0;

      // If a previous typewriter is still running, finalize it immediately
      setPendingResponse(prev => {
        if (prev) {
          const prevMsg: ChatMessage = {
            id: prev.id, agent_id: agent.id, user_id: 'default',
            source: 'agent',
            content: [{ type: 'text', text: prev.text }],
            metadata: { elapsed_secs: prev.elapsedSecs },
            created_at: Date.now(),
          };
          setMessages(msgs => [...msgs, prevMsg]);
        }
        return { id: msgId, text: event.data.content, elapsedSecs };
      });

      // Persist agent response to server (fire-and-forget)
      api.postChatMessage(agent.id, {
        id: msgId,
        source: 'agent',
        content: [{ type: 'text', text: event.data.content }],
      }, apiKey).catch(err => console.error('Failed to persist agent response:', err));
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

  const sendMessage = async () => {
    if (!input.trim() || isTyping || pendingResponse) return;
    artifactPanel.clearArtifacts();

    const msgId = Date.now().toString();
    const userMsg: ChatMessage = {
      id: msgId,
      agent_id: agent.id,
      user_id: 'default',
      source: 'user',
      content: [{ type: 'text', text: input }],
      created_at: Date.now(),
    };

    setMessages(prev => [...prev, userMsg]);
    setInput('');
    setIsTyping(true);
    sendTimestampRef.current = Date.now();

    try {
      // Persist user message first — cancel send if this fails
      await api.postChatMessage(agent.id, {
        id: userMsg.id,
        source: 'user',
        content: userMsg.content,
      }, apiKey);

      // Send to event bus for agent processing
      const clotoMsg: ClotoMessage = {
        id: msgId,
        source: { type: 'User', id: 'user', name: 'User' },
        target_agent: agent.id,
        content: input,
        timestamp: new Date().toISOString(),
        metadata: { target_agent_id: agent.id }
      };

      await api.postChat(clotoMsg, apiKey);
    } catch (err) {
      // Rollback: remove the user message from UI and show error
      setMessages(prev => prev.filter(m => m.id !== msgId));
      setInput(input); // Restore input so user can retry
      setIsTyping(false);
      const errMsg = err instanceof Error ? err.message : 'Failed to send message';
      console.error("Failed to send message:", errMsg);
      // Show transient error in UI
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

  const handleReset = async () => {
    setMessages([]);
    setIsTyping(false);
    setPendingResponse(null);
    setHasMore(false);
    initialLoadDone.current = false;
    artifactPanel.clearArtifacts();
    try {
      await api.deleteChatMessages(agent.id, apiKey);
    } catch (err) {
      console.error('Failed to delete chat messages:', err);
    }
  };

  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Header */}
      <div className="p-4 border-b border-edge-subtle flex items-center justify-between bg-glass-strong">
        <div className="flex items-center gap-3">
          <button
            onClick={onBack}
            className="p-2 rounded-full bg-glass-subtle border border-edge hover:border-brand hover:text-brand transition-all"
          >
            <ArrowLeft size={16} />
          </button>
          <div className="p-2 text-white rounded-md shadow-sm" style={{ backgroundColor: agentColor(agent) }}>
            <AgentIcon agent={agent} size={18} />
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
      <div ref={scrollRef} onScroll={handleScroll} className="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar">
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
        ) : messages.length === 0 && !pendingResponse && !isTyping ? (
          <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
            <Zap size={32} strokeWidth={1} className="opacity-20" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Ready for instructions</p>
          </div>
        ) : (
          messages.map((msg) => {
            const isUser = msg.source === 'user';
            return (
              <div key={msg.id} className={`flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
                <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm ${
                  isUser ? 'bg-surface-primary border border-edge-subtle text-content-tertiary' : 'text-white'
                }`} style={!isUser ? { backgroundColor: agentColor(agent) } : undefined}>
                  {isUser ? <UserIcon size={14} /> : <AgentIcon agent={agent} size={14} />}
                </div>
                <div className={`max-w-[80%] text-base leading-7 select-text ${
                  isUser
                    ? 'p-4 rounded-2xl rounded-tr-none shadow-sm bg-surface-primary text-content-primary'
                    : 'pt-1 text-content-primary'
                }`}>
                  <MessageContent content={msg.content} />
                  {!isUser && msg.metadata?.elapsed_secs != null && (
                    <div className="mt-2 text-xs font-mono text-content-tertiary">
                      {msg.metadata.elapsed_secs}s
                    </div>
                  )}
                </div>
              </div>
            );
          })
        )}
        {/* Typewriter animation for current response */}
        {pendingResponse && (
          <div className="flex items-start gap-3 message-enter">
            <div className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0 shadow-sm"
                 style={{ backgroundColor: agentColor(agent) }}>
              <AgentIcon agent={agent} size={14} />
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
        {/* Skeleton (waiting for SSE response) */}
        {isTyping && (
          <SkeletonThinking
            agentColor={agentColor(agent)}
            agentIcon={<AgentIcon agent={agent} size={14} />}
          />
        )}
      </div>

      {/* Input Area */}
      <div className="p-4 bg-glass-strong border-t border-edge-subtle">
        <div className="relative flex items-center">
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && sendMessage()}
            disabled={isTyping || !!pendingResponse}
            placeholder={isTyping || pendingResponse ? "PROCESSING..." : "ENTER COMMAND..."}
            className="w-full bg-surface-primary border border-edge rounded-xl py-3 px-4 pr-12 text-xs font-mono focus:outline-none focus:border-brand transition-colors placeholder:text-content-muted disabled:opacity-50 shadow-inner"
          />
          <button
            onClick={sendMessage}
            disabled={isTyping || !!pendingResponse || !input.trim()}
            className="absolute right-2 p-2 bg-brand text-white rounded-lg hover:scale-105 active:scale-95 transition-all disabled:opacity-30 disabled:grayscale disabled:scale-100 shadow-lg shadow-brand/20"
          >
            <Send size={16} />
          </button>
        </div>
      </div>
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
