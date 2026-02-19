import React, { useState, useEffect, useRef, useCallback } from 'react';
import { ChevronRight, Puzzle, Activity, MessageSquare, Send, Zap, User as UserIcon, RotateCcw, Plus, Cpu, Settings, Terminal, Power, Lock, Trash2 } from 'lucide-react';
import { AgentMetadata, PluginManifest, ExivMessage, ChatMessage, ContentBlock } from '../types';
import { AgentPluginWorkspace } from './AgentPluginWorkspace';
import { useEventStream } from '../hooks/useEventStream';
import { AgentIcon, agentColor, AgentTypeIcon, agentTypeColor, isAiAgent, statusBadgeClass, statusDotColor } from '../lib/agentIdentity';
import { isLlmPlugin } from '../lib/pluginUtils';
import { useLongPress } from '../hooks/useLongPress';
import { useAgentCreation } from '../hooks/useAgentCreation';
import { PowerToggleModal } from './PowerToggleModal';

import { api, API_BASE } from '../services/api';
import { useApiKey } from '../contexts/ApiKeyContext';

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

function LongPressPowerButton({ agent, onComplete }: { agent: AgentMetadata; onComplete: (agent: AgentMetadata) => void }) {
  const durationMs = agent.enabled ? 2000 : 1000;
  const { progress, handlers } = useLongPress(durationMs, () => onComplete(agent));

  const isOn = agent.enabled;
  const progressColor = isOn ? 'bg-red-400/25' : 'bg-emerald-400/25';
  const ringColor = isOn
    ? (progress > 0 ? 'border-red-300 text-red-500' : 'border-emerald-200 text-emerald-500')
    : (progress > 0 ? 'border-emerald-300 text-emerald-500' : 'border-edge text-content-tertiary');

  return (
    <button
      {...handlers}
      onMouseDown={(e) => { e.stopPropagation(); handlers.onMouseDown(); }}
      onTouchStart={(e) => { e.stopPropagation(); handlers.onTouchStart(); }}
      onClick={(e) => e.stopPropagation()}
      className={`relative p-2 rounded-lg border transition-all overflow-hidden ${ringColor} ${
        isOn ? 'hover:bg-emerald-50' : 'hover:bg-surface-base'
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
  const { apiKey } = useApiKey();
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
        await migrateLegacyData(agent.id, apiKey);

        const { messages: loaded, has_more } = await api.getChatMessages(agent.id, apiKey, undefined, 50);
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
      }, apiKey).catch(err => console.error('Failed to persist agent response:', err));
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
      // Persist user message first — cancel send if this fails
      await api.postChatMessage(agent.id, {
        id: userMsg.id,
        source: 'user',
        content: userMsg.content,
      }, apiKey);

      // Send to event bus for agent processing
      const exivMsg: ExivMessage = {
        id: msgId,
        source: { type: 'User', id: 'user', name: 'User' },
        target_agent: agent.id,
        content: input,
        timestamp: new Date().toISOString(),
        metadata: { target_agent_id: agent.id }
      };

      await api.postChat(exivMsg, apiKey);
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
    setHasMore(false);
    try {
      await api.deleteChatMessages(agent.id, apiKey);
    } catch (err) {
      console.error('Failed to delete chat messages:', err);
    }
  };

  return (
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Console Header */}
      <div className="p-4 border-b border-edge-subtle flex items-center justify-between bg-glass-strong">
        <div className="flex items-center gap-3">
          <div className="p-2 text-white rounded-lg shadow-lg" style={{ backgroundColor: agentColor(agent), boxShadow: `0 10px 15px -3px ${agentColor(agent)}33` }}>
            <AgentIcon agent={agent} size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-content-primary tracking-tight uppercase">{agent.name} Console</h2>
            <div className="flex items-center gap-2">
              <span className="w-1.5 h-1.5 bg-emerald-500 rounded-full animate-pulse" />
              <span className="text-[8px] font-mono text-content-tertiary uppercase tracking-widest">Neural Link Active</span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <LongPressResetButton onReset={handleReset} />
          <button
            onClick={onBack}
            className="px-4 py-1.5 rounded-full border border-edge text-[9px] font-bold text-content-tertiary hover:text-brand hover:border-brand/30 transition-all uppercase tracking-widest"
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
          <div className="text-center text-[9px] font-mono text-content-muted py-2 animate-pulse">
            Loading older messages...
          </div>
        )}

        {isLoading ? (
          <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
            <Activity size={24} className="animate-pulse" />
            <p className="text-[10px] font-mono tracking-[0.2em] uppercase">Loading session...</p>
          </div>
        ) : messages.length === 0 ? (
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
                <div className={`max-w-[80%] p-4 rounded-2xl text-xs leading-relaxed shadow-sm select-text ${
                  isUser
                    ? 'bg-surface-primary text-content-primary rounded-tr-none'
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
            <div className="bg-surface-secondary text-content-tertiary p-3 rounded-2xl rounded-tl-none text-[10px] font-mono">
              THINKING...
            </div>
          </div>
        )}
      </div>

      {/* Input Area */}
      <div className="p-4 bg-glass-strong border-t border-edge-subtle">
        <div className="relative flex items-center">
          <input
            type="text"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyPress={(e) => e.key === 'Enter' && sendMessage()}
            disabled={isTyping}
            placeholder={isTyping ? "PROCESSING..." : "ENTER COMMAND..."}
            className="w-full bg-surface-primary border border-edge rounded-xl py-3 px-4 pr-12 text-xs font-mono focus:outline-none focus:border-brand transition-colors placeholder:text-content-muted disabled:opacity-50 shadow-inner"
          />
          <button
            onClick={sendMessage}
            disabled={isTyping || !input.trim()}
            className="absolute right-2 p-2 bg-brand text-white rounded-lg hover:scale-105 active:scale-95 transition-all disabled:opacity-30 disabled:grayscale disabled:scale-100 shadow-lg shadow-brand/20"
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
    <div className="flex flex-col h-full bg-glass backdrop-blur-3xl animate-in fade-in duration-500">
      {/* Header */}
      <div className="p-4 border-b border-edge-subtle flex items-center justify-between bg-glass-strong">
        <div className="flex items-center gap-3">
          <div className="p-2 text-white rounded-lg shadow-lg" style={{ backgroundColor: color, boxShadow: `0 10px 15px -3px ${color}33` }}>
            <AgentIcon agent={agent} size={18} />
          </div>
          <div>
            <h2 className="text-sm font-black text-content-primary tracking-tight uppercase">{agent.name}</h2>
            <div className="flex items-center gap-2">
              <span className={`w-1.5 h-1.5 rounded-full ${statusDotColor(agent.status)}`} />
              <span className="text-[8px] font-mono text-content-tertiary uppercase tracking-widest">
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
                  : 'border-edge text-content-tertiary hover:bg-surface-base'
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
            className="px-4 py-1.5 rounded-full border border-edge text-[9px] font-bold text-content-tertiary hover:text-content-secondary hover:border-edge transition-all uppercase tracking-widest"
          >
            Back
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-8 no-scrollbar">
        <div className="max-w-lg mx-auto space-y-6">
          {/* Description */}
          <div className="bg-surface-primary border border-edge-subtle rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.15em] mb-3">Description</h3>
            <p className="text-sm text-content-secondary leading-relaxed">
              {agent.description || 'No description provided.'}
            </p>
          </div>

          {/* Configuration */}
          <div className="bg-surface-primary border border-edge-subtle rounded-2xl p-6 shadow-sm">
            <h3 className="text-[10px] font-black text-content-tertiary uppercase tracking-[0.15em] mb-4">Configuration</h3>
            <div className="space-y-3">
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Agent ID</span>
                <span className="text-[11px] font-mono text-content-tertiary">{agent.id}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Bridge Engine</span>
                <span className="text-[11px] font-mono text-content-tertiary">{enginePlugin?.name || agent.default_engine_id || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Memory</span>
                <span className="text-[11px] font-mono text-content-tertiary">{memoryPlugin?.name || agent.metadata?.preferred_memory || 'None'}</span>
              </div>
              <div className="flex items-center justify-between py-2 border-b border-edge-subtle">
                <span className="text-[11px] font-bold text-content-secondary">Type</span>
                <span className="inline-flex items-center gap-1.5 text-[11px] font-mono px-2 py-0.5 rounded-full" style={{ backgroundColor: `${color}12`, color }}>
                  <Cpu size={10} />
                  Container
                </span>
              </div>
              <div className="flex items-center justify-between py-2">
                <span className="text-[11px] font-bold text-content-secondary">Power</span>
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
          <div className="flex items-start gap-3 p-4 bg-surface-base rounded-xl border border-edge-subtle">
            <Terminal size={14} className="text-content-tertiary shrink-0 mt-0.5" />
            <p className="text-[10px] text-content-tertiary leading-relaxed">
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
async function migrateLegacyData(agentId: string, apiKey: string) {
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
      }, apiKey).catch(() => {}); // Ignore duplicate ID errors
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
  const { apiKey } = useApiKey();
  const [internalAgents, setInternalAgents] = useState<AgentMetadata[]>([]);
  const [internalPlugins, setInternalPlugins] = useState<PluginManifest[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [internalSelectedAgent, setInternalSelectedAgent] = useState<AgentMetadata | null>(null);
  const [configuringAgent, setConfiguringAgent] = useState<AgentMetadata | null>(null);

  // Power toggle modal
  const [powerTarget, setPowerTarget] = useState<AgentMetadata | null>(null);

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState<AgentMetadata | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const DEFAULT_AGENT_ID = 'agent.exiv_default';

  const handleDeleteConfirm = async () => {
    if (!deleteTarget) return;
    setIsDeleting(true);
    setDeleteError(null);
    try {
      await api.deleteAgent(deleteTarget.id, apiKey);
      setDeleteTarget(null);
      refreshAgents();
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : 'Unknown error');
    } finally {
      setIsDeleting(false);
    }
  };

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

  // Creation form
  const { form: newAgent, updateField, handleTypeChange, handleCreate, isCreating, createError } = useAgentCreation(refreshAgents);

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

  const handlePowerToggle = async (agent: AgentMetadata) => {
    if (agent.metadata?.has_power_password === 'true') {
      setPowerTarget(agent);
    } else {
      try {
        await api.toggleAgentPower(agent.id, !agent.enabled, apiKey);
        refreshAgents();
      } catch (err) {
        console.error('Failed to toggle power:', err);
      }
    }
  };

  if (configuringAgent) {
    return (
      <AgentPluginWorkspace
        agent={configuringAgent}
        availablePlugins={plugins.filter(p => p.is_active)}
        onBack={() => { setConfiguringAgent(null); refreshAgents(); }}
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
  const filteredEngines = allEngines.filter(p => newAgent.type === 'ai' ? isLlmPlugin(p) : !isLlmPlugin(p));
  const allMemories = plugins.filter(p => (p.service_type === 'Memory' || p.category === 'Memory') && p.is_active);
  const memories = allMemories.filter(p => newAgent.type === 'ai' ? true : !isLlmPlugin(p));

  return (
    <div className="relative flex h-full bg-glass-subtle backdrop-blur-sm overflow-hidden">
      {/* Password Modal */}
      {powerTarget && (
        <PowerToggleModal
          agent={powerTarget}
          onClose={() => setPowerTarget(null)}
          onSuccess={refreshAgents}
        />
      )}

      {/* Delete confirmation modal */}
      {deleteTarget && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm">
          <div className="bg-surface-primary border border-edge rounded-2xl shadow-xl p-6 w-80 space-y-4">
            <div className="flex items-center gap-3">
              <div className="p-2 rounded-xl bg-red-500/10 text-red-500"><Trash2 size={18} /></div>
              <div>
                <h3 className="font-bold text-content-primary text-sm">Delete Agent</h3>
                <p className="text-[10px] text-content-tertiary font-mono mt-0.5">Irreversible operation</p>
              </div>
            </div>
            <div className="bg-surface-secondary rounded-xl p-3 space-y-1">
              <p className="text-xs font-bold text-content-primary">{deleteTarget.name}</p>
              <p className="text-[10px] text-content-tertiary font-mono">{deleteTarget.id}</p>
            </div>
            <p className="text-xs text-content-secondary">
              All chat history for this agent will be permanently deleted. This cannot be undone.
            </p>
            {deleteError && (
              <p className="text-xs text-red-400">{deleteError}</p>
            )}
            <div className="flex gap-2 pt-1">
              <button
                onClick={() => { setDeleteTarget(null); setDeleteError(null); }}
                disabled={isDeleting}
                className="flex-1 py-2 rounded-xl border border-edge text-xs font-bold text-content-secondary hover:bg-surface-secondary transition-all disabled:opacity-50"
              >
                Cancel
              </button>
              <button
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                className="flex-1 py-2 rounded-xl bg-red-500 text-white text-xs font-bold hover:bg-red-600 transition-all disabled:opacity-50 flex items-center justify-center gap-1"
              >
                {isDeleting ? <Activity size={12} className="animate-spin" /> : <Trash2 size={12} />}
                Delete
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Main content */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Header */}
        <div className="p-6 flex items-center justify-between">
          <div>
            <h2 className="text-xl font-black tracking-tight text-content-primary uppercase">Agent Management</h2>
            <p className="text-[10px] text-content-tertiary font-mono tracking-widest uppercase mt-1">
              EXIV-SYSTEM / Registered Instances
            </p>
          </div>
          <div className="px-3 py-1 rounded-full bg-surface-secondary text-[10px] font-bold text-content-secondary">
            {agents.filter(a => a.enabled).length} / {agents.length} ACTIVE
          </div>
        </div>

        {/* Agent List */}
        <div className="flex-1 overflow-y-auto p-6 space-y-3 no-scrollbar bg-gradient-to-b from-surface-primary/40 from-25% via-surface-primary/20 via-65% to-brand/[0.05]">
          {isLoading ? (
            <div className="h-full flex items-center justify-center text-content-muted font-mono text-[10px] tracking-widest uppercase animate-pulse">
              Scanning for containers...
            </div>
          ) : agents.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-content-muted space-y-4">
              <Zap size={32} strokeWidth={1} className="opacity-20" />
              <p className="text-[10px] font-mono tracking-[0.2em] uppercase">No agents registered</p>
            </div>
          ) : (
            agents.map((agent) => (
              <div
                key={agent.id}
                className="group p-4 bg-surface-primary border border-edge rounded-xl shadow-sm flex items-center gap-4 cursor-pointer"
                onClick={() => handleSelectAgent(agent)}
              >
                <div className="p-2.5 rounded-xl shrink-0" style={{ backgroundColor: `${agentColor(agent)}12`, color: agentColor(agent) }}>
                  <AgentIcon agent={agent} size={22} />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="font-bold text-content-primary text-sm truncate">{agent.name}</h3>
                    <span className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[8px] font-bold ${statusBadgeClass(agent.status)}`}>
                      {agent.metadata?.has_power_password === 'true' && <Lock size={7} />}
                      {agent.status.toUpperCase()}
                    </span>
                  </div>
                  <p className="text-[11px] text-content-secondary mt-0.5 truncate">{agent.description}</p>
                  <div className="flex gap-2 mt-2">
                    <span className="text-[9px] bg-surface-secondary px-1.5 py-0.5 rounded text-content-tertiary font-mono">
                      ENGINE: {agent.default_engine_id || 'DEFAULT'}
                    </span>
                    <span className="text-[9px] bg-surface-secondary px-1.5 py-0.5 rounded text-content-tertiary font-mono">
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
                          : 'border-edge text-content-tertiary hover:bg-surface-base'
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
                    className="p-2 rounded-lg border border-edge-subtle text-content-tertiary hover:text-brand hover:border-brand/30 hover:bg-brand/5 transition-all"
                    onClick={(e) => { e.stopPropagation(); setConfiguringAgent(agent); }}
                  >
                    <Puzzle size={16} />
                  </button>
                  {agent.id === DEFAULT_AGENT_ID ? (
                    <div title="Default agent is protected" className="p-2 text-content-muted opacity-30">
                      <Lock size={15} />
                    </div>
                  ) : (
                    <button
                      title="Delete agent"
                      className="p-2 rounded-lg border border-edge-subtle text-content-tertiary hover:text-red-500 hover:border-red-200 hover:bg-red-50 transition-all"
                      onClick={(e) => { e.stopPropagation(); setDeleteTarget(agent); setDeleteError(null); }}
                    >
                      <Trash2 size={15} />
                    </button>
                  )}
                  <ChevronRight size={18} className="text-content-muted group-hover:text-content-secondary transition-colors" />
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* Right Sidebar: Create Form */}
      <div className="w-[380px] shrink-0 border-l border-[var(--border-strong)] bg-surface-base/30 overflow-y-auto no-scrollbar hidden lg:flex flex-col">
        <div className="p-5">
          <h3 className="text-[11px] font-black text-content-secondary uppercase tracking-[0.15em]">Initialize New Agent</h3>
        </div>
        <div className="p-5 flex-1">
          <div className="space-y-4">
            {/* Agent Type Selector */}
            <div>
              <label className="block text-xs font-bold text-content-secondary mb-2">Agent Type</label>
              <div className="grid grid-cols-2 gap-3">
                {([['ai', 'AI Agent', 'LLM-powered reasoning'], ['container', 'Container', 'Script / bridge process']] as const).map(([type, label, desc]) => {
                  const selected = newAgent.type === type;
                  const color = agentTypeColor(type);
                  return (
                    <button
                      key={type}
                      type="button"
                      onClick={() => handleTypeChange(type)}
                      className={`flex items-center gap-2.5 p-3 rounded-xl border-2 transition-all text-left ${
                        selected ? 'bg-surface-primary shadow-md' : 'bg-surface-primary/50 border-edge hover:border-edge'
                      }`}
                      style={selected ? { borderColor: color } : undefined}
                    >
                      <div className="p-1.5 rounded-lg text-white shrink-0" style={{ backgroundColor: selected ? color : '#94a3b8' }}>
                        <AgentTypeIcon type={type} size={16} />
                      </div>
                      <div>
                        <div className="text-[11px] font-bold text-content-primary">{label}</div>
                        <div className="text-[8px] text-content-tertiary">{desc}</div>
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Agent Name</label>
              <input
                type="text"
                value={newAgent.name}
                onChange={e => updateField('name', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary"
                placeholder="e.g. Mike"
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Description / System Prompt</label>
              <textarea
                value={newAgent.desc}
                onChange={e => updateField('desc', e.target.value)}
                className="w-full px-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary h-16 resize-none"
                placeholder="Briefly describe the agent's role."
              />
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">
                {newAgent.type === 'ai' ? 'LLM Engine' : 'Bridge Engine'}
              </label>
              {filteredEngines.length > 0 ? (
                <select
                  value={newAgent.engine}
                  onChange={e => updateField('engine', e.target.value)}
                  className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
                >
                  <option value="">Select Engine...</option>
                  {filteredEngines.map(p => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              ) : (
                <div className="w-full px-2 py-1.5 rounded-lg border border-dashed border-content-muted text-[10px] text-content-tertiary font-mono text-center">
                  No {newAgent.type === 'ai' ? 'LLM' : 'bridge'} engines available
                </div>
              )}
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">Memory Engine</label>
              <select
                value={newAgent.memory}
                onChange={e => updateField('memory', e.target.value)}
                className="w-full px-2 py-1.5 rounded-lg border border-edge text-xs focus:outline-none focus:border-brand bg-surface-primary"
              >
                <option value="">Select Memory...</option>
                {memories.map(p => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="block text-xs font-bold text-content-secondary mb-1">
                Power Password <span className="text-content-muted font-normal">(optional)</span>
              </label>
              <div className="relative">
                <Lock size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-content-muted" />
                <input
                  type="password"
                  value={newAgent.password}
                  onChange={e => updateField('password', e.target.value)}
                  className="w-full pl-9 pr-3 py-2 rounded-lg border border-edge text-sm focus:outline-none focus:border-brand bg-surface-primary"
                  placeholder="Leave empty for no password"
                />
              </div>
              <p className="text-[9px] text-content-tertiary mt-1">Require password to toggle power on/off</p>
            </div>

            {createError && (
              <p className="text-xs text-red-400 text-center px-1">{createError}</p>
            )}
            <button
              onClick={handleCreate}
              disabled={!newAgent.name || !newAgent.desc || !newAgent.engine || isCreating}
              className="w-full mt-2 text-white py-2.5 rounded-xl text-sm font-bold shadow-sm hover:shadow-md transition-all disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
              style={{ backgroundColor: agentTypeColor(newAgent.type) }}
            >
              {isCreating ? <Activity size={16} className="animate-spin" /> : <Plus size={16} />}
              {newAgent.type === 'ai' ? 'CREATE AI AGENT' : 'CREATE CONTAINER'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
