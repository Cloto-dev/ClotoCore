import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Cpu, ChevronRight, Puzzle, Activity, MessageSquare, Send, Zap, User as UserIcon, RotateCcw } from 'lucide-react';
import { AgentMetadata, PluginManifest, ExivMessage, ChatMessage, ContentBlock } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';

import { api, API_BASE } from '../services/api';

export interface AgentTerminalProps {
  agents?: AgentMetadata[];
  plugins?: PluginManifest[];
  selectedAgent?: AgentMetadata | null;
  onSelectAgent?: (agent: AgentMetadata | null) => void;
}

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'exiv-chat-';

function LongPressResetButton({ onReset }: { onReset: () => void }) {
  const [progress, setProgress] = useState(0);
  const rafRef = useRef<number>(0);
  const startRef = useRef(0);

  const start = () => {
    startRef.current = Date.now();
    const tick = () => {
      const elapsed = Date.now() - startRef.current;
      const p = Math.min(elapsed / 2000, 1);
      setProgress(p);
      if (p >= 1) {
        onReset();
        setProgress(0);
        return;
      }
      rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
  };

  const cancel = () => {
    cancelAnimationFrame(rafRef.current);
    setProgress(0);
  };

  useEffect(() => () => cancelAnimationFrame(rafRef.current), []);

  return (
    <button
      onMouseDown={start}
      onMouseUp={cancel}
      onMouseLeave={cancel}
      onTouchStart={start}
      onTouchEnd={cancel}
      className="relative px-3 py-1.5 rounded-full border border-slate-200 text-[9px] font-bold text-slate-400 hover:text-amber-500 hover:border-amber-400/30 transition-all uppercase tracking-widest flex items-center gap-1.5 overflow-hidden"
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

/** Render a single ContentBlock */
function ContentBlockView({ block }: { block: ContentBlock }) {
  switch (block.type) {
    case 'text':
      return <span>{block.text}</span>;
    case 'image':
      return (
        <img
          src={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          alt={block.filename || 'image'}
          className="max-w-full rounded-lg mt-1"
          loading="lazy"
        />
      );
    case 'code':
      return (
        <pre className="bg-black/10 rounded-lg p-2 mt-1 overflow-x-auto text-[10px] font-mono">
          <code>{block.text}</code>
        </pre>
      );
    case 'tool_result':
      return (
        <div className="bg-black/10 rounded-lg p-2 mt-1 text-[10px] font-mono border-l-2 border-emerald-400">
          {block.text}
        </div>
      );
    case 'file':
      return (
        <a
          href={block.attachment_id ? api.getAttachmentUrl(block.attachment_id) : block.url}
          download={block.filename}
          className="inline-flex items-center gap-1 underline text-[10px] mt-1"
        >
          {block.filename || 'Download'}
        </a>
      );
    default:
      return <span>{block.text || ''}</span>;
  }
}

/** Render message content (supports both string and ContentBlock[]) */
function MessageContent({ content }: { content: string | ContentBlock[] }) {
  if (typeof content === 'string') {
    return <span>{content}</span>;
  }
  return (
    <>
      {content.map((block, i) => (
        <ContentBlockView key={i} block={block} />
      ))}
    </>
  );
}

function AgentConsole({ agent, onBack }: { agent: AgentMetadata, onBack: () => void }) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [isTyping, setIsTyping] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [hasMore, setHasMore] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const initialLoadDone = useRef(false);

  // Load initial messages from server
  useEffect(() => {
    if (initialLoadDone.current) return;
    initialLoadDone.current = true;

    const loadMessages = async () => {
      try {
        // First, check for legacy localStorage data and migrate
        await migrateLegacyData(agent.id);

        const { messages: loaded, has_more } = await api.getChatMessages(agent.id, undefined, 50);
        // API returns newest-first; reverse for display (oldest at top)
        setMessages(loaded.reverse());
        setHasMore(has_more);
      } catch (err) {
        console.error('Failed to load chat messages:', err);
      } finally {
        setIsLoading(false);
      }
    };
    loadMessages();
  }, [agent.id]);

  // Scroll to bottom on initial load and new messages
  useEffect(() => {
    if (!isLoading && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages.length, isLoading]);

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
      const { messages: older, has_more } = await api.getChatMessages(agent.id, oldestTs, 50);

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
  useEventStream(`${API_BASE}/events`, (event) => {
    if (event.type === 'ThoughtResponse' && event.data.agent_id === agent.id) {
      setIsTyping(false);
      const agentMsg: ChatMessage = {
        id: event.data.source_message_id + "-resp",
        agent_id: agent.id,
        user_id: 'default',
        source: 'agent',
        content: [{ type: 'text', text: event.data.content }],
        created_at: Date.now(),
      };
      setMessages(prev => [...prev, agentMsg]);

      // Persist agent response to server (fire-and-forget)
      api.postChatMessage(agent.id, {
        id: agentMsg.id,
        source: 'agent',
        content: agentMsg.content,
      }).catch(err => console.error('Failed to persist agent response:', err));
    }
  });

  const sendMessage = async () => {
    if (!input.trim() || isTyping) return;

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

    try {
      // Persist user message to server
      api.postChatMessage(agent.id, {
        id: userMsg.id,
        source: 'user',
        content: userMsg.content,
      }).catch(err => console.error('Failed to persist user message:', err));

      // Send to event bus for agent processing (existing ExivMessage format)
      const exivMsg: ExivMessage = {
        id: msgId,
        source: { type: 'User', id: 'user', name: 'User' },
        target_agent: agent.id,
        content: input,
        timestamp: new Date().toISOString(),
        metadata: { target_agent_id: agent.id }
      };

      const res = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(exivMsg)
      });
      if (!res.ok) {
        throw new Error(`Chat request failed: ${res.status}`);
      }
    } catch (err) {
      console.error("Failed to send message:", err);
      setIsTyping(false);
    }
  };

  const handleReset = async () => {
    setMessages([]);
    setIsTyping(false);
    setHasMore(false);
    try {
      await api.deleteChatMessages(agent.id);
    } catch (err) {
      console.error('Failed to delete chat messages:', err);
    }
  };

  return (
    <div className="flex flex-col h-full bg-white/40 backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Console Header */}
      <div className="p-4 border-b border-slate-100 flex items-center justify-between bg-white/60">
        <div className="flex items-center gap-3">
          <div className="p-2 bg-[#2e4de6] text-white rounded-lg shadow-lg shadow-[#2e4de6]/20">
            <Cpu size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-slate-800 tracking-tight uppercase">{agent.name} Console</h2>
            <div className="flex items-center gap-2">
              <span className="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse" />
              <span className="text-[8px] font-mono text-slate-400 uppercase tracking-widest">Neural Link Active</span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <LongPressResetButton onReset={handleReset} />
          <button
            onClick={onBack}
            className="px-4 py-1.5 rounded-full border border-slate-200 text-[9px] font-bold text-slate-400 hover:text-[#2e4de6] hover:border-[#2e4de6]/30 transition-all uppercase tracking-widest"
          >
            Disconnect
          </button>
        </div>
      </div>

      {/* Message Stream */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar">
        {/* Sentinel for lazy loading older messages */}
        {hasMore && <div ref={sentinelRef} className="h-1" />}
        {isLoadingMore && (
          <div className="text-center text-[9px] font-mono text-slate-300 py-2 animate-pulse">
            Loading older messages...
          </div>
        )}

        {isLoading ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-300 space-y-4">
            <Activity size={24} className="animate-pulse" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Loading session...</p>
          </div>
        ) : messages.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-300 space-y-4">
            <Zap size={32} strokeWidth={1} className="opacity-20" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Ready for instructions</p>
          </div>
        ) : (
          messages.map((msg) => {
            const isUser = msg.source === 'user';
            return (
              <div key={msg.id} className={`flex items-start gap-3 ${isUser ? 'flex-row-reverse' : ''}`}>
                <div className={`w-8 h-8 rounded-lg flex items-center justify-center shrink-0 shadow-sm ${
                  isUser ? 'bg-white border border-slate-100 text-slate-400' : 'bg-[#2e4de6] text-white'
                }`}>
                  {isUser ? <UserIcon size={14} /> : <span className="text-[10px] font-black">AI</span>}
                </div>
                <div className={`max-w-[80%] p-4 rounded-2xl text-xs leading-relaxed shadow-sm ${
                  isUser
                    ? 'bg-white text-slate-700 rounded-tr-none'
                    : 'bg-[#2e4de6] text-white rounded-tl-none'
                }`}>
                  <MessageContent content={msg.content} />
                </div>
              </div>
            );
          })
        )}
        {isTyping && (
          <div className="flex items-start gap-3 animate-pulse">
            <div className="w-8 h-8 rounded-lg bg-[#2e4de6] text-white flex items-center justify-center shrink-0">
              <Activity size={14} />
            </div>
            <div className="bg-slate-100 text-slate-400 p-3 rounded-2xl rounded-tl-none text-[10px] font-mono">
              THINKING...
            </div>
          </div>
        )}
      </div>

      {/* Input Area */}
      <div className="p-4 bg-white/60 border-t border-slate-100">
        <div className="relative flex items-center">
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyPress={(e) => e.key === 'Enter' && sendMessage()}
            disabled={isTyping}
            placeholder={isTyping ? "PROCESSING..." : "ENTER COMMAND..."}
            className="w-full bg-white border border-slate-200 rounded-xl py-3 px-4 pr-12 text-xs font-mono focus:outline-none focus:border-[#2e4de6] transition-colors placeholder:text-slate-300 disabled:opacity-50 shadow-inner"
          />
          <button
            onClick={sendMessage}
            disabled={isTyping || !input.trim()}
            className="absolute right-2 p-2 bg-[#2e4de6] text-white rounded-lg hover:scale-105 active:scale-95 transition-all disabled:opacity-30 disabled:grayscale disabled:scale-100 shadow-lg shadow-[#2e4de6]/20"
          >
            <Send size={16} />
          </button>
        </div>
      </div>
    </div>
  );
}

/** Migrate legacy localStorage session data to server */
async function migrateLegacyData(agentId: string) {
  const key = LEGACY_SESSION_KEY_PREFIX + agentId;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return;
    const legacyMessages: ExivMessage[] = JSON.parse(raw);
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
      }).catch(() => {}); // Ignore duplicate ID errors
    }

    // Remove legacy data
    localStorage.removeItem(key);
    console.log(`Migrated ${legacyMessages.length} legacy messages for agent ${agentId}`);
  } catch {
    // Silently ignore migration errors
  }
}

