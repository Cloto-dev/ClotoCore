import React, { useState, useEffect, useRef, useCallback } from 'react';
import { ChevronRight, Puzzle, Activity, MessageSquare, Send, Zap, User as UserIcon, RotateCcw, Plus, Cpu, Settings, Terminal, Power, Lock } from 'lucide-react';
import { AgentMetadata, PluginManifest, ExivMessage, ChatMessage, ContentBlock } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor, AgentTypeIcon, agentTypeColor, AgentType, isAiAgent, statusBadgeClass, statusDotColor } from '../lib/agentIdentity';
import { isLlmPlugin } from '../lib/pluginUtils';
import { useLongPress } from '../hooks/useLongPress';

import { api, API_BASE } from '../services/api';

export interface AgentTerminalProps {
  agents?: AgentMetadata[];
  plugins?: PluginManifest[];
  selectedAgent?: AgentMetadata | null;
  onSelectAgent?: (agent: AgentMetadata | null) => void;
  onRefresh?: () => void;
}

// Legacy localStorage key prefix for migration
const LEGACY_SESSION_KEY_PREFIX = 'exiv-chat-';

function LongPressResetButton({ onReset }: { onReset: () => void }) {
  const { progress, handlers } = useLongPress(2000, onReset);

  return (
    <button
      {...handlers}
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

function LongPressPowerButton({ agent, onComplete }: { agent: AgentMetadata; onComplete: (agent: AgentMetadata) => void }) {
  const durationMs = agent.enabled ? 2000 : 1000;
  const { progress, handlers } = useLongPress(durationMs, () => onComplete(agent));

  const isOn = agent.enabled;
  const progressColor = isOn ? 'bg-red-400/25' : 'bg-emerald-400/25';
  const ringColor = isOn
    ? (progress > 0 ? 'border-red-300 text-red-500' : 'border-emerald-200 text-emerald-500')
    : (progress > 0 ? 'border-emerald-300 text-emerald-500' : 'border-slate-200 text-slate-400');

  return (
    <button
      {...handlers}
      onMouseDown={(e) => { e.stopPropagation(); handlers.onMouseDown(); }}
      onTouchStart={(e) => { e.stopPropagation(); handlers.onTouchStart(); }}
      onClick={(e) => e.stopPropagation()}
      className={`relative p-2 rounded-lg border transition-all overflow-hidden ${ringColor} ${
        isOn ? 'hover:bg-emerald-50' : 'hover:bg-slate-50'
      }`}
      title={isOn ? `Power Off (hold ${durationMs / 1000}s)` : `Power On (hold ${durationMs / 1000}s)`}
    >
      {progress > 0 && (
        <span
          className={`absolute inset-0 ${progressColor} origin-left transition-none`}
          style={{ transform: `scaleX(${progress})` }}
        />
      )}
      <Power size={16} className="relative" />
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
          <div className="p-2 text-white rounded-lg shadow-lg" style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}>
            <AgentIcon agent={agent} size={18} />
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
                  isUser ? 'bg-white border border-slate-100 text-slate-400' : 'text-white'
                }`} style={!isUser ? { backgroundColor: agentColor(agent) } : undefined}>
                  {isUser ? <UserIcon size={14} /> : <AgentIcon agent={agent} size={14} />}
                </div>
                <div className={`max-w-[80%] p-4 rounded-2xl text-xs leading-relaxed shadow-sm ${
                  isUser
                    ? 'bg-white text-slate-700 rounded-tr-none'
                    : 'text-white rounded-tl-none'
                }`} style={!isUser ? { backgroundColor: agentColor(agent) } : undefined}>
                  <MessageContent content={msg.content} />
                </div>
              </div>
            );
          })
        )}
        {isTyping && (
          <div className="flex items-start gap-3 animate-pulse">
            <div className="w-8 h-8 rounded-lg text-white flex items-center justify-center shrink-0" style={{ backgroundColor: agentColor(agent) }}>
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

function ContainerDashboard({ agent, plugins, onBack, onConfigure, onPowerToggle }: {
  agent: AgentMetadata;
  plugins: PluginManifest[];
  onBack: () => void;
  onConfigure: () => void;
  onPowerToggle: (agent: AgentMetadata) => void;
}) {
  const color = agentColor(agent);
  const enginePlugin = plugins.find(p => p.id === agent.default_engine_id);
  const memoryPlugin = plugins.find(p => p.id === agent.metadata?.preferred_memory);

  return (
    <div className="flex flex-col h-full bg-white/40 backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Header */}
      <div className="p-4 border-b border-slate-100 flex items-center justify-between bg-white/60">
        <div className="flex items-center gap-3">
          <div className="p-2 text-white rounded-lg shadow-lg" style={{ backgroundColor: color, boxShadow: `0 10px 15px -3px ${color}33` }}>
            <AgentIcon agent={agent} size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-slate-800 tracking-tight uppercase">{agent.name}</h2>
            <div className="flex items-center gap-2">
              <span className={`w-1.5 h-1.5 rounded-full ${statusDotColor(agent.status)}`} />
              <span className="text-[8px] font-mono text-slate-400 uppercase tracking-widest">
                Container Process {agent.enabled ? '· Running' : '· Stopped'}
              </span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {agent.metadata?.has_power_password === 'true' ? (
            <button
              className={`p-2 rounded-lg border transition-all ${
                agent.enabled
                  ? 'border-emerald-200 text-emerald-500 hover:bg-emerald-50'
                  : 'border-slate-200 text-slate-400 hover:bg-slate-50'
              }`}
              title={agent.enabled ? 'Power Off' : 'Power On'}
              onClick={() => onPowerToggle(agent)}
            >
              <Power size={16} />
            </button>
          ) : (
            <LongPressPowerButton agent={agent} onComplete={onPowerToggle} />
          )}
          <button
            onClick={onBack}
            className="px-4 py-1.5 rounded-full border border-slate-200 text-[9px] font-bold text-slate-400 hover:text-slate-600 hover:border-slate-300 transition-all uppercase tracking-widest"
          >
            Back
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-8 no-scrollbar">
        <div className="max-w-lg mx-auto space-y-6">
          {/* Description */}
          <div className="bg-white border border-slate-100 rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-slate-400 uppercase tracking-[0.15em] mb-3">Description</h3>
            <p className="text-sm text-slate-600 leading-relaxed">
              {agent.description || 'No description provided.'}
            </p>
          </div>

          {/* Configuration */}
          <div className="bg-white border border-slate-100 rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-slate-400 uppercase tracking-[0.15em] mb-4">Configuration</h3>
            <div className="space-y-3">
              <div className="flex items-center justify-between py-2 border-b border-slate-50">
                <span className="text-[11px] font-bold text-slate-500">Agent ID</span>
                <span className="text-[11px] font-mono text-slate-400">{agent.id}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-slate-50">
                <span className="text-[11px] font-bold text-slate-500">Bridge Engine</span>
                <span className="text-[11px] font-mono text-slate-400">{enginePlugin?.name || agent.default_engine_id || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-slate-50">
                <span className="text-[11px] font-bold text-slate-500">Memory</span>
                <span className="text-[11px] font-mono text-slate-400">{memoryPlugin?.name || agent.metadata?.preferred_memory || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-slate-50">
                <span className="text-[11px] font-bold text-slate-500">Type</span>
                <span className="inline-flex items-center gap-1.5 text-[11px] font-mono px-2 py-0.5 rounded-full" style={{ backgroundColor: `${color}12`, color }}>
                  <Cpu size={10} />
                  Container
                </span>
              </div>
              <div className="flex items-center justify-between py-2">
                <span className="text-[11px] font-bold text-slate-500">Power</span>
                <span className={`inline-flex items-center gap-1.5 px-2 py-0.5 rounded text-[10px] font-bold ${statusBadgeClass(agent.status)}`}>
                  {agent.metadata?.has_power_password === 'true' && <Lock size={8} />}
                  {agent.status.toUpperCase()}
                </span>
              </div>
            </div>
          </div>

          {/* Actions */}
          <div className="flex gap-3">
            <button
              onClick={onConfigure}
              className="flex-1 flex items-center justify-center gap-2 py-3 rounded-xl text-white text-xs font-bold shadow-lg transition-all hover:shadow-xl active:scale-[0.98]"
              style={{ backgroundColor: color, boxShadow: `0 10px 15px -3px ${color}33` }}
            >
              <Puzzle size={14} />
              Manage Plugins
            </button>
          </div>

          {/* Info Notice */}
          <div className="flex items-start gap-3 p-4 bg-slate-50 rounded-xl border border-slate-100">
            <Terminal size={14} className="text-slate-400 shrink-0 mt-0.5" />
            <p className="text-[10px] text-slate-400 leading-relaxed">
              This is a non-AI container agent. It operates through bridge scripts and does not support interactive chat.
              Use the plugin workspace to configure its engine and memory modules.
            </p>
          </div>
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
  onSelectAgent: propOnSelectAgent,
  onRefresh: propOnRefresh
}: AgentTerminalProps = {}) {
  const [internalAgents, setInternalAgents] = useState<AgentMetadata[]>([]);
  const [internalPlugins, setInternalPlugins] = useState<PluginManifest[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [internalSelectedAgent, setInternalSelectedAgent] = useState<AgentMetadata | null>(null);
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  // Creation form state (must be before any early returns)
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [newEngine, setNewEngine] = useState('');
  const [newMemory, setNewMemory] = useState('');
  const [newType, setNewType] = useState<AgentType>('ai');
  const [newPassword, setNewPassword] = useState('');
  const [isCreating, setIsCreating] = useState(false);

  // Power toggle + password modal state
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);
  const [powerPassword, setPowerPassword] = useState('');
  const [powerError, setPowerError] = useState('');
  const [isPowerLoading, setIsPowerLoading] = useState(false);

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

  const refreshAgents = () => {
    if (propOnRefresh) propOnRefresh();
    if (!propAgents) fetchInitialData();
  };

  // Listen for AgentPowerChanged events to auto-refresh
  useEventStream(`${API_BASE}/events`, (event) => {
    if (event.type === 'AgentPowerChanged') {
      refreshAgents();
    }
  });

  const handleSelectAgent = (agent: AgentMetadata | null) => {
    if (propOnSelectAgent) {
      propOnSelectAgent(agent);
    } else {
      setInternalSelectedAgent(agent);
    }
  };

  const handlePowerToggle = (agent: AgentMetadata) => {
    if (agent.metadata?.has_power_password === 'true') {
      setPowerTarget(agent);
      setPowerPassword('');
      setPowerError('');
    } else {
      executePowerToggle(agent, undefined);
    }
  };

  const executePowerToggle = async (agent: AgentMetadata, password?: string) => {
    setIsPowerLoading(true);
    setPowerError('');
    try {
      await api.toggleAgentPower(agent.id, !agent.enabled, password);
      setPowerTarget(null);
      setPowerPassword('');
      refreshAgents();
    } catch (err: any) {
      setPowerError(err.message || 'Failed to toggle power');
    } finally {
      setIsPowerLoading(false);
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
    if (isAiAgent(selectedAgent)) {
      return <AgentConsole agent={selectedAgent} onBack={() => handleSelectAgent(null)} />;
    }
    return (
      <ContainerDashboard
        agent={selectedAgent}
        plugins={plugins}
        onBack={() => handleSelectAgent(null)}
        onConfigure={() => setConfiguringAgent(selectedAgent)}
        onPowerToggle={handlePowerToggle}
      />
    );
  }

  const allEngines = plugins.filter(p => p.service_type === 'Reasoning' && p.is_active && p.category === 'Agent');
  const filteredEngines = allEngines.filter(p => newType === 'ai' ? isLlmPlugin(p) : !isLlmPlugin(p));
  const allMemories = plugins.filter(p => (p.service_type === 'Memory' || p.category === 'Memory') && p.is_active);
  const memories = allMemories.filter(p => newType === 'ai' ? true : !isLlmPlugin(p));

  const handleTypeChange = (type: AgentType) => {
    setNewType(type);
    setNewEngine('');
    setNewMemory('');
  };

  const handleCreate = async () => {
    setIsCreating(true);
    try {
      const res = await fetch('/api/agents', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: newName,
          description: newDesc,
          default_engine: newEngine,
          metadata: { preferred_memory: newMemory, agent_type: newType },
          password: newPassword || undefined
        })
      });
      if (res.ok) {
        setNewName(''); setNewDesc(''); setNewEngine(''); setNewMemory(''); setNewType('ai'); setNewPassword('');
        refreshAgents();
      }
    } catch (e) {
      console.error(e);
    } finally {
      setIsCreating(false);
    }
  };

  return (
    <div className="relative flex h-full bg-white/80 backdrop-blur-sm overflow-hidden">
      {/* Password Modal */}
      {powerTarget && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm">
          <div className="bg-white rounded-2xl shadow-2xl p-6 w-80 space-y-4 animate-in fade-in zoom-in-95 duration-200">
            <div className="flex items-center gap-3">
              <div className={`p-2 rounded-lg ${powerTarget.enabled ? 'bg-red-50 text-red-500' : 'bg-emerald-50 text-emerald-500'}`}>
                <Power size={18} />
              </div>
              <div>
                <h3 className="text-sm font-bold text-slate-800">
                  {powerTarget.enabled ? 'Power Off' : 'Power On'} {powerTarget.name}
                </h3>
                <p className="text-[10px] text-slate-400">Enter power password to continue</p>
              </div>
            </div>
            <div className="relative">
              <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-300" />
              <input
                type="password"
                value={powerPassword}
                onChange={e => setPowerPassword(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && powerPassword && executePowerToggle(powerTarget, powerPassword)}
                className="w-full pl-9 pr-3 py-2.5 rounded-xl border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
                placeholder="Password"
                autoFocus
              />
            </div>
            {powerError && (
              <p className="text-[10px] text-red-500 font-medium">{powerError}</p>
            )}
            <div className="flex gap-2">
              <button
                onClick={() => { setPowerTarget(null); setPowerPassword(''); setPowerError(''); }}
                className="flex-1 py-2 rounded-xl border border-slate-200 text-xs font-bold text-slate-500 hover:bg-slate-50 transition-all"
                disabled={isPowerLoading}
              >
                Cancel
              </button>
              <button
                onClick={() => executePowerToggle(powerTarget, powerPassword)}
                disabled={!powerPassword || isPowerLoading}
                className={`flex-1 py-2 rounded-xl text-white text-xs font-bold transition-all disabled:opacity-50 ${
                  powerTarget.enabled ? 'bg-red-500 hover:bg-red-600' : 'bg-emerald-500 hover:bg-emerald-600'
                }`}
              >
                {isPowerLoading ? 'Processing...' : 'Confirm'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Main content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="p-6 border-b border-slate-100 flex items-center justify-between bg-white/40">
          <div>
            <h2 className="text-xl font-black tracking-tight text-slate-800 uppercase">Agent Management</h2>
            <p className="text-[10px] text-slate-400 font-mono tracking-widest uppercase mt-1">
              EXIV-SYSTEM / Registered Instances
            </p>
          </div>
          <div className="px-3 py-1 rounded-full bg-slate-100 text-[10px] font-bold text-slate-500">
            {agents.filter(a => a.enabled).length} / {agents.length} ACTIVE
          </div>
        </div>

        {/* Agent List */}
        <div className="flex-1 overflow-y-auto p-6 space-y-3 no-scrollbar">
          {isLoading ? (
            <div className="h-full flex items-center justify-center text-slate-300 font-mono text-[10px] tracking-widest uppercase animate-pulse">
              Scanning for containers...
            </div>
          ) : agents.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-slate-300 space-y-4">
              <Zap size={32} strokeWidth={1} className="opacity-20" />
              <p className="text-[10px] font-mono tracking-[0.2em] uppercase">No agents registered</p>
            </div>
          ) : (
            agents.map((agent) => (
              <div
                key={agent.id}
                className="group p-4 bg-white border border-slate-100 rounded-xl shadow-sm flex items-center gap-4 transition-shadow duration-300 cursor-pointer"
                onMouseEnter={(e) => e.currentTarget.style.boxShadow = `0 8px 25px -5px ${agentColor(agent)}30`}
                onMouseLeave={(e) => e.currentTarget.style.boxShadow = ''}
                onClick={() => handleSelectAgent(agent)}
              >
                <div className="p-2.5 rounded-xl shrink-0" style={{ backgroundColor: `${agentColor(agent)}12`, color: agentColor(agent) }}>
                  <AgentIcon agent={agent} size={22} />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="font-bold text-slate-700 text-sm truncate">{agent.name}</h3>
                    <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[8px] font-bold ${statusBadgeClass(agent.status)}`}>
                      {agent.metadata?.has_power_password === 'true' && <Lock size={7} />}
                      {agent.status.toUpperCase()}
                    </span>
                  </div>
                  <p className="text-[11px] text-slate-500 mt-0.5 truncate">{agent.description}</p>
                  <div className="flex gap-2 mt-2">
                    <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">
                      ENGINE: {agent.default_engine_id || 'DEFAULT'}
                    </span>
                    <span className="text-[9px] bg-slate-100 px-1.5 py-0.5 rounded text-slate-400 font-mono">
                      MEM: {agent.metadata?.preferred_memory || 'DEFAULT'}
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  {agent.metadata?.has_power_password === 'true' ? (
                    <button
                      className={`p-2 rounded-lg border transition-all ${
                        agent.enabled
                          ? 'border-emerald-200 text-emerald-500 hover:bg-emerald-50'
                          : 'border-slate-200 text-slate-400 hover:bg-slate-50'
                      }`}
                      title={agent.enabled ? 'Power Off' : 'Power On'}
                      onClick={(e) => { e.stopPropagation(); handlePowerToggle(agent); }}
                    >
                      <Power size={16} />
                    </button>
                  ) : (
                    <LongPressPowerButton agent={agent} onComplete={handlePowerToggle} />
                  )}
                  <button
                    title="Manage Plugins"
                    className="p-2 rounded-lg border border-slate-100 text-slate-400 hover:text-[#2e4de6] hover:border-[#2e4de6]/30 hover:bg-[#2e4de6]/5 transition-all"
                    onClick={(e) => { e.stopPropagation(); setConfiguringAgent(agent); }}
                  >
                    <Puzzle size={16} />
                  </button>
                  <ChevronRight size={18} className="text-slate-300 group-hover:text-slate-500 transition-colors" />
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* Right Sidebar: Create Form */}
      <div className="w-[380px] shrink-0 border-l border-slate-100 bg-slate-50/30 overflow-y-auto no-scrollbar hidden lg:flex flex-col">
        <div className="p-5 border-b border-slate-100 bg-white/40">
          <h3 className="text-[11px] font-black text-slate-500 uppercase tracking-[0.15em]">Initialize New Agent</h3>
        </div>
        <div className="p-5 flex-1">
          <div className="space-y-4">
            {/* Agent Type Selector */}
            <div>
              <label className="block text-xs font-bold text-slate-500 mb-2">Agent Type</label>
              <div className="grid grid-cols-2 gap-3">
                {([['ai', 'AI Agent', 'LLM-powered reasoning'], ['container', 'Container', 'Script / bridge process']] as const).map(([type, label, desc]) => {
                  const selected = newType === type;
                  const color = agentTypeColor(type);
                  return (
                    <button
                      key={type}
                      type="button"
                      onClick={() => handleTypeChange(type)}
                      className={`flex items-center gap-2.5 p-3 rounded-xl border-2 transition-all text-left ${
                        selected ? 'bg-white shadow-md' : 'bg-white/50 border-slate-200 hover:border-slate-300'
                      }`}
                      style={selected ? { borderColor: color } : undefined}
                    >
                      <div className="p-1.5 rounded-lg text-white shrink-0" style={{ backgroundColor: selected ? color : '#94a3b8' }}>
                        <AgentTypeIcon type={type} size={16} />
                      </div>
                      <div>
                        <div className="text-[11px] font-bold text-slate-700">{label}</div>
                        <div className="text-[8px] text-slate-400">{desc}</div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">Agent Name</label>
              <input
                type="text"
                value={newName}
                onChange={e => setNewName(e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
                placeholder="e.g. Mike"
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">Description / System Prompt</label>
              <textarea
                value={newDesc}
                onChange={e => setNewDesc(e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6] h-16 resize-none"
                placeholder="Briefly describe the agent's role."
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">
                {newType === 'ai' ? 'LLM Engine' : 'Bridge Engine'}
              </label>
              {filteredEngines.length > 0 ? (
                <select
                  value={newEngine}
                  onChange={e => setNewEngine(e.target.value)}
                  className="w-full px-2 py-1.5 rounded-lg border border-slate-200 text-xs focus:outline-none focus:border-[#2e4de6] bg-white"
                >
                  <option value="">Select Engine...</option>
                  {filteredEngines.map(p => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              ) : (
                <div className="w-full px-2 py-1.5 rounded-lg border border-dashed border-slate-300 text-[10px] text-slate-400 font-mono text-center">
                  No {newType === 'ai' ? 'LLM' : 'bridge'} engines available
                </div>
              )}
            </div>

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">Memory Engine</label>
              <select
                value={newMemory}
                onChange={e => setNewMemory(e.target.value)}
                className="w-full px-2 py-1.5 rounded-lg border border-slate-200 text-xs focus:outline-none focus:border-[#2e4de6] bg-white"
              >
                <option value="">Select Memory...</option>
                {memories.map(p => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="block text-xs font-bold text-slate-500 mb-1">
                Power Password <span className="text-slate-300 font-normal">(optional)</span>
              </label>
              <div className="relative">
                <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-300" />
                <input
                  type="password"
                  value={newPassword}
                  onChange={e => setNewPassword(e.target.value)}
                  className="w-full pl-9 pr-3 py-2 rounded-lg border border-slate-200 text-sm focus:outline-none focus:border-[#2e4de6]"
                  placeholder="Leave empty for no password"
                />
              </div>
              <p className="text-[9px] text-slate-400 mt-1">Require password to toggle power on/off</p>
            </div>

            <button
              onClick={handleCreate}
              disabled={!newName || !newEngine || isCreating}
              className="w-full mt-2 text-white py-2.5 rounded-xl text-sm font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
              style={{ backgroundColor: agentTypeColor(newType) }}
            >
              {isCreating ? <Activity size={16} className="animate-spin" /> : <Plus size={16} />}
              {newType === 'ai' ? 'CREATE AI AGENT' : 'CREATE CONTAINER'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