export function AgentTerminal({
  agents: propAgents,
  plugins: propPlugins,
  selectedAgent: propSelectedAgent,
  onSelectAgent: propOnSelectAgent
}: AgentTerminalProps = {}) {
  const [internalAgents, setInternalAgents] = useState<AgentMetadata[]>([]);
  const [internalPlugins, setInternalPlugins] = useState<PluginManifest[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [internalSelectedAgent, setInternalSelectedAgent] = useState<AgentMetadata | null>(null);
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  const agents = propAgents || internalAgents;
  const plugins = propPlugins || internalPlugins;
  const selectedAgent = propSelectedAgent !== undefined ? propSelectedAgent : internalSelectedAgent;

  useEffect(() => {
    if (!propAgents) {
      fetchInitialData();
    } else {
      setIsLoading(false);
    }
  }, [propAgents]);

  const fetchInitialData = async () => {
    try {
      const [agentsData, pluginsData] = await Promise.all([
        api.getAgents(),
        api.getPlugins()
      ]);
      setInternalAgents(agentsData);
      setInternalPlugins(pluginsData);
    } catch (err) {
      console.error('Failed to fetch data:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleSelectAgent = (agent: AgentMetadata | null) => {
    if (propOnSelectAgent) {
      propOnSelectAgent(agent);
    } else {
      setInternalSelectedAgent(agent);
    }
  };

  if (configuringAgent) {
    return (
      <AgentPluginWorkspace
        agent={configuringAgent}
        availablePlugins={plugins.filter(p => p.is_active)}
        onBack={() => setConfiguringAgent(null)}
      />
    );
  }

  if (selectedAgent) {
    return <AgentConsole agent={selectedAgent} onBack={() => handleSelectAgent(null)} />;
  }

  return (
    <div className="flex flex-col h-full bg-white/20 backdrop-blur-3xl p-6 overflow-hidden">
      {/* Header */}
      <div className="mb-8 px-4 flex justify-between items-end">
        <div>
          <h2 className="text-2xl font-black tracking-tighter text-slate-800 uppercase leading-none">AI Containers</h2>
          <p className="text-[10px] text-slate-400 font-mono tracking-[0.3em] uppercase mt-2">Exiv Registered Instances</p>
        </div>
        <div className="text-[9px] font-mono text-slate-300">SYSTEM_UPTIME: 100%</div>
      </div>

      {/* Container List */}
      <div className="flex-1 overflow-y-auto space-y-2 no-scrollbar px-2">
        {isLoading ? (
          <div className="h-full flex items-center justify-center text-slate-300 font-mono text-[10px] tracking-widest uppercase animate-pulse">
            Scanning for containers...
          </div>
        ) : (
          agents.map((agent) => (
            <div
              key={agent.id}
              className="group flex h-24 bg-white/60 hover:bg-white border border-slate-100 rounded-2xl overflow-hidden transition-all duration-500 shadow-sm"
            >
              {/* Left / Main Info */}
              <div
                onClick={() => handleSelectAgent(agent)}
                className="flex-1 flex items-center px-8 cursor-pointer relative overflow-hidden"
              >
                <div className="absolute left-0 top-0 w-1 h-full bg-[#2e4de6] opacity-0 group-hover:opacity-100 transition-opacity" />
                <div className="w-12 h-12 rounded-xl bg-slate-50 flex items-center justify-center text-[#2e4de6] mr-6 border border-slate-100 group-hover:bg-[#2e4de6] group-hover:text-white transition-colors duration-500">
                  <Cpu size={24} />
                </div>
                <div>
                  <div className="flex items-center gap-3">
                    <h3 className="text-lg font-black text-slate-800 tracking-tight">{agent.name}</h3>
                    <span className="flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-50 text-emerald-500 text-[8px] font-bold tracking-widest uppercase">
                       <Activity size={8} /> Online
                    </span>
                  </div>
                  <p className="text-[10px] text-slate-400 mt-1 font-mono">{agent.description}</p>
                </div>
                <ChevronRight size={20} className="ml-auto text-slate-200 group-hover:text-[#2e4de6] transition-colors" />
              </div>

              {/* Right Small Square (Plugin Connection Port) */}
              <button
                title="Manage Container Plugins"
                className="w-24 h-full border-l border-slate-100 bg-slate-50/50 hover:bg-[#2e4de6]/10 flex flex-col items-center justify-center gap-2 transition-all hover:text-[#2e4de6]"
                onClick={(e) => {
                   e.stopPropagation();
                   setConfiguringAgent(agent);
                }}
              >
                <Puzzle size={20} className="text-slate-400 group-hover:text-[#2e4de6]/80 transition-colors" />
                <span className="text-[8px] font-black tracking-tighter uppercase text-slate-400 group-hover:text-[#2e4de6]">Plugins</span>
              </button>
            </div>
          ))
        )}
      </div>

      {/* Footer / Stats */}
      <div className="mt-6 pt-6 border-t border-slate-100/50 flex justify-between items-center px-4">
        <div className="flex gap-6">
           <div className="flex flex-col">
             <span className="text-[8px] font-mono text-slate-300 uppercase">Total Memory</span>
             <span className="text-[10px] font-bold text-slate-500">2.4 GB</span>
           </div>
           <div className="flex flex-col">
             <span className="text-[8px] font-mono text-slate-300 uppercase">Active Units</span>
             <span className="text-[10px] font-bold text-slate-500">{agents.length} Agent(s)</span>
           </div>
        </div>
        <div className="p-2 rounded-lg bg-slate-50 border border-slate-100">
           <MessageSquare size={16} className="text-slate-300" />
        </div>
      </div>
    </div>
  );
}
